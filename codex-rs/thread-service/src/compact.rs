use std::sync::Arc;
use std::time::Instant;

use crate::Prompt;
use crate::client_common::ResponseEvent;
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
use codex_turn_items::last_assistant_message_from_turn;
#[cfg(test)]
use codex_turn_items::process_remote_compacted_history;
use futures::prelude::*;
use hooks::PostCompactHookOutcome;
use hooks::PreCompactHookOutcome;
use hooks::run_post_compact_hooks;
use hooks::run_pre_compact_hooks;
use model_service_api::TurnModelRequest;
use protocol::error::CodexErr;
use protocol::error::Result as CodexResult;
use protocol::items::ContextCompactionItem;
use protocol::items::TurnItem;
use protocol::models::ResponseInputItem;
use protocol::models::ResponseItem;
use protocol::protocol::CompactedItem;
use protocol::protocol::EventMsg;
use protocol::protocol::TurnStartedEvent;
use protocol::protocol::WarningEvent;
use protocol::user_input::UserInput;
use rollout_trace_api::InferenceTraceContext;
use tracing::error;

pub const SUMMARIZATION_PROMPT: &str = include_str!("../templates/compact/prompt.md");
pub const SUMMARY_PREFIX: &str = include_str!("../templates/compact/summary_prefix.md");

