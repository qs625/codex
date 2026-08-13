use std::sync::Arc;
use std::time::Instant;

use crate::event_mapping::injected_context_item_from_response_items;
use crate::client_common::PromptBuildParams;
use crate::client_common::build_prompt;
#[cfg(test)]
use crate::session::PreviousTurnSettings;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::util::backoff;
use codex_analytics_api::CodexCompactionEvent;
use codex_analytics_api::CompactionImplementation;
use codex_analytics_api::CompactionPhase;
use codex_analytics_api::CompactionReason;
use codex_analytics_api::CompactionStatus;
use codex_analytics_api::CompactionStrategy;
use codex_analytics_api::CompactionTrigger;
use codex_analytics_api::now_unix_seconds;
use codex_features::Feature;
use codex_config_types::CompactReplacementFileRole as ConfigCompactReplacementFileRole;
use codex_turn_items::last_assistant_message_from_turn;
#[cfg(test)]
use codex_turn_items::process_remote_compacted_history;
use codex_turn_items::raw_assistant_output_text_from_item;
use compact_service::FsCompactService;
use compact_service_api::CompactMemoryRole;
use compact_service_api::CompactReplacementFile;
use compact_service_api::ReplacementHistoryInput;
use compact_service_api::SoftCompactInputs;
use compact_service_api::SoftCompactThresholds;
use hooks::PostCompactHookOutcome;
use hooks::PreCompactHookOutcome;
use hooks::run_post_compact_hooks;
use hooks::run_pre_compact_hooks;
use protocol::error::CodexErr;
use protocol::error::Result as CodexResult;
use protocol::items::ContextCompactionItem;
use protocol::items::ContextCompactionReplacementItem;
use protocol::items::TurnItem;
use protocol::items::context_compaction_replacement_items_from_response_items;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::CompactedItem;
use protocol::protocol::CodexErrorInfo;
use protocol::protocol::ErrorEvent;
use protocol::protocol::EventMsg;
use protocol::protocol::TurnStartedEvent;
use protocol::protocol::WarningEvent;
use protocol::user_input::UserInput;
use futures::StreamExt;
use tracing::warn;

pub const SUMMARIZATION_PROMPT: &str = include_str!("../templates/compact/prompt.md");
pub const SUMMARY_PREFIX: &str = include_str!("../templates/compact/summary_prefix.md");
pub(crate) const DEFAULT_COMPACTED_MESSAGE: &str = "Memory-backed checkpoint recorded.";
pub(crate) const COMPACT_CONTEXT_WINDOW_RECOVERY_FAILED_MESSAGE: &str =
    "The thread is still too large for this model's context window after automatic compaction. Reduce recent tool output or switch to a model with a larger context window, then try again.";

/// Controls whether compaction replacement history must include initial context.
///
/// Pre-turn compaction may use `DoNotInject`: it replaces history with a summary and clears
/// `reference_context_item`, so the next regular turn will fully reinject initial context after
/// compaction.
///
/// Manual and mid-turn compaction use `BeforeLastUserMessage` so the rebuilt initial context lands
/// ahead of the retained compact checkpoint block and remains visible in replacement history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InitialContextInjection {
    BeforeLastUserMessage,
    DoNotInject,
}

pub(crate) async fn run_inline_auto_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    initial_context_injection: InitialContextInjection,
    reason: CompactionReason,
    phase: CompactionPhase,
) -> CodexResult<()> {
    run_inline_auto_compact_task_with_retained_suffix(
        sess,
        turn_context,
        initial_context_injection,
        reason,
        phase,
        Vec::new(),
    )
    .await
}

pub(crate) async fn run_inline_auto_compact_task_with_retained_suffix(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    initial_context_injection: InitialContextInjection,
    reason: CompactionReason,
    phase: CompactionPhase,
    retained_suffix: Vec<ResponseItem>,
) -> CodexResult<()> {
    run_compact_task_inner(
        sess,
        turn_context,
        Vec::new(),
        initial_context_injection,
        CompactionTrigger::Auto,
        reason,
        phase,
        retained_suffix,
    )
    .await?;
    Ok(())
}

