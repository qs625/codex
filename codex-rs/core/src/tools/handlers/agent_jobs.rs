use crate::StateDbHandle;
use crate::agent::control::SpawnAgentOptions;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::next_thread_spawn_depth;
use crate::agent::status::is_final;
use crate::config::Config;
use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::handlers::multi_agents_common::build_agent_spawn_config;
use crate::tools::handlers::parse_arguments;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::user_input::UserInput;
use codex_state_api::AgentJob;
use codex_state_api::AgentJobItem;
use codex_state_api::AgentJobItemStatus;
use codex_state_api::build_agent_job_worker_prompt;
use codex_state_api::default_agent_job_output_csv_path;
use codex_state_api::ensure_unique_agent_job_headers;
use codex_state_api::parse_agent_job_csv;
use codex_state_api::render_agent_job_csv;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch::Receiver;
use tokio::time::Duration;
use tokio::time::Instant;
use tokio::time::timeout;
use uuid::Uuid;

mod report_agent_job_result;
mod spawn_agents_on_csv;

pub use report_agent_job_result::ReportAgentJobResultHandler;
pub use spawn_agents_on_csv::SpawnAgentsOnCsvHandler;

const DEFAULT_AGENT_JOB_CONCURRENCY: usize = 16;
const MAX_AGENT_JOB_CONCURRENCY: usize = 64;
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_AGENT_JOB_ITEM_TIMEOUT: Duration = Duration::from_secs(60 * 30);

#[derive(Debug, Deserialize)]
struct SpawnAgentsOnCsvArgs {
    csv_path: String,
    instruction: String,
    id_column: Option<String>,
    output_csv_path: Option<String>,
    output_schema: Option<Value>,
    max_concurrency: Option<usize>,
    max_workers: Option<usize>,
    max_runtime_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ReportAgentJobResultArgs {
    job_id: String,
    item_id: String,
    result: Value,
    stop: Option<bool>,
}

#[derive(Debug, Serialize)]
struct SpawnAgentsOnCsvResult {
    job_id: String,
    status: String,
    output_csv_path: String,
    total_items: usize,
    completed_items: usize,
    failed_items: usize,
    job_error: Option<String>,
    failed_item_errors: Option<Vec<AgentJobFailureSummary>>,
}

#[derive(Debug, Serialize)]
struct AgentJobFailureSummary {
    item_id: String,
    source_id: Option<String>,
    last_error: String,
}

#[derive(Debug, Serialize)]
struct ReportAgentJobResultToolResult {
    accepted: bool,
}

#[derive(Debug, Clone)]
struct JobRunnerOptions {
    max_concurrency: usize,
    spawn_config: Config,
}

#[derive(Debug, Clone)]
struct ActiveJobItem {
    item_id: String,
    started_at: Instant,
    status_rx: Option<Receiver<AgentStatus>>,
}

fn required_state_db(session: &Arc<Session>) -> Result<StateDbHandle, FunctionCallError> {
    session.state_db().ok_or_else(|| {
        FunctionCallError::Fatal("sqlite state db is unavailable for this session".to_string())
    })
}

async fn build_runner_options(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    requested_concurrency: Option<usize>,
) -> Result<JobRunnerOptions, FunctionCallError> {
    let session_source = turn.session_source.clone();
    let child_depth = next_thread_spawn_depth(&session_source);
    let max_depth = turn.config.agent_max_depth;
    if exceeds_thread_spawn_depth_limit(child_depth, max_depth) {
        return Err(FunctionCallError::RespondToModel(
            "agent depth limit reached; this session cannot spawn more subagents".to_string(),
        ));
    }
    if turn.config.agent_max_threads == Some(0) {
        return Err(FunctionCallError::RespondToModel(
            "agent thread limit reached; this session cannot spawn more subagents".to_string(),
        ));
    }
    let max_concurrency =
        normalize_concurrency(requested_concurrency, turn.config.agent_max_threads);
    let base_instructions = session.get_base_instructions().await;
    let spawn_config =
        build_agent_spawn_config(&base_instructions, turn.as_ref(), /*cwd*/ None)?;
    Ok(JobRunnerOptions {
        max_concurrency,
        spawn_config,
    })
}

fn normalize_concurrency(requested: Option<usize>, max_threads: Option<usize>) -> usize {
    let requested = requested.unwrap_or(DEFAULT_AGENT_JOB_CONCURRENCY).max(1);
    let requested = requested.min(MAX_AGENT_JOB_CONCURRENCY);
    if let Some(max_threads) = max_threads {
        requested.min(max_threads.max(1))
    } else {
        requested
    }
}

fn normalize_max_runtime_seconds(requested: Option<u64>) -> Result<Option<u64>, FunctionCallError> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    if requested == 0 {
        return Err(FunctionCallError::RespondToModel(
            "max_runtime_seconds must be >= 1".to_string(),
        ));
    }
    Ok(Some(requested))
}