/// Controls whether compaction replacement history must include initial context.
///
/// Pre-turn/manual compaction variants use `DoNotInject`: they replace history with a summary and
/// clear `reference_context_item`, so the next regular turn will fully reinject initial context
/// after compaction.
///
/// Mid-turn compaction must use `BeforeLastUserMessage` because the model is trained to see the
/// compaction summary as the last item in history after mid-turn compaction; we therefore inject
/// initial context into the replacement history just above the last real user message.
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
    let prompt = turn_context.compact_prompt().to_string();
    let input = vec![UserInput::Text {
        text: prompt,
        // Compaction prompt is synthesized; no UI element ranges to preserve.
        text_elements: Vec::new(),
    }];

    run_compact_task_inner(
        sess,
        turn_context,
        input,
        initial_context_injection,
        CompactionTrigger::Auto,
        reason,
        phase,
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
        InitialContextInjection::DoNotInject,
        CompactionTrigger::Manual,
        CompactionReason::UserRequested,
        CompactionPhase::StandaloneTurn,
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
    input: Vec<UserInput>,
    initial_context_injection: InitialContextInjection,
) -> CodexResult<String> {
    let compaction_item = ContextCompactionItem::new();
    let started_compaction_item = TurnItem::ContextCompaction(compaction_item.clone());
    sess.emit_turn_item_started(&turn_context, &started_compaction_item)
        .await;
    let initial_input_for_turn: ResponseInputItem =
        codex_model_input::response_input_item_from_user_input(input);

    let mut history = sess.clone_history().await;
    history.record_items(
        &[initial_input_for_turn.into()],
        turn_context.truncation_policy,
    );

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

    let completed_response_id = loop {
        // Clone is required because of the loop
        let turn_input = history
            .clone()
            .for_prompt(&turn_context.model_info.input_modalities);
        let turn_input_len = turn_input.len();
        let prompt = Prompt {
            input: turn_input,
            base_instructions: sess.get_base_instructions().await,
            personality: turn_context.personality,
            ..Default::default()
        };
        let turn_metadata_header = turn_context.turn_metadata_state.current_header_value();
        let attempt_result = drain_to_completed(
            &sess,
            turn_context.as_ref(),
            &mut *client_session,
            turn_metadata_header.as_deref(),
            &prompt,
        )
        .await;

        match attempt_result {
            Ok(response_id) => {
                break response_id;
            }
            Err(CodexErr::Interrupted) => {
                return Err(CodexErr::Interrupted);
            }
            Err(e @ CodexErr::ContextWindowExceeded) => {
                if turn_input_len > 1 {
                    // Trim from the beginning to preserve cache (prefix-based) and keep recent messages intact.
                    error!(
                        "Context window exceeded while compacting; removing oldest history item. Error: {e}"
                    );
                    history.remove_first_item();
                    retries = 0;
                    continue;
                }
                sess.set_total_tokens_full(turn_context.as_ref()).await;
                let event = EventMsg::Error(e.to_error_event(/*message_prefix*/ None));
                sess.send_event(&turn_context, event).await;
                return Err(e);
            }
            Err(e) => {
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
    };

    let history_snapshot = sess.clone_history().await;
    let history_items = history_snapshot.raw_items();
    let summary_suffix = last_assistant_message_from_turn(history_items).unwrap_or_default();
    let summary_text = format!("{SUMMARY_PREFIX}\n{summary_suffix}");
    let user_messages = collect_user_messages(history_items);

    let mut new_history = build_compacted_history(Vec::new(), &user_messages, &summary_text);

    if matches!(
        initial_context_injection,
        InitialContextInjection::BeforeLastUserMessage
    ) {
        let initial_context = sess.build_initial_context(turn_context.as_ref()).await;
        new_history =
            insert_initial_context_before_last_real_user_or_summary(new_history, initial_context);
    }
    let reference_context_item = match initial_context_injection {
        InitialContextInjection::DoNotInject => None,
        InitialContextInjection::BeforeLastUserMessage => Some(turn_context.to_turn_context_item()),
    };
    let replacement_history = Some(new_history.clone());
    let compacted_item = CompactedItem {
        message: summary_text.clone(),
        replacement_history: replacement_history.clone(),
    };
    sess.replace_compacted_history(new_history, reference_context_item, compacted_item)
        .await;
    if turn_context
        .features
        .enabled(Feature::ResponsesWebsocketResponseProcessed)
    {
        client_session
            .send_response_processed(&completed_response_id)
            .await;
    }
    client_session.reset_websocket_session();
    sess.recompute_token_usage(&turn_context).await;

    let mut compaction_item_value = serde_json::to_value(&compaction_item).map_err(|err| {
        CodexErr::Fatal(format!(
            "failed to serialize context compaction item: {err}"
        ))
    })?;
    let Some(compaction_item_object) = compaction_item_value.as_object_mut() else {
        return Err(CodexErr::Fatal(
            "failed to serialize context compaction item as object".to_string(),
        ));
    };
    let replacement_history_value = serde_json::to_value(&replacement_history).map_err(|err| {
        CodexErr::Fatal(format!(
            "failed to serialize compact replacement history: {err}"
        ))
    })?;
    compaction_item_object.insert("replacementHistory".to_string(), replacement_history_value);
    let compaction_item: ContextCompactionItem = serde_json::from_value(compaction_item_value)
        .map_err(|err| {
            CodexErr::Fatal(format!(
                "failed to deserialize context compaction item: {err}"
            ))
        })?;
    sess.emit_turn_item_completed(&turn_context, TurnItem::ContextCompaction(compaction_item))
        .await;
    let warning = EventMsg::Warning(WarningEvent {
        message: "Heads up: Long threads and multiple compactions can cause the model to be less accurate. Start a new thread when possible to keep threads small and targeted.".to_string(),
    });
    sess.send_event(&turn_context, warning).await;
    Ok(summary_suffix)
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

async fn drain_to_completed(
    sess: &Session,
    turn_context: &TurnContext,
    client_session: &mut dyn model_service_api::ModelTurnClientApi,
    turn_metadata_header: Option<&str>,
    prompt: &Prompt,
) -> CodexResult<String> {
    let mut stream = client_session
        .stream_responses(TurnModelRequest {
            request: model_service_api::ResponsesModelRequest {
                input: prompt.input.clone(),
                tools: prompt.tools.clone(),
                parallel_tool_calls: prompt.parallel_tool_calls,
                base_instructions: prompt.base_instructions.clone(),
                personality: prompt.personality,
                output_schema: prompt.output_schema.clone(),
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
            // Rollout tracing currently models remote compaction only; local compaction streams
            // are left untraced until the reducer has a first-class local compaction lifecycle.
            inference_trace: InferenceTraceContext::disabled(),
        })
        .await
        .map_err(|err| CodexErr::Stream(err.to_string(), None))?;
    loop {
        let maybe_event = stream.next().await;
        let Some(event) = maybe_event else {
            return Err(CodexErr::Stream(
                "stream closed before response.completed".into(),
                None,
            ));
        };
        match event {
            Ok(event) => match crate::session::turn::map_model_response_event(event) {
                ResponseEvent::OutputItemDone(item) => {
                    sess.record_into_history(std::slice::from_ref(&item), turn_context)
                        .await;
                }
                ResponseEvent::ServerReasoningIncluded(included) => {
                    sess.set_server_reasoning_included(included).await;
                }
                ResponseEvent::RateLimits(snapshot) => {
                    sess.update_rate_limits(turn_context, snapshot).await;
                }
                ResponseEvent::Completed {
                    response_id,
                    token_usage,
                    ..
                } => {
                    sess.update_token_usage_info(turn_context, token_usage.as_ref())
                        .await;
                    return Ok(response_id);
                }
                _ => continue,
            },
            Err(err) => return Err(CodexErr::Stream(err.to_string(), None)),
        }
    }
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;