pub(crate) async fn run_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    input: Vec<UserInput>,
) -> CodexResult<()> {
    let start_event = EventMsg::TurnStarted(TurnStartedEvent {
        turn_id: turn_context.sub_id.clone(),
        started_at: turn_context.turn_timing_state.started_at_unix_secs().await,
        model_context_window: turn_context.model_context_window(),
        collaboration_mode_kind: turn_context.collaboration_mode.mode,
    });
    sess.send_event(&turn_context, start_event).await;
    run_compact_task_inner(
        sess.clone(),
        turn_context,
        input,
        InitialContextInjection::BeforeLastUserMessage,
        CompactionTrigger::Manual,
        CompactionReason::UserRequested,
        CompactionPhase::StandaloneTurn,
        Vec::new(),
    )
    .await?;
    Ok(())
}

async fn run_compact_task_inner(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    input: Vec<UserInput>,
    initial_context_injection: InitialContextInjection,
    trigger: CompactionTrigger,
    reason: CompactionReason,
    phase: CompactionPhase,
    retained_suffix: Vec<ResponseItem>,
) -> CodexResult<()> {
    let attempt = CompactionAnalyticsAttempt::begin(
        sess.as_ref(),
        turn_context.as_ref(),
        trigger,
        reason,
        CompactionImplementation::Responses,
        phase,
    )
    .await;
    let pre_compact_outcome =
        run_pre_compact_hooks(sess.as_ref(), turn_context.as_ref(), trigger).await;
    match pre_compact_outcome {
        PreCompactHookOutcome::Continue => {}
        PreCompactHookOutcome::Stopped { reason } => {
            let error = reason.unwrap_or_else(|| "PreCompact hook stopped execution".to_string());
            attempt
                .track(sess.as_ref(), CompactionStatus::Interrupted, Some(error))
                .await;
            return Err(CodexErr::TurnAborted);
        }
    }
    let result = run_compact_task_inner_impl(
        Arc::clone(&sess),
        Arc::clone(&turn_context),
        input,
        initial_context_injection,
        retained_suffix,
    )
    .await;
    let status = compaction_status_from_result(&result);
    let error = result.as_ref().err().map(ToString::to_string);
    if result.is_ok() {
        let post_compact_outcome =
            run_post_compact_hooks(sess.as_ref(), turn_context.as_ref(), trigger).await;
        if let PostCompactHookOutcome::Stopped = post_compact_outcome {
            attempt.track(sess.as_ref(), status, error).await;
            return Err(CodexErr::TurnAborted);
        }
    }
    attempt.track(sess.as_ref(), status, error).await;
    result.map(|_| ())
}

