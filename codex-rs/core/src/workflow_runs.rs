use crate::workflows::WorkflowRegistry;
use crate::workflows::WorkflowSummary;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::Weak;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tokio::sync::oneshot;

const WORKFLOW_RUNS_DIR: &str = "workflow-runs";
const RUN_SNAPSHOT_FILE: &str = "run.json";
const BOOTSTRAP_FILE: &str = "runner.mjs";
static WORKFLOW_RUN_STATES: once_cell::sync::Lazy<
    StdMutex<HashMap<PathBuf, Weak<WorkflowRunState>>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(HashMap::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub run_id: String,
    pub workflow: WorkflowSummary,
    pub status: WorkflowRunStatus,
    pub runner_status: String,
    pub inputs: Value,
    pub created_at: i64,
    pub updated_at: i64,
    pub revision: u64,
    pub message: String,
    pub abort_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
}

#[derive(Debug)]
struct LiveRunHandle {
    abort_tx: oneshot::Sender<Option<String>>,
}

struct WorkflowRunState {
    next_id: AtomicU64,
    runs: Arc<Mutex<BTreeMap<String, WorkflowRun>>>,
    live_runs: Arc<Mutex<HashMap<String, LiveRunHandle>>>,
    updates: broadcast::Sender<WorkflowRun>,
    store_root: Option<PathBuf>,
}

pub struct WorkflowRunManager {
    state: Arc<WorkflowRunState>,
}

impl Default for WorkflowRunManager {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl WorkflowRunManager {
    pub fn new(codex_home: impl Into<PathBuf>) -> Self {
        let store_root = codex_home.into().join(WORKFLOW_RUNS_DIR);
        let mut states = WORKFLOW_RUN_STATES
            .lock()
            .expect("workflow run state registry should not be poisoned");
        if let Some(state) = states.get(&store_root).and_then(Weak::upgrade) {
            return Self { state };
        }
        let state = Arc::new(WorkflowRunState {
            next_id: AtomicU64::new(0),
            runs: Arc::new(Mutex::new(BTreeMap::new())),
            live_runs: Arc::new(Mutex::new(HashMap::new())),
            updates: broadcast::channel(/*capacity*/ 128).0,
            store_root: Some(store_root.clone()),
        });
        states.insert(store_root, Arc::downgrade(&state));
        Self { state }
    }

