use std::pin::Pin;
use std::sync::Arc;

use codex_extension_api::ExtensionData;
use codex_protocol::config_types::ModeKind;
use codex_protocol::items::TurnItem;
use codex_turn_items::FinalizedTurnItemFacts;
use codex_turn_items::completed_item_defers_mailbox_delivery_to_next_turn;
use codex_turn_items::finalize_agent_message_content;
use codex_turn_items::finalized_turn_item_facts;
use codex_turn_items::raw_assistant_output_text_from_item;
use codex_turn_items::response_input_to_response_item;
use codex_turn_items::response_item_may_include_external_context;
use codex_utils_stream_parser::strip_citations;
use tokio_util::sync::CancellationToken;

use crate::SharedTurnDiffTracker;
use crate::parse_turn_item;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::state_db_bridge as state_db;
use codex_context_manager::ContextualUserFragment;
use codex_context_manager::ImageGenerationInstructions;
use codex_memories_read_api::citations::parse_memory_citation;
use codex_memories_read_api::citations::thread_ids_from_memory_citation;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::memory_citation::MemoryCitation;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_image::decode_base64_image_bytes;
use futures::Future;
use tracing::debug;
use tracing::instrument;
use tracing::warn;

const GENERATED_IMAGE_ARTIFACTS_DIR: &str = "generated_images";

pub(crate) fn image_generation_artifact_path(
    codex_home: &AbsolutePathBuf,
    session_id: &str,
    call_id: &str,
) -> AbsolutePathBuf {
    let sanitize = |value: &str| {
        let mut sanitized: String = value
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        if sanitized.is_empty() {
            sanitized = "generated_image".to_string();
        }
        sanitized
    };

    codex_home
        .join(GENERATED_IMAGE_ARTIFACTS_DIR)
        .join(sanitize(session_id))
        .join(format!("{}.png", sanitize(call_id)))
}

async fn save_image_generation_result(
    codex_home: &AbsolutePathBuf,
    session_id: &str,
    call_id: &str,
    result: &str,
) -> Result<AbsolutePathBuf> {
    let bytes = decode_base64_image_bytes(result.trim().as_bytes()).map_err(|err| {
        CodexErr::InvalidRequest(format!("invalid image generation payload: {err}"))
    })?;
    let path = image_generation_artifact_path(codex_home, session_id, call_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, bytes).await?;
    Ok(path)
}

/// Persist a completed model response item and record any cited memory usage.
pub(crate) async fn record_completed_response_item(
    sess: &Session,
    turn_context: &TurnContext,
    item: &ResponseItem,
) {
    record_completed_response_item_with_finalized_facts(
        sess,
        turn_context,
        item,
        /*finalized_facts*/ None,
    )
    .await;
}

pub(crate) async fn record_completed_response_item_with_finalized_facts(
    sess: &Session,
    turn_context: &TurnContext,
    item: &ResponseItem,
    finalized_facts: Option<&FinalizedTurnItemFacts>,
) {
    sess.record_conversation_items(turn_context, std::slice::from_ref(item))
        .await;
    let defers_mailbox_delivery = finalized_facts.map_or_else(
        || {
            completed_item_defers_mailbox_delivery_to_next_turn(
                item,
                turn_context.collaboration_mode.mode == ModeKind::Plan,
            )
        },
        |facts| facts.defers_mailbox_delivery_to_next_turn,
    );
    if defers_mailbox_delivery {
        sess.defer_mailbox_delivery_to_next_turn(&turn_context.sub_id)
            .await;
    }
    mark_thread_memory_mode_polluted_if_external_context(sess, turn_context, item).await;
    let has_memory_citation = if let Some(memory_citation) =
        finalized_facts.and_then(|facts| facts.memory_citation.as_ref())
    {
        record_stage1_output_usage_for_memory_citation(
            sess.services.state_db.as_ref(),
            memory_citation,
        )
        .await
    } else {
        record_stage1_output_usage_and_detect_memory_citation(sess.services.state_db.as_ref(), item)
            .await
    };
    if has_memory_citation {
        sess.record_memory_citation_for_turn(&turn_context.sub_id)
            .await;
    }
}

pub(crate) async fn mark_thread_memory_mode_polluted_if_external_context(
    sess: &Session,
    turn_context: &TurnContext,
    item: &ResponseItem,
) {
    if !turn_context.config.memories.disable_on_external_context
        || !response_item_may_include_external_context(item)
    {
        return;
    }
    state_db::mark_thread_memory_mode_polluted(
        sess.services.state_db.as_deref(),
        sess.conversation_id,
        "record_completed_response_item",
    )
    .await;
}