async fn run_compact_task_inner_impl(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    _input: Vec<UserInput>,
    initial_context_injection: InitialContextInjection,
    retained_suffix: Vec<ResponseItem>,
) -> CodexResult<String> {
    let compact_service = FsCompactService::new();
    let replacement_files = compact_replacement_files(turn_context.as_ref());
    let staged_recovery = !retained_suffix.is_empty();

    let compaction_item = ContextCompactionItem::new();
    let initial_input_for_turn = compact_prompt_control_item(turn_context.compact_prompt());
    if !staged_recovery {
        let started_compaction_item = TurnItem::ContextCompaction(compaction_item.clone());
        sess.emit_turn_item_started(&turn_context, &started_compaction_item)
            .await;
        sess.record_conversation_items(&turn_context, std::slice::from_ref(&initial_input_for_turn))
            .await;
    }

    let max_retries = turn_context.provider.info().stream_max_retries();
    let mut retries = 0;
    let model_client_api =
        crate::session::turn::model_client_api_for_turn(sess.as_ref(), turn_context.as_ref())
            .await
            .map_err(|err| {
                CodexErr::Fatal(format!("failed to resolve compact model client api: {err}"))
            })?;
    let mut client_session = model_client_api
        .create_turn_client()
        .await
        .map_err(|err| CodexErr::Fatal(format!("failed to create compact model client: {err}")))?;
    // Reuse one client session so turn-scoped state (sticky routing, websocket incremental
    // request tracking)
    // survives retries within this compact turn.
    let explicitly_enabled_connectors = std::collections::HashSet::new();
    let turn_diff_tracker = Arc::new(tokio::sync::Mutex::new(
        thread_service_api::TurnDiffTracker::default(),
    ));
    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&sess) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let compact_tool_inputs = Arc::new(crate::session::turn::TurnToolInputs {
        session_capability: Arc::downgrade(&session_capability),
        mcp_tools: Vec::new(),
        deferred_mcp_tools: Vec::new(),
        discoverable_tools: Vec::new(),
        default_agent_type_description: String::new(),
        expose_model_visible_tools: false,
    });
    let skills_outcome = Some(turn_context.turn_skills.outcome.as_ref());
    let mut isolated_compacted_message = None;
    loop {
        let mut turn_input = sess
            .clone_history()
            .await
            .for_prompt(&turn_context.model_info.input_modalities);
        if staged_recovery {
            turn_input.push(initial_input_for_turn.clone());
        }
        let turn_metadata_header = turn_context.turn_metadata_state.current_header_value();
        let attempt_result = if staged_recovery {
            match run_isolated_compact_sampling(
                sess.as_ref(),
                turn_context.as_ref(),
                &mut *client_session,
                turn_metadata_header.as_deref(),
                turn_input,
            )
            .await
            {
                Ok(compacted_message) => {
                    isolated_compacted_message = Some(compacted_message);
                    Ok(crate::session::turn::SamplingRequestResult {
                        needs_follow_up: false,
                        last_agent_message: None,
                    })
                }
                Err(err) => Err(err),
            }
        } else {
            crate::session::turn::run_sampling_request(crate::session::turn::SamplingRequest {
                tool_inputs_override: Some(Arc::clone(&compact_tool_inputs)),
                sess: Arc::clone(&sess),
                turn_context: Arc::clone(&turn_context),
                turn_store: Arc::clone(&turn_context.extension_data),
                turn_diff_tracker: Arc::clone(&turn_diff_tracker),
                client_session: &mut *client_session,
                turn_metadata_header: turn_metadata_header.as_deref(),
                input: turn_input,
                explicitly_enabled_connectors: &explicitly_enabled_connectors,
                skills_outcome,
                cancellation_token: tokio_util::sync::CancellationToken::new(),
            })
            .await
        };

        match attempt_result {
            Ok(result) => {
                if result.needs_follow_up {
                    continue;
                }
                break;
            }
            Err(CodexErr::Interrupted) => {
                return Err(CodexErr::Interrupted);
            }
            Err(e @ CodexErr::ContextWindowExceeded) => {
                sess.set_total_tokens_full(turn_context.as_ref()).await;
                if !staged_recovery {
                    send_compact_context_window_error(sess.as_ref(), turn_context.as_ref()).await;
                }
                return Err(e);
            }
            Err(e) => {
                if retries >= max_retries
                    && client_session.try_switch_fallback_transport(
                        turn_context.session_telemetry.clone(),
                        turn_context.model_info.clone(),
                    )
                {
                    retries = 0;
                    continue;
                }
                if retries < max_retries {
                    retries += 1;
                    let delay = backoff(retries);
                    sess.notify_stream_error(
                        turn_context.as_ref(),
                        format!("Reconnecting... {retries}/{max_retries}"),
                        e,
                    )
                    .await;
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    let event = EventMsg::Error(e.to_error_event(/*message_prefix*/ None));
                    sess.send_event(&turn_context, event).await;
                    return Err(e);
                }
            }
        }
    }

    let history_snapshot = sess.clone_history().await;
    let history_items = history_snapshot.raw_items();
    let memory_bundle = compact_service
        .read_memory_bundle(&replacement_files)
        .await
        .map_err(|err| {
            CodexErr::Fatal(format!("failed to read compact replacement files: {err}"))
        })?;
    let compact_window_summary =
        compact_service.summarize_compact_window(history_items, SUMMARY_PREFIX);
    let compacted_message = isolated_compacted_message.unwrap_or_else(|| {
        compact_turn_final_output(history_items, turn_context.compact_prompt())
            .unwrap_or_else(|| DEFAULT_COMPACTED_MESSAGE.to_string())
    });
    let mut new_history = compact_service.build_replacement_history(ReplacementHistoryInput {
        initial_context: Vec::new(),
        memory_bundle: memory_bundle.clone(),
        recent_real_user_messages: compact_window_summary.recent_real_user_messages,
        final_output: Some(compacted_message.clone()),
    });

    let mut injected_initial_context_item = None;
    let mut injected_initial_context_len = 0;
    if matches!(
        initial_context_injection,
        InitialContextInjection::BeforeLastUserMessage
    ) {
        let initial_context = sess.build_initial_context(turn_context.as_ref()).await;
        injected_initial_context_len = initial_context.len();
        injected_initial_context_item = injected_context_item_from_response_items(&initial_context);
        new_history =
            prepend_initial_context_to_memory_checkpoint_history(new_history, initial_context);
    }
    let reference_context_item = match initial_context_injection {
        InitialContextInjection::DoNotInject => None,
        InitialContextInjection::BeforeLastUserMessage => Some(
            sess.reference_context_item_for_turn(turn_context.as_ref())
                .await,
        ),
    };
    let visible_replacement_history_len = new_history.len();
    let mut persisted_replacement_history = new_history.clone();
    persisted_replacement_history.extend(retained_suffix);
    let replacement_history = Some(persisted_replacement_history.clone());
    let compacted_item = CompactedItem {
        message: compacted_message.clone(),
        replacement_history: replacement_history.clone(),
        visible_replacement_history_len: (visible_replacement_history_len
            != persisted_replacement_history.len())
        .then_some(visible_replacement_history_len),
    };
    let replacement_history_tail = new_history
        .iter()
        .skip(injected_initial_context_len)
        .cloned()
        .collect::<Vec<_>>();
    let mut replacement_history_items = Vec::new();
    if let Some(TurnItem::InjectedContext(item)) = injected_initial_context_item {
        replacement_history_items.push(ContextCompactionReplacementItem::InjectedContext(item));
    }
    replacement_history_items.extend(context_compaction_replacement_items_from_response_items(
        replacement_history_tail,
    ));
    let compaction_item = ContextCompactionItem {
        replacement_history: replacement_history_items,
        ..compaction_item
    };
    sess.replace_compacted_history(
        persisted_replacement_history,
        reference_context_item,
        compacted_item,
    )
        .await;
    client_session.reset_websocket_session();
    sess.recompute_token_usage(&turn_context).await;

    sess.emit_turn_item_completed(&turn_context, TurnItem::ContextCompaction(compaction_item))
        .await;
    let warning = EventMsg::Warning(WarningEvent {
        message: "Heads up: Long threads and multiple compactions can cause the model to be less accurate. Start a new thread when possible to keep threads small and targeted.".to_string(),
    });
    sess.send_event(&turn_context, warning).await;
    Ok(compacted_message)
}