async fn run_agent_job_loop(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    db: StateDbHandle,
    job_id: String,
    options: JobRunnerOptions,
) -> anyhow::Result<()> {
    let job = db
        .get_agent_job(job_id.as_str())
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent job {job_id} was not found"))?;
    let runtime_timeout = job_runtime_timeout(&job);
    let mut active_items: HashMap<ThreadId, ActiveJobItem> = HashMap::new();
    recover_running_items(
        session.clone(),
        db.clone(),
        job_id.as_str(),
        &mut active_items,
        runtime_timeout,
    )
    .await?;

    let mut cancel_requested = db.is_agent_job_cancelled(job_id.as_str()).await?;
    loop {
        let mut progressed = false;

        if !cancel_requested && db.is_agent_job_cancelled(job_id.as_str()).await? {
            cancel_requested = true;
        }

        if !cancel_requested && active_items.len() < options.max_concurrency {
            let slots = options.max_concurrency - active_items.len();
            let pending_items = db
                .list_agent_job_items(
                    job_id.as_str(),
                    Some(AgentJobItemStatus::Pending),
                    Some(slots),
                )
                .await?;
            for item in pending_items {
                let prompt = build_agent_job_worker_prompt(&job, &item)?;
                let items = vec![UserInput::Text {
                    text: prompt,
                    text_elements: Vec::new(),
                }];
                let thread_id = match session
                    .services
                    .agent_control
                    .spawn_agent_with_metadata(
                        options.spawn_config.clone(),
                        items.into(),
                        Some(SessionSource::SubAgent(SubAgentSource::Other(format!(
                            "agent_job:{job_id}"
                        )))),
                        SpawnAgentOptions {
                            environments: Some(turn.environments.to_selections()),
                            ..Default::default()
                        },
                    )
                    .await
                {
                    Ok(spawned_agent) => spawned_agent.thread_id,
                    Err(CodexErr::AgentLimitReached { .. }) => {
                        db.mark_agent_job_item_pending(
                            job_id.as_str(),
                            item.item_id.as_str(),
                            /*error_message*/ None,
                        )
                        .await?;
                        break;
                    }
                    Err(err) => {
                        let error_message = format!("failed to spawn worker: {err}");
                        db.mark_agent_job_item_failed(
                            job_id.as_str(),
                            item.item_id.as_str(),
                            error_message.as_str(),
                        )
                        .await?;
                        progressed = true;
                        continue;
                    }
                };
                let assigned = db
                    .mark_agent_job_item_running_with_thread(
                        job_id.as_str(),
                        item.item_id.as_str(),
                        thread_id.to_string().as_str(),
                    )
                    .await?;
                if !assigned {
                    let _ = session
                        .services
                        .agent_control
                        .shutdown_live_agent(thread_id)
                        .await;
                    continue;
                }
                active_items.insert(
                    thread_id,
                    ActiveJobItem {
                        item_id: item.item_id.clone(),
                        started_at: Instant::now(),
                        status_rx: session
                            .services
                            .agent_control
                            .subscribe_status(thread_id)
                            .await
                            .ok(),
                    },
                );
                progressed = true;
            }
        }

        if reap_stale_active_items(
            session.clone(),
            db.clone(),
            job_id.as_str(),
            &mut active_items,
            runtime_timeout,
        )
        .await?
        {
            progressed = true;
        }

        let finished = find_finished_threads(session.clone(), &active_items).await;
        if finished.is_empty() {
            let progress = db.get_agent_job_progress(job_id.as_str()).await?;
            if cancel_requested {
                if progress.running_items == 0 && active_items.is_empty() {
                    break;
                }
            } else if progress.pending_items == 0
                && progress.running_items == 0
                && active_items.is_empty()
            {
                break;
            }
            if !progressed {
                wait_for_status_change(&active_items).await;
            }
            continue;
        }

        for (thread_id, item_id) in finished {
            finalize_finished_item(
                session.clone(),
                db.clone(),
                job_id.as_str(),
                item_id.as_str(),
                thread_id,
            )
            .await?;
            active_items.remove(&thread_id);
        }
    }

    if let Err(err) = export_job_csv_snapshot(db.clone(), &job).await {
        let message = format!("auto-export failed: {err}");
        db.mark_agent_job_failed(job_id.as_str(), message.as_str())
            .await?;
        return Ok(());
    }
    let cancelled = cancel_requested || db.is_agent_job_cancelled(job_id.as_str()).await?;
    if cancelled {
        return Ok(());
    }
    db.mark_agent_job_completed(job_id.as_str()).await?;
    Ok(())
}