async fn record_stage1_output_usage_and_detect_memory_citation(
    state_db_ctx: Option<&state_db::StateDbHandle>,
    item: &ResponseItem,
) -> bool {
    let Some(raw_text) = raw_assistant_output_text_from_item(item) else {
        return false;
    };

    let (_, citations) = strip_citations(&raw_text);
    let Some(memory_citation) = parse_memory_citation(citations) else {
        return false;
    };
    record_stage1_output_usage_for_memory_citation(state_db_ctx, &memory_citation).await
}

async fn record_stage1_output_usage_for_memory_citation(
    state_db_ctx: Option<&state_db::StateDbHandle>,
    memory_citation: &MemoryCitation,
) -> bool {
    let thread_ids = thread_ids_from_memory_citation(memory_citation);
    if thread_ids.is_empty() {
        return true;
    }

    state_db::record_stage1_output_usage(
        state_db_ctx.map(std::sync::Arc::as_ref),
        &thread_ids,
        "memory_citation",
    )
    .await;
    true
}

/// Handle a completed output item from the model stream, recording it and
/// queuing any tool execution futures. This records items immediately so
/// history and rollout stay in sync even if the turn is later cancelled.
pub(crate) type InFlightFuture<'f> =
    Pin<Box<dyn Future<Output = Result<ResponseItem>> + Send + 'f>>;

#[derive(Default)]
pub(crate) struct OutputItemResult {
    pub last_agent_message: Option<String>,
    pub needs_follow_up: bool,
    pub tool_future: Option<InFlightFuture<'static>>,
}

pub(crate) struct HandleOutputCtx {
    pub sess: Arc<Session>,
    pub turn_context: Arc<TurnContext>,
    pub turn_store: Arc<ExtensionData>,
    pub tool_inputs: Arc<crate::session::turn::TurnToolInputs>,
    pub turn_diff_tracker: SharedTurnDiffTracker,
    pub cancellation_token: CancellationToken,
}

async fn apply_turn_item_contributors(
    sess: &Session,
    turn_store: &ExtensionData,
    item: &mut TurnItem,
) {
    let contributors = sess.services.extensions.turn_item_contributors().to_vec();
    for contributor in contributors {
        if let Err(err) = contributor
            .contribute(&sess.services.thread_extension_data, turn_store, item)
            .await
        {
            warn!("turn item contributor failed: {err}");
        }
    }
}

pub(crate) enum TurnItemContributorPolicy<'a> {
    Skip,
    Run(&'a ExtensionData),
}

pub(crate) struct FinalizedTurnItem {
    pub(crate) turn_item: TurnItem,
    pub(crate) facts: FinalizedTurnItemFacts,
}

pub(crate) async fn finalize_non_tool_response_item(
    sess: &Session,
    turn_context: &TurnContext,
    contributor_policy: TurnItemContributorPolicy<'_>,
    item: &ResponseItem,
    plan_mode: bool,
) -> Option<FinalizedTurnItem> {
    let turn_item =
        handle_non_tool_response_item(sess, turn_context, contributor_policy, item, plan_mode)
            .await?;
    let facts = finalized_turn_item_facts(&turn_item);
    Some(FinalizedTurnItem { turn_item, facts })
}