pub(crate) async fn run_isolated_compact_sampling(
    sess: &Session,
    turn_context: &TurnContext,
    client_session: &mut dyn model_service_api::ModelTurnClientApi,
    turn_metadata_header: Option<&str>,
    input: Vec<ResponseItem>,
) -> CodexResult<String> {
    let base_instructions = sess.get_base_instructions().await;
    let prompt = build_prompt(PromptBuildParams {
        input,
        tools: Vec::new(),
        parallel_tool_calls: false,
        base_instructions,
        personality: turn_context.personality,
        output_schema: turn_context.final_output_json_schema.clone(),
        output_schema_strict: false,
    });
    let inference_trace = sess.services.rollout_thread_trace.inference_trace_context(
        turn_context.sub_id.as_str(),
        turn_context.model_info.slug.as_str(),
        turn_context.provider.info().name.as_str(),
    );
    let mut stream = client_session
        .stream_responses(model_service_api::TurnModelRequest {
            request: model_service_api::ResponsesModelRequest {
                input: prompt.input,
                tools: prompt.tools,
                parallel_tool_calls: prompt.parallel_tool_calls,
                base_instructions: prompt.base_instructions,
                personality: prompt.personality,
                output_schema: prompt.output_schema,
                output_schema_strict: prompt.output_schema_strict,
                model: Some(turn_context.model_info.slug.clone()),
                reasoning_effort: turn_context.reasoning_effort,
                reasoning_summary: turn_context.reasoning_summary,
                service_tier: crate::session::turn::model_service_tier(
                    turn_context.config.service_tier.as_deref(),
                ),
                verbosity: None,
                turn_metadata_header: turn_metadata_header.map(ToOwned::to_owned),
            },
            model_info: turn_context.model_info.clone(),
            session_telemetry: turn_context.session_telemetry.clone(),
            turn_metadata_header: turn_metadata_header.map(ToOwned::to_owned),
            inference_trace,
        })
        .await
        .map_err(|err| err.into_codex_err())?;

    let mut assistant_output = String::new();
    while let Some(event) = stream.next().await {
        match event.map_err(|err| err.into_codex_err())? {
            model_service_api::ModelResponseEvent::ItemDone { item }
            | model_service_api::ModelResponseEvent::ItemAdded { item } => {
                if let Some(text) = raw_assistant_output_text_from_item(&item) {
                    assistant_output = text;
                }
            }
            model_service_api::ModelResponseEvent::OutputTextDelta { delta } => {
                assistant_output.push_str(&delta);
            }
            model_service_api::ModelResponseEvent::Completed {
                response_id,
                token_usage,
                ..
            } => {
                sess.record_token_usage_info(turn_context, token_usage.as_ref())
                    .await;
                if sess.enabled(Feature::ResponsesWebsocketResponseProcessed) {
                    client_session.send_response_processed(&response_id).await;
                }
                return Ok(if assistant_output.is_empty() {
                    DEFAULT_COMPACTED_MESSAGE.to_string()
                } else {
                    assistant_output
                });
            }
            model_service_api::ModelResponseEvent::Created
            | model_service_api::ModelResponseEvent::ServerModel { .. }
            | model_service_api::ModelResponseEvent::ModelVerifications { .. }
            | model_service_api::ModelResponseEvent::ServerReasoningIncluded { .. }
            | model_service_api::ModelResponseEvent::ToolCallInputDelta { .. }
            | model_service_api::ModelResponseEvent::ReasoningSummaryDelta { .. }
            | model_service_api::ModelResponseEvent::ReasoningContentDelta { .. }
            | model_service_api::ModelResponseEvent::ReasoningSummaryPartAdded { .. }
            | model_service_api::ModelResponseEvent::RateLimits { .. }
            | model_service_api::ModelResponseEvent::ModelsEtag { .. } => {}
        }
    }

    Err(CodexErr::Stream(
        "compact stream closed before response.completed".to_string(),
        None,
    ))
}