    pub fn in_memory() -> Self {
        Self {
            state: Arc::new(WorkflowRunState {
                next_id: AtomicU64::new(0),
                runs: Arc::new(Mutex::new(BTreeMap::new())),
                live_runs: Arc::new(Mutex::new(HashMap::new())),
                updates: broadcast::channel(/*capacity*/ 128).0,
                store_root: None,
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WorkflowRun> {
        self.state.updates.subscribe()
    }

    pub async fn start(
        &self,
        registry: &WorkflowRegistry,
        workflow_id: &str,
        inputs: Value,
    ) -> Result<WorkflowRun, String> {
        let workflow = registry
            .find(workflow_id)
            .ok_or_else(|| format!("unknown workflow `{workflow_id}`"))?
            .clone();
        let sequence = self.state.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let now = unix_timestamp_seconds();
        let run_id = format!("wf_{now}_{sequence}");
        let snapshot_path = self
            .prepare_snapshot(&run_id, &workflow)
            .await?
            .map(|path| path.display().to_string());
        let run = WorkflowRun {
            run_id,
            workflow,
            status: WorkflowRunStatus::Running,
            runner_status: "runner_starting".to_string(),
            inputs,
            created_at: now,
            updated_at: now,
            revision: 1,
            message: "workflow runner is starting".to_string(),
            abort_reason: None,
            output: None,
            error: None,
            snapshot_path,
        };
        self.save_and_cache(run.clone()).await?;
        let _ = self.state.updates.send(run.clone());
        self.spawn_runner(run.clone(), WorkflowRunnerMode::Start)
            .await?;
        Ok(run)
    }

    pub async fn status(&self, run_id: &str) -> Result<WorkflowRun, String> {
        if let Some(run) = self.state.runs.lock().await.get(run_id).cloned() {
            return Ok(run);
        }
        self.load_run(run_id).await
    }

    pub async fn resume(&self, run_id: &str, inputs: Option<Value>) -> Result<WorkflowRun, String> {
        let mut run = self.status(run_id).await?;
        if self.state.live_runs.lock().await.contains_key(run_id) {
            return Err(format!("workflow run `{run_id}` is already running"));
        }
        match run.status {
            WorkflowRunStatus::Aborted => {
                return Err(format!("workflow run `{run_id}` is aborted"));
            }
            WorkflowRunStatus::Completed => {
                return Err(format!("workflow run `{run_id}` is already completed"));
            }
            WorkflowRunStatus::Running | WorkflowRunStatus::Failed => {}
        }
        if let Some(inputs) = inputs {
            run.inputs = inputs;
        }
        run.status = WorkflowRunStatus::Running;
        run.revision += 1;
        run.updated_at = unix_timestamp_seconds();
        run.runner_status = "runner_resuming".to_string();
        run.message = "workflow runner is resuming from snapshot".to_string();
        run.abort_reason = None;
        run.error = None;
        self.save_and_cache(run.clone()).await?;
        let _ = self.state.updates.send(run.clone());
        self.spawn_runner(run.clone(), WorkflowRunnerMode::Resume)
            .await?;
        Ok(run)
    }

    pub async fn abort(&self, run_id: &str, reason: Option<String>) -> Result<WorkflowRun, String> {
        if let Some(handle) = self.state.live_runs.lock().await.remove(run_id) {
            let _ = handle.abort_tx.send(reason.clone());
        }

        let mut run = self.status(run_id).await?;
        run.status = WorkflowRunStatus::Aborted;
        run.revision += 1;
        run.updated_at = unix_timestamp_seconds();
        run.runner_status = "aborted".to_string();
        run.message = "workflow run aborted".to_string();
        run.abort_reason = reason;
        self.save_and_cache(run.clone()).await?;
        let _ = self.state.updates.send(run.clone());
        Ok(run)
    }

    async fn spawn_runner(&self, run: WorkflowRun, mode: WorkflowRunnerMode) -> Result<(), String> {
        let Some(snapshot_dir) = run_snapshot_dir(&run) else {
            let failed = failed_run(run, "workflow run has no snapshot path".to_string());
            self.save_and_cache(failed).await?;
            return Ok(());
        };

        let bootstrap = snapshot_dir.join(BOOTSTRAP_FILE);
        write_bootstrap(&bootstrap).await?;
        let (abort_tx, abort_rx) = oneshot::channel();
        self.state
            .live_runs
            .lock()
            .await
            .insert(run.run_id.clone(), LiveRunHandle { abort_tx });
        let runs = Arc::clone(&self.state.runs);
        let live_runs = Arc::clone(&self.state.live_runs);
        let updates = self.state.updates.clone();
        let store_root = self.state.store_root.clone();
        let run_id = run.run_id.clone();
        tokio::spawn(async move {
            let updated = run_workflow_process(run, snapshot_dir, bootstrap, mode, abort_rx).await;
            let Some(updated) = terminal_update_preserving_abort(&runs, updated).await else {
                live_runs.lock().await.remove(&run_id);
                return;
            };
            if let Some(root) = &store_root {
                let _ = persist_run(root, &updated).await;
            }
            let updated_run_id = updated.run_id.clone();
            runs.lock()
                .await
                .insert(updated_run_id.clone(), updated.clone());
            live_runs.lock().await.remove(&updated_run_id);
            let _ = updates.send(updated);
        });
        Ok(())
    }

    async fn prepare_snapshot(
        &self,
        run_id: &str,
        workflow: &WorkflowSummary,
    ) -> Result<Option<PathBuf>, String> {
        let Some(store_root) = &self.state.store_root else {
            return Ok(None);
        };
        let snapshot_dir = store_root.join(run_id);
        copy_workflow_snapshot_dir(Path::new(&workflow.path), &snapshot_dir)?;
        tokio::fs::create_dir_all(snapshot_dir.join("node_modules/@codex/workflow"))
            .await
            .map_err(|err| format!("failed to create workflow shim directory: {err}"))?;
        tokio::fs::write(
            snapshot_dir.join("node_modules/@codex/workflow/package.json"),
            r#"{"type":"module","main":"index.js"}"#,
        )
        .await
        .map_err(|err| format!("failed to write workflow shim package: {err}"))?;
        tokio::fs::write(
            snapshot_dir.join("node_modules/@codex/workflow/index.js"),
            "export function defineWorkflow(definition) { return definition; }\n",
        )
        .await
        .map_err(|err| format!("failed to write workflow shim: {err}"))?;
        Ok(Some(snapshot_dir))
    }

    async fn save_and_cache(&self, run: WorkflowRun) -> Result<(), String> {
        if let Some(store_root) = &self.state.store_root {
            persist_run(store_root, &run).await?;
        }
        self.state.runs.lock().await.insert(run.run_id.clone(), run);
        Ok(())
    }

    async fn load_run(&self, run_id: &str) -> Result<WorkflowRun, String> {
        let Some(store_root) = &self.state.store_root else {
            return Err(format!("unknown workflow run `{run_id}`"));
        };
        let path = store_root.join(run_id).join(RUN_SNAPSHOT_FILE);
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|_| format!("unknown workflow run `{run_id}`"))?;
        let run: WorkflowRun = serde_json::from_str(&text)
            .map_err(|err| format!("failed to read workflow run `{run_id}`: {err}"))?;
        self.state
            .runs
            .lock()
            .await
            .insert(run.run_id.clone(), run.clone());
        Ok(run)
    }
}

async fn terminal_update_preserving_abort(
    runs: &Mutex<BTreeMap<String, WorkflowRun>>,
    updated: WorkflowRun,
) -> Option<WorkflowRun> {
    let existing = runs.lock().await.get(&updated.run_id).cloned();
    if let Some(existing) = existing
        && existing.status == WorkflowRunStatus::Aborted
        && updated.status != WorkflowRunStatus::Aborted
    {
        return None;
    }
    Some(updated)
}

#[derive(Clone, Copy)]
enum WorkflowRunnerMode {
    Start,
    Resume,
}

impl WorkflowRunnerMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Resume => "resume",
        }
    }
}

async fn run_workflow_process(
    mut run: WorkflowRun,
    snapshot_dir: PathBuf,
    bootstrap: PathBuf,
    mode: WorkflowRunnerMode,
    abort_rx: oneshot::Receiver<Option<String>>,
) -> WorkflowRun {
    run.runner_status = "runner_active".to_string();
    run.message = "workflow runner is executing TypeScript entry".to_string();
    run.updated_at = unix_timestamp_seconds();

    let input = serde_json::json!({
        "runId": &run.run_id,
        "workflowId": &run.workflow.id,
        "entry": &run.workflow.entry,
        "inputs": &run.inputs,
        "mode": mode.as_str(),
        "revision": run.revision,
    });
    let mut command = tokio::process::Command::new("node");
    command
        .arg(&bootstrap)
        .arg(serde_json::to_string(&input).unwrap_or_default())
        .current_dir(&snapshot_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => return failed_run(run, format!("failed to start Node workflow runner: {err}")),
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_task = tokio::spawn(read_pipe(stdout));
    let stderr_task = tokio::spawn(read_pipe(stderr));
    tokio::pin!(abort_rx);
    let process_result = tokio::select! {
        result = child.wait() => RunnerExit::Process(result),
        reason = &mut abort_rx => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            RunnerExit::Aborted(reason.ok().flatten())
        }
    };
    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    match process_result {
        RunnerExit::Aborted(reason) => {
            run.status = WorkflowRunStatus::Aborted;
            run.runner_status = "aborted".to_string();
            run.message = "workflow run aborted".to_string();
            run.abort_reason = reason;
        }
        RunnerExit::Process(Ok(status)) if status.success() => {
            run.status = WorkflowRunStatus::Completed;
            run.runner_status = "completed".to_string();
            run.message = "workflow runner completed".to_string();
            run.output = parse_runner_output(&stdout);
            run.error = None;
        }
        RunnerExit::Process(Ok(status)) => {
            run.status = WorkflowRunStatus::Failed;
            run.runner_status = "failed".to_string();
            run.message = format!("workflow runner exited with status {status}");
            run.error = Some(stderr_or_stdout(stderr, stdout));
        }
        RunnerExit::Process(Err(err)) => {
            run = failed_run(run, format!("failed to wait for workflow runner: {err}"));
        }
    }
    run.revision += 1;
    run.updated_at = unix_timestamp_seconds();
    run
}

enum RunnerExit {
    Process(std::io::Result<std::process::ExitStatus>),
    Aborted(Option<String>),
}

async fn read_pipe(pipe: Option<impl tokio::io::AsyncRead + Unpin>) -> String {
    let Some(pipe) = pipe else {
        return String::new();
    };
    let mut reader = BufReader::new(pipe).lines();
    let mut output = String::new();
    while let Ok(Some(line)) = reader.next_line().await {
        output.push_str(&line);
        output.push('\n');
    }
    output
}

fn parse_runner_output(stdout: &str) -> Option<Value> {
    stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<RunnerOutput>(line).ok())
        .map(|output| output.output)
}