#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_output_item_done(
    ctx: &mut HandleOutputCtx,
    item: ResponseItem,
    previously_active_item: Option<TurnItem>,
) -> Result<OutputItemResult> {
    let mut output = OutputItemResult::default();
    let plan_mode = ctx.turn_context.collaboration_mode.mode == ModeKind::Plan;

    match ToolCall::from_response_item(item.clone()) {
        // The model emitted a tool call; log it, persist the item immediately, and queue the tool execution.
        Ok(Some(call)) => {
            ctx.sess
                .accept_mailbox_delivery_for_current_turn(&ctx.turn_context.sub_id)
                .await;

            let payload_preview = call.payload.log_payload().into_owned();
            tracing::info!(
                thread_id = %ctx.sess.conversation_id,
                "ToolCall: {} {}",
                call.tool_name,
                payload_preview
            );

            record_completed_response_item(ctx.sess.as_ref(), ctx.turn_context.as_ref(), &item)
                .await;

            let cancellation_token = ctx.cancellation_token.child_token();
            let tool_future: InFlightFuture<'static> =
                Box::pin(crate::session::turn::handle_tool_call(
                    Arc::clone(&ctx.sess.services.tool_service),
                    Arc::clone(&ctx.sess),
                    Arc::clone(&ctx.turn_context),
                    Arc::clone(&ctx.tool_inputs),
                    Arc::clone(&ctx.turn_diff_tracker),
                    call,
                    cancellation_token,
                ));

            output.needs_follow_up = true;
            output.tool_future = Some(tool_future);
        }
        // No tool call: convert messages/reasoning into turn items and mark them as complete.
        Ok(None) => {
            let finalized_turn_item = finalize_non_tool_response_item(
                ctx.sess.as_ref(),
                ctx.turn_context.as_ref(),
                TurnItemContributorPolicy::Run(ctx.turn_store.as_ref()),
                &item,
                plan_mode,
            )
            .await;
            let finalized_facts = finalized_turn_item
                .as_ref()
                .map(|finalized| finalized.facts.clone());
            if let Some(finalized_turn_item) = finalized_turn_item {
                if previously_active_item.is_none() {
                    let mut started_item = finalized_turn_item.turn_item.clone();
                    if let TurnItem::ImageGeneration(item) = &mut started_item {
                        item.status = "in_progress".to_string();
                        item.revised_prompt = None;
                        item.result.clear();
                        item.saved_path = None;
                    }
                    ctx.sess
                        .emit_turn_item_started(&ctx.turn_context, &started_item)
                        .await;
                }

                ctx.sess
                    .emit_turn_item_completed(&ctx.turn_context, finalized_turn_item.turn_item)
                    .await;
            }
            record_completed_response_item_with_finalized_facts(
                ctx.sess.as_ref(),
                ctx.turn_context.as_ref(),
                &item,
                finalized_facts.as_ref(),
            )
            .await;

            output.last_agent_message = finalized_facts.and_then(|facts| facts.last_agent_message);
        }
        // The tool request should be answered directly (or was denied); push that response into the transcript.
        Err(FunctionCallError::RespondToModel(message)) => {
            let response = ResponseInputItem::FunctionCallOutput {
                call_id: String::new(),
                output: FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text(message),
                    ..Default::default()
                },
            };
            record_completed_response_item(ctx.sess.as_ref(), ctx.turn_context.as_ref(), &item)
                .await;
            if let Some(response_item) = response_input_to_response_item(&response) {
                ctx.sess
                    .record_conversation_items(
                        &ctx.turn_context,
                        std::slice::from_ref(&response_item),
                    )
                    .await;
            }

            output.needs_follow_up = true;
        }
        // A fatal error occurred; surface it back into history.
        Err(FunctionCallError::Fatal(message)) => {
            return Err(CodexErr::Fatal(message));
        }
    }

    Ok(output)
}

pub(crate) async fn handle_non_tool_response_item(
    sess: &Session,
    turn_context: &TurnContext,
    contributor_policy: TurnItemContributorPolicy<'_>,
    item: &ResponseItem,
    plan_mode: bool,
) -> Option<TurnItem> {
    debug!(?item, "Output item");

    match item {
        ResponseItem::Message { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. } => {
            let mut turn_item = parse_turn_item(item)?;
            if let TurnItemContributorPolicy::Run(turn_store) = contributor_policy {
                apply_turn_item_contributors(sess, turn_store, &mut turn_item).await;
            }
            if let TurnItem::AgentMessage(agent_message) = &mut turn_item {
                finalize_agent_message_content(agent_message, plan_mode);
            }
            if let TurnItem::ImageGeneration(image_item) = &mut turn_item {
                let session_id = sess.conversation_id.to_string();
                match save_image_generation_result(
                    &turn_context.config.codex_home,
                    &session_id,
                    &image_item.id,
                    &image_item.result,
                )
                .await
                {
                    Ok(path) => {
                        image_item.saved_path = Some(path);
                        let image_output_path = image_generation_artifact_path(
                            &turn_context.config.codex_home,
                            &session_id,
                            "<image_id>",
                        );
                        let image_output_dir = image_output_path
                            .parent()
                            .unwrap_or_else(|| turn_context.config.codex_home.clone());
                        let message: ResponseItem =
                            ContextualUserFragment::into(ImageGenerationInstructions::new(
                                image_output_dir.display(),
                                image_output_path.display(),
                            ));
                        sess.record_conversation_items(turn_context, &[message])
                            .await;
                    }
                    Err(err) => {
                        let output_path = image_generation_artifact_path(
                            &turn_context.config.codex_home,
                            &session_id,
                            &image_item.id,
                        );
                        let output_dir = output_path
                            .parent()
                            .unwrap_or_else(|| turn_context.config.codex_home.clone());
                        tracing::warn!(
                            call_id = %image_item.id,
                            output_dir = %output_dir.display(),
                            "failed to save generated image: {err}"
                        );
                    }
                }
            }
            Some(turn_item)
        }
        ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. } => {
            debug!("unexpected tool output from stream");
            None
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "stream_events_utils_tests.rs"]
mod tests;