pub(crate) async fn send_compact_context_window_error(sess: &Session, turn_context: &TurnContext) {
    let event = EventMsg::Error(ErrorEvent {
        message: COMPACT_CONTEXT_WINDOW_RECOVERY_FAILED_MESSAGE.to_string(),
        codex_error_info: Some(CodexErrorInfo::ContextWindowExceeded),
    });
    sess.send_event(turn_context, event).await;
}

fn compact_turn_final_output(
    history_items: &[ResponseItem],
    compact_prompt: &str,
) -> Option<String> {
    let prompt_index = history_items
        .iter()
        .rposition(|item| is_compact_prompt_control_item(item, compact_prompt))?;
    let compact_turn_items = history_items.get(prompt_index + 1..)?;
    last_assistant_message_from_turn(compact_turn_items)
}

fn compact_prompt_control_item(compact_prompt: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: vec![ContentItem::InputText {
            text: compact_prompt.to_string(),
        }],
        phase: None,
    }
}

fn is_compact_prompt_control_item(item: &ResponseItem, compact_prompt: &str) -> bool {
    matches!(
        item,
        ResponseItem::Message { role, content, .. }
            if role == "developer"
                && content.iter().any(|content_item| matches!(
                    content_item,
                    ContentItem::InputText { text } if text == compact_prompt
                ))
    )
}