#[derive(Deserialize)]
struct RunnerOutput {
    output: Value,
}

fn stderr_or_stdout(stderr: String, stdout: String) -> String {
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    stdout.trim().to_string()
}

fn failed_run(mut run: WorkflowRun, error: String) -> WorkflowRun {
    run.status = WorkflowRunStatus::Failed;
    run.runner_status = "failed".to_string();
    run.message = "workflow runner failed".to_string();
    run.error = Some(error);
    run.updated_at = unix_timestamp_seconds();
    run
}

fn run_snapshot_dir(run: &WorkflowRun) -> Option<PathBuf> {
    run.snapshot_path.as_ref().map(PathBuf::from)
}

async fn persist_run(store_root: &Path, run: &WorkflowRun) -> Result<(), String> {
    let dir = store_root.join(&run.run_id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|err| format!("failed to create workflow run store: {err}"))?;
    let json = serde_json::to_string_pretty(run)
        .map_err(|err| format!("failed to serialize workflow run: {err}"))?;
    tokio::fs::write(dir.join(RUN_SNAPSHOT_FILE), json)
        .await
        .map_err(|err| format!("failed to persist workflow run: {err}"))
}

fn copy_workflow_snapshot_dir(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|err| format!("failed to create workflow snapshot: {err}"))?;
    for entry in fs::read_dir(source)
        .map_err(|err| format!("failed to read workflow directory for snapshot: {err}"))?
    {
        let entry =
            entry.map_err(|err| format!("failed to read workflow snapshot entry: {err}"))?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();
        if file_name_str == "node_modules" || file_name_str == BOOTSTRAP_FILE {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(&file_name);
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to inspect workflow snapshot entry: {err}"))?;
        if file_type.is_dir() {
            copy_workflow_snapshot_dir(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path)
                .map_err(|err| format!("failed to copy workflow snapshot file: {err}"))?;
        }
    }
    Ok(())
}