async fn export_job_csv_snapshot(db: StateDbHandle, job: &AgentJob) -> anyhow::Result<()> {
    let items = db
        .list_agent_job_items(job.id.as_str(), /*status*/ None, /*limit*/ None)
        .await?;
    let csv_content = render_agent_job_csv(job.input_headers.as_slice(), items.as_slice())
        .map_err(|err| anyhow::anyhow!("failed to render job csv for auto-export: {err}"))?;
    let output_path = PathBuf::from(job.output_csv_path.clone());
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&output_path, csv_content).await?;
    Ok(())
}

async fn recover_running_items(
    session: Arc<Session>,
    db: StateDbHandle,
    job_id: &str,
    active_items: &mut HashMap<ThreadId, ActiveJobItem>,
    runtime_timeout: Duration,
) -> anyhow::Result<()> {
    let running_items = db
        .list_agent_job_items(
            job_id,
            Some(AgentJobItemStatus::Running),
            /*limit*/ None,
        )
        .await?;
    for item in running_items {
        if is_item_stale(&item, runtime_timeout) {
            let error_message = format!("worker exceeded max runtime of {runtime_timeout:?}");
            db.mark_agent_job_item_failed(job_id, item.item_id.as_str(), error_message.as_str())
                .await?;
            if let Some(assigned_thread_id) = item.assigned_thread_id.as_ref()
                && let Ok(thread_id) = ThreadId::from_string(assigned_thread_id.as_str())
            {
                let _ = session
                    .services
                    .agent_control
                    .shutdown_live_agent(thread_id)
                    .await;
            }
            continue;
        }
        let Some(assigned_thread_id) = item.assigned_thread_id.clone() else {
            db.mark_agent_job_item_failed(
                job_id,
                item.item_id.as_str(),
                "running item is missing assigned_thread_id",
            )
            .await?;
            continue;
        };
        let thread_id = match ThreadId::from_string(assigned_thread_id.as_str()) {
            Ok(thread_id) => thread_id,
            Err(err) => {
                let error_message = format!("invalid assigned_thread_id: {err:?}");
                db.mark_agent_job_item_failed(
                    job_id,
                    item.item_id.as_str(),
                    error_message.as_str(),
                )
                .await?;
                continue;
            }
        };
        if is_final(&session.services.agent_control.get_status(thread_id).await) {
            finalize_finished_item(
                session.clone(),
                db.clone(),
                job_id,
                item.item_id.as_str(),
                thread_id,
            )
            .await?;
        } else {
            active_items.insert(
                thread_id,
                ActiveJobItem {
                    item_id: item.item_id.clone(),
                    started_at: started_at_from_item(&item),
                    status_rx: session
                        .services
                        .agent_control
                        .subscribe_status(thread_id)
                        .await
                        .ok(),
                },
            );
        }
    }
    Ok(())
}