pub(crate) struct CompactionAnalyticsAttempt {
    thread_id: String,
    turn_id: String,
    trigger: CompactionTrigger,
    reason: CompactionReason,
    implementation: CompactionImplementation,
    phase: CompactionPhase,
    active_context_tokens_before: i64,
    started_at: u64,
    start_instant: Instant,
}

impl CompactionAnalyticsAttempt {
    pub(crate) async fn begin(
        sess: &Session,
        turn_context: &TurnContext,
        trigger: CompactionTrigger,
        reason: CompactionReason,
        implementation: CompactionImplementation,
        phase: CompactionPhase,
    ) -> Self {
        let active_context_tokens_before = sess.get_total_token_usage().await;
        Self {
            thread_id: sess.conversation_id.to_string(),
            turn_id: turn_context.sub_id.clone(),
            trigger,
            reason,
            implementation,
            phase,
            active_context_tokens_before,
            started_at: now_unix_seconds(),
            start_instant: Instant::now(),
        }
    }

    pub(crate) async fn track(
        self,
        sess: &Session,
        status: CompactionStatus,
        error: Option<String>,
    ) {
        let active_context_tokens_after = sess.get_total_token_usage().await;
        sess.services
            .analytics_events_client
            .track_compaction(CodexCompactionEvent {
                thread_id: self.thread_id,
                turn_id: self.turn_id,
                trigger: self.trigger,
                reason: self.reason,
                implementation: self.implementation,
                phase: self.phase,
                strategy: CompactionStrategy::Memento,
                status,
                error,
                active_context_tokens_before: self.active_context_tokens_before,
                active_context_tokens_after,
                started_at: self.started_at,
                completed_at: now_unix_seconds(),
                duration_ms: Some(
                    u64::try_from(self.start_instant.elapsed().as_millis()).unwrap_or(u64::MAX),
                ),
            });
    }
}

pub(crate) fn compaction_status_from_result<T>(result: &CodexResult<T>) -> CompactionStatus {
    match result {
        Ok(_) => CompactionStatus::Completed,
        Err(CodexErr::Interrupted | CodexErr::TurnAborted) => CompactionStatus::Interrupted,
        Err(_) => CompactionStatus::Failed,
    }
}

pub(crate) fn collect_user_messages(items: &[ResponseItem]) -> Vec<String> {
    codex_context_manager::collect_compaction_user_messages(items, Some(SUMMARY_PREFIX))
}