async fn write_bootstrap(path: &Path) -> Result<(), String> {
    tokio::fs::write(path, BOOTSTRAP_SOURCE)
        .await
        .map_err(|err| format!("failed to write workflow runner bootstrap: {err}"))
}

fn unix_timestamp_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

const BOOTSTRAP_SOURCE: &str = r#"
const runInput = JSON.parse(process.argv[2] ?? "{}");
const module = await import(new URL(runInput.entry, import.meta.url).href);
const definition = module.default;

if (!definition || typeof definition.run !== "function") {
  throw new Error("workflow entry must default-export defineWorkflow({ run })");
}

const runtimeEvents = [];
const wf = {
  inputs: runInput.inputs ?? null,
  runId: runInput.runId,
  workflowId: runInput.workflowId,
  mode: runInput.mode,
  revision: runInput.revision,
  emit(event) {
    runtimeEvents.push(event);
  },
  Agent(id, options) {
    const agent = {
      id,
      options,
      async wait() {
        return { summary: `workflow agent '${id}' is bound but external agent execution is not available in this runner` };
      },
      async followup(message) {
        runtimeEvents.push({ type: "agentFollowup", id, message });
      }
    };
    runtimeEvents.push({ type: "agentBound", id, options });
    return agent;
  },
  async shell(command) {
    runtimeEvents.push({ type: "shellRequested", command });
    return { status: "skipped", reason: "shell execution is not available in this runner" };
  }
};

