use super::*;
use codex_extension_api::ExtensionData;
use codex_extension_api::TurnItemContributionFuture;
use codex_extension_api::TurnItemContributor;
use pretty_assertions::assert_eq;
use protocol::items::AgentMessageContent;
use protocol::models::ContentItem;
use protocol::models::FunctionCallOutputPayload;
use std::sync::Arc;

use crate::stream_events_utils::InFlightToolResult;

struct RewriteAgentMessageContributor;

impl TurnItemContributor for RewriteAgentMessageContributor {
    fn contribute<'a>(
        &'a self,
        _thread_store: &'a ExtensionData,
        _turn_store: &'a ExtensionData,
        item: &'a mut TurnItem,
    ) -> TurnItemContributionFuture<'a> {
        Box::pin(async move {
            if let TurnItem::AgentMessage(agent_message) = item {
                agent_message.content = vec![AgentMessageContent::Text {
                    text: "plan contributed assistant text".to_string(),
                }];
            }
            Ok(())
        })
    }
}

fn assistant_output_text(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: Some("msg-1".to_string()),
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn history_with_function_transaction(call_id: &str) -> codex_context_manager::ContextManager {
    let items = vec![
        ResponseItem::FunctionCall {
            id: None,
            name: "lookup".to_string(),
            namespace: None,
            arguments: "{}".to_string(),
            call_id: call_id.to_string(),
        },
        ResponseItem::FunctionCallOutput {
            call_id: call_id.to_string(),
            output: FunctionCallOutputPayload::from_text("done".to_string()),
        },
    ];
    let mut history = codex_context_manager::ContextManager::new();
    history.record_items(
        items.iter(),
        codex_utils_output_truncation::TruncationPolicy::Tokens(10_000),
    );
    history
}

#[test]
fn invalid_model_input_recovery_requires_exact_complete_transaction() {
    let target = ModelInputItemReference {
        kind: protocol::error::ModelInputItemKind::FunctionCall,
        call_id: "call-1".to_string(),
    };
    let error = InvalidModelInputError {
        message: "invalid".to_string(),
        error_type: Some("invalid_request_error".to_string()),
        code: Some("invalid_value".to_string()),
        param: Some("input[3].arguments.outer".to_string()),
        input_index: Some(3),
        source: Some(target.clone()),
    };

    assert_eq!(
        recoverable_model_context_target(
            &error,
            &history_with_function_transaction("call-1"),
            &HashSet::new(),
        ),
        Some(target)
    );
}

#[test]
fn invalid_model_input_recovery_does_not_guess_for_malformed_param() {
    let target = ModelInputItemReference {
        kind: protocol::error::ModelInputItemKind::FunctionCall,
        call_id: "call-1".to_string(),
    };
    let error = InvalidModelInputError {
        message: "invalid".to_string(),
        error_type: Some("invalid_request_error".to_string()),
        code: Some("invalid_value".to_string()),
        param: Some("input[-1].arguments".to_string()),
        input_index: None,
        source: Some(target),
    };

    assert!(
        recoverable_model_context_target(
            &error,
            &history_with_function_transaction("call-1"),
            &HashSet::new(),
        )
        .is_none()
    );
}

#[tokio::test]
async fn terminal_tool_outcome_finishes_sampling_without_model_output() {
    let (session, turn_context) = crate::session::tests::make_session_and_context().await;
    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let mut in_flight = FuturesOrdered::new();
    let terminal: InFlightFuture<'static> = Box::pin(async { Ok(InFlightToolResult::FinishTurn) });
    in_flight.push_back(terminal);

    assert!(
        drain_in_flight(&mut in_flight, session, turn_context)
            .await
            .expect("terminal result should drain")
    );
}

#[tokio::test]
async fn plan_mode_uses_contributed_turn_item_for_last_agent_message() {
    let (mut session, turn_context) = crate::session::tests::make_session_and_context().await;
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.turn_item_contributor(Arc::new(RewriteAgentMessageContributor));
    session.services.extensions = Arc::new(builder.build());
    let turn_store = ExtensionData::new(turn_context.sub_id.clone());
    let mut state = PlanModeStreamState::new(&turn_context.sub_id);
    let mut last_agent_message = None;
    let item = assistant_output_text("original assistant text");

    let handled = handle_assistant_item_done_in_plan_mode(
        &session,
        &turn_context,
        &turn_store,
        &item,
        &mut state,
        /*previously_active_item*/ None,
        &mut last_agent_message,
    )
    .await;

    assert!(handled);
    assert_eq!(
        last_agent_message.as_deref(),
        Some("plan contributed assistant text")
    );
}