pub(crate) fn is_summary_message(message: &str) -> bool {
    codex_context_manager::is_compaction_summary_message(message, Some(SUMMARY_PREFIX))
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn insert_initial_context_before_last_real_user_or_summary(
    compacted_history: Vec<ResponseItem>,
    initial_context: Vec<ResponseItem>,
) -> Vec<ResponseItem> {
    codex_context_manager::insert_initial_context_before_last_real_user_or_summary(
        compacted_history,
        initial_context,
        Some(SUMMARY_PREFIX),
    )
}

fn prepend_initial_context_to_memory_checkpoint_history(
    mut compacted_history: Vec<ResponseItem>,
    initial_context: Vec<ResponseItem>,
) -> Vec<ResponseItem> {
    let mut refreshed = initial_context;
    refreshed.append(&mut compacted_history);
    refreshed
}

#[cfg(test)]
pub(crate) fn build_compacted_history(
    initial_context: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
) -> Vec<ResponseItem> {
    codex_context_manager::build_compacted_history(initial_context, user_messages, summary_text)
}

#[cfg(test)]
pub(crate) async fn process_compacted_history(
    sess: &Session,
    turn_context: &TurnContext,
    compacted_history: Vec<ResponseItem>,
    initial_context_injection: InitialContextInjection,
) -> Vec<ResponseItem> {
    let initial_context = if matches!(
        initial_context_injection,
        InitialContextInjection::BeforeLastUserMessage
    ) {
        sess.build_initial_context(turn_context).await
    } else {
        Vec::new()
    };

    process_remote_compacted_history(compacted_history, initial_context)
}

pub(crate) async fn should_auto_compact_in_soft_window(
    sess: &Session,
    turn_context: &TurnContext,
    total_usage_tokens: i64,
    auto_compact_limit: i64,
) -> CodexResult<bool> {
    if auto_compact_limit <= 0 {
        return Ok(false);
    }
    let usage_ratio = total_usage_tokens as f64 / auto_compact_limit as f64;
    let thresholds = auto_compact_thresholds(turn_context);
    if !should_evaluate_auto_compact_decision(usage_ratio, thresholds) {
        return Ok(false);
    }

    let compact_service = FsCompactService::new();
    let replacement_files = compact_replacement_files(turn_context);
    let memory_bundle = match compact_service.read_memory_bundle(&replacement_files).await {
        Ok(bundle) => bundle,
        Err(err) => {
            warn!("skip soft compact because compact replacement files are unavailable: {err}");
            return Ok(false);
        }
    };
    let compact_window_items = sess.compact_window_items().await;
    let compact_window =
        compact_service.summarize_compact_window(&compact_window_items, SUMMARY_PREFIX);
    let current_work_completeness = if memory_bundle.current_work_content().is_some() {
        compact_service.current_work_completeness(&memory_bundle)
    } else {
        1.0
    };
    let decision = compact_service.evaluate_soft_compact(SoftCompactInputs {
        usage_ratio,
        thresholds,
        turns_since_last_compact: compact_window.turns_since_last_compact,
        recent_file_read_search_count: compact_window.recent_file_read_search_count,
        recent_tool_output_bytes: compact_window.recent_tool_output_bytes,
        current_work_completeness,
        cooldown_turns_satisfied: compact_window.turns_since_last_compact >= 2,
        cooldown_bytes_satisfied: compact_window.recent_tool_output_bytes >= 8_000,
    });
    Ok(decision.should_compact)
}

fn auto_compact_thresholds(turn_context: &TurnContext) -> SoftCompactThresholds {
    SoftCompactThresholds::resolve(
        turn_context.config.model_auto_compact_soft_ratio,
        turn_context.config.model_auto_compact_hard_ratio,
    )
    .expect("auto compact thresholds are validated during config load")
}

fn should_evaluate_auto_compact_decision(
    usage_ratio: f64,
    thresholds: SoftCompactThresholds,
) -> bool {
    usage_ratio >= thresholds.soft_lower_bound
}

fn compact_replacement_files(turn_context: &TurnContext) -> Vec<CompactReplacementFile> {
    turn_context
        .config
        .memories
        .compact_replacement_files
        .iter()
        .map(|file| CompactReplacementFile {
            path: file.path.clone(),
            role: match file.role {
                ConfigCompactReplacementFileRole::CurrentWork => CompactMemoryRole::CurrentWork,
                ConfigCompactReplacementFileRole::ProjectUnderstanding => {
                    CompactMemoryRole::ProjectUnderstanding
                }
                ConfigCompactReplacementFileRole::UserPreferences => {
                    CompactMemoryRole::UserPreferences
                }
                ConfigCompactReplacementFileRole::Custom => CompactMemoryRole::Custom,
            },
            label: file.label.clone(),
            token_limit: file.token_limit,
        })
        .collect()
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;