const result = await definition.run(wf);
console.log(JSON.stringify({
  output: {
    result: result ?? null,
    events: runtimeEvents,
    staticGraph: definition.staticGraph ?? null
  }
}));
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::WorkflowInputSpec;
    use crate::workflows::WorkflowSource;
    use pretty_assertions::assert_eq;
    use std::fs;

    fn registry_for(path: &Path) -> WorkflowRegistry {
        WorkflowRegistry {
            workflows: vec![WorkflowSummary {
                id: "feature-dev".to_string(),
                name: "feature-dev".to_string(),
                description: "project description".to_string(),
                source: WorkflowSource::Project,
                path: path.display().to_string(),
                entry: "workflow.ts".to_string(),
                version: Some("0.1.0".to_string()),
                when_to_use: Vec::new(),
                inputs: std::collections::BTreeMap::from([(
                    "objective".to_string(),
                    WorkflowInputSpec {
                        input_type: "string".to_string(),
                        description: Some("goal".to_string()),
                    },
                )]),
            }],
            diagnostics: Vec::new(),
        }
    }

    fn write_entry(dir: &Path, body: &str) {
        fs::create_dir_all(dir).expect("create workflow dir");
        fs::write(dir.join("workflow.ts"), body).expect("write workflow entry");
    }

    #[tokio::test]
    async fn workflow_run_manager_executes_and_persists_runner_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflow_dir = temp.path().join("workflow");
        write_entry(
            &workflow_dir,
            r#"import { defineWorkflow } from "@codex/workflow";
export default defineWorkflow({
  async run(wf) {
    wf.emit({ type: "started", objective: wf.inputs.objective });
    return { ok: true, mode: wf.mode };
  }
});"#,
        );
        let registry = registry_for(&workflow_dir);
        let manager = WorkflowRunManager::new(temp.path().join("home"));

        let started = manager
            .start(
                &registry,
                "feature-dev",
                serde_json::json!({"objective": "ship"}),
            )
            .await
            .expect("start workflow run");
        assert_eq!(started.workflow.id, "feature-dev");
        assert_eq!(started.status, WorkflowRunStatus::Running);

        let completed = wait_for_terminal(&manager, &started.run_id).await;
        assert_eq!(completed.status, WorkflowRunStatus::Completed);
        assert_eq!(completed.runner_status, "completed");
        assert_eq!(
            completed
                .output
                .as_ref()
                .and_then(|output| output.pointer("/result/ok")),
            Some(&Value::Bool(true))
        );

        let reloaded = WorkflowRunManager::new(temp.path().join("home"))
            .status(&started.run_id)
            .await
            .expect("load persisted run");
        assert_eq!(reloaded, completed);
    }

    #[tokio::test]
    async fn workflow_run_manager_can_abort_live_runner() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflow_dir = temp.path().join("workflow");
        write_entry(
            &workflow_dir,
            r#"import { defineWorkflow } from "@codex/workflow";
export default defineWorkflow({
  async run() {
    await new Promise((resolve) => setTimeout(resolve, 30_000));
  }
});"#,
        );
        let registry = registry_for(&workflow_dir);
        let manager = WorkflowRunManager::new(temp.path().join("home"));

        let started = manager
            .start(&registry, "feature-dev", serde_json::json!({}))
            .await
            .expect("start workflow run");
        let aborted = manager
            .abort(&started.run_id, Some("not needed".to_string()))
            .await
            .expect("abort workflow run");

        assert_eq!(aborted.status, WorkflowRunStatus::Aborted);
        assert_eq!(aborted.abort_reason.as_deref(), Some("not needed"));

        wait_for_no_live_runner(&manager, &started.run_id).await;
        let final_status = manager.status(&started.run_id).await.expect("final status");
        assert_eq!(final_status.status, WorkflowRunStatus::Aborted);
        assert_eq!(final_status.abort_reason.as_deref(), Some("not needed"));
    }

    #[tokio::test]
    async fn workflow_run_manager_shares_live_runs_for_same_store() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflow_dir = temp.path().join("workflow");
        write_entry(
            &workflow_dir,
            r#"import { defineWorkflow } from "@codex/workflow";
export default defineWorkflow({
  async run() {
    await new Promise((resolve) => setTimeout(resolve, 30_000));
  }
});"#,
        );
        let registry = registry_for(&workflow_dir);
        let codex_home = temp.path().join("home");
        let starter = WorkflowRunManager::new(codex_home.clone());
        let aborter = WorkflowRunManager::new(codex_home);

        let started = starter
            .start(&registry, "feature-dev", serde_json::json!({}))
            .await
            .expect("start workflow run");
        let aborted = aborter
            .abort(&started.run_id, Some("cross control".to_string()))
            .await
            .expect("abort workflow run from shared manager");

        assert_eq!(aborted.status, WorkflowRunStatus::Aborted);
        wait_for_no_live_runner(&starter, &started.run_id).await;
        let final_status = starter.status(&started.run_id).await.expect("final status");
        assert_eq!(final_status.status, WorkflowRunStatus::Aborted);
        assert_eq!(final_status.abort_reason.as_deref(), Some("cross control"));
    }

    #[tokio::test]
    async fn workflow_run_manager_snapshots_relative_imports() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflow_dir = temp.path().join("workflow");
        write_entry(
            &workflow_dir,
            r#"import { defineWorkflow } from "@codex/workflow";
import { helperValue } from "./helper.ts";

export default defineWorkflow({
  async run() {
    return { helperValue };
  }
});"#,
        );
        fs::write(
            workflow_dir.join("helper.ts"),
            "export const helperValue = 'from-helper';\n",
        )
        .expect("write helper");
        let registry = registry_for(&workflow_dir);
        let manager = WorkflowRunManager::new(temp.path().join("home"));

        let started = manager
            .start(&registry, "feature-dev", serde_json::json!({}))
            .await
            .expect("start workflow run");

        fs::write(
            workflow_dir.join("helper.ts"),
            "export const helperValue = 'mutated-after-start';\n",
        )
        .expect("mutate helper after snapshot");
        let completed = wait_for_terminal(&manager, &started.run_id).await;

        assert_eq!(completed.status, WorkflowRunStatus::Completed);
        assert_eq!(
            completed
                .output
                .as_ref()
                .and_then(|output| output.pointer("/result/helperValue")),
            Some(&Value::String("from-helper".to_string()))
        );
    }

    async fn wait_for_terminal(manager: &WorkflowRunManager, run_id: &str) -> WorkflowRun {
        for _ in 0..50 {
            let run = manager.status(run_id).await.expect("run status");
            if run.status != WorkflowRunStatus::Running {
                return run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("workflow run did not finish");
    }

    async fn wait_for_no_live_runner(manager: &WorkflowRunManager, run_id: &str) {
        for _ in 0..50 {
            if !manager.state.live_runs.lock().await.contains_key(run_id) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("workflow run did not stop");
    }
}