async fn find_finished_threads(
    session: Arc<Session>,
    active_items: &HashMap<ThreadId, ActiveJobItem>,
) -> Vec<(ThreadId, String)> {
    let mut finished = Vec::new();
    for (thread_id, item) in active_items {
        let status = active_item_status(session.as_ref(), *thread_id, item).await;
        if is_final(&status) {
            finished.push((*thread_id, item.item_id.clone()));
        }
    }
    finished
}

async fn active_item_status(
    session: &Session,
    thread_id: ThreadId,
    item: &ActiveJobItem,
) -> AgentStatus {
    if let Some(status_rx) = item.status_rx.as_ref()
        && status_rx.has_changed().is_ok()
    {
        return status_rx.borrow().clone();
    }
    session.services.agent_control.get_status(thread_id).await
}

async fn wait_for_status_change(active_items: &HashMap<ThreadId, ActiveJobItem>) {
    let mut waiters = FuturesUnordered::new();
    for item in active_items.values() {
        if let Some(status_rx) = item.status_rx.as_ref() {
            let mut status_rx = status_rx.clone();
            waiters.push(async move {
                let _ = status_rx.changed().await;
            });
        }
    }
    if waiters.is_empty() {
        tokio::time::sleep(STATUS_POLL_INTERVAL).await;
        return;
    }
    let _ = timeout(STATUS_POLL_INTERVAL, waiters.next()).await;
}

async fn reap_stale_active_items(
    session: Arc<Session>,
    db: StateDbHandle,
    job_id: &str,
    active_items: &mut HashMap<ThreadId, ActiveJobItem>,
    runtime_timeout: Duration,
) -> anyhow::Result<bool> {
    let mut stale = Vec::new();
    for (thread_id, item) in active_items.iter() {
        if item.started_at.elapsed() >= runtime_timeout {
            stale.push((*thread_id, item.item_id.clone()));
        }
    }
    if stale.is_empty() {
        return Ok(false);
    }
    for (thread_id, item_id) in stale {
        let error_message = format!("worker exceeded max runtime of {runtime_timeout:?}");
        db.mark_agent_job_item_failed(job_id, item_id.as_str(), error_message.as_str())
            .await?;
        let _ = session
            .services
            .agent_control
            .shutdown_live_agent(thread_id)
            .await;
        active_items.remove(&thread_id);
    }
    Ok(true)
}

async fn finalize_finished_item(
    session: Arc<Session>,
    db: StateDbHandle,
    job_id: &str,
    item_id: &str,
    thread_id: ThreadId,
) -> anyhow::Result<()> {
    let item = db
        .get_agent_job_item(job_id, item_id)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("job item not found for finalization: {job_id}/{item_id}")
        })?;
    if matches!(item.status, AgentJobItemStatus::Running) {
        if item.result_json.is_some() {
            let _ = db.mark_agent_job_item_completed(job_id, item_id).await?;
        } else {
            let _ = db
                .mark_agent_job_item_failed(
                    job_id,
                    item_id,
                    "worker finished without calling report_agent_job_result",
                )
                .await?;
        }
    }
    let _ = session
        .services
        .agent_control
        .shutdown_live_agent(thread_id)
        .await;
    Ok(())
}

fn job_runtime_timeout(job: &AgentJob) -> Duration {
    job.max_runtime_seconds
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_AGENT_JOB_ITEM_TIMEOUT)
}

fn started_at_from_item(item: &AgentJobItem) -> Instant {
    let now = chrono::Utc::now();
    let age = now.signed_duration_since(item.updated_at);
    if let Ok(age) = age.to_std() {
        Instant::now().checked_sub(age).unwrap_or_else(Instant::now)
    } else {
        Instant::now()
    }
}

fn is_item_stale(item: &AgentJobItem, runtime_timeout: Duration) -> bool {
    let now = chrono::Utc::now();
    if let Ok(age) = now.signed_duration_since(item.updated_at).to_std() {
        age >= runtime_timeout
    } else {
        false
    }
}
