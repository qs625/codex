use super::ThreadHistoryBuilder;
use super::support::convert_dynamic_tool_content_items;
use crate::protocol::event_item_projection::ProjectedEventItem;
use crate::protocol::event_item_projection::project_event_msg_item;
use crate::protocol::item_builders::build_command_execution_begin_item;
use crate::protocol::item_builders::build_command_execution_end_item;
use crate::protocol::item_builders::build_file_change_approval_request_item;
use crate::protocol::item_builders::build_file_change_begin_item;
use crate::protocol::item_builders::build_file_change_end_item;
use crate::protocol::item_builders::build_item_from_guardian_event;
use crate::protocol::CommandExecutionStatus;
use crate::protocol::DynamicToolCallStatus;
use crate::protocol::McpToolCallError;
use crate::protocol::McpToolCallResult;
use crate::protocol::McpToolCallStatus;
use crate::protocol::ThreadItem;
use crate::protocol::WebSearchAction;
use protocol::protocol::ApplyPatchApprovalRequestEvent;
use protocol::protocol::DynamicToolCallResponseEvent;
use protocol::protocol::EventMsg;
use protocol::protocol::ExecCommandBeginEvent;
use protocol::protocol::ExecCommandEndEvent;
use protocol::protocol::ExecCommandOutputDeltaEvent;
use protocol::protocol::GuardianAssessmentEvent;
use protocol::protocol::GuardianAssessmentStatus;
use protocol::protocol::ImageGenerationBeginEvent;
use protocol::protocol::ImageGenerationEndEvent;
use protocol::protocol::McpToolCallBeginEvent;
use protocol::protocol::McpToolCallEndEvent;
use protocol::protocol::PatchApplyBeginEvent;
use protocol::protocol::PatchApplyEndEvent;
use protocol::protocol::ViewImageToolCallEvent;
use protocol::protocol::WebSearchBeginEvent;
use protocol::protocol::WebSearchEndEvent;

impl ThreadHistoryBuilder {
    pub(super) fn handle_projected_event_item(&mut self, event: &EventMsg) {
        let Some(projected) = project_event_msg_item(event) else {
            return;
        };
        match projected {
            ProjectedEventItem::Started { turn_id, item, .. }
            | ProjectedEventItem::Completed { turn_id, item, .. } => {
                self.upsert_item_in_turn_id_or_create(&turn_id, item);
            }
        }
    }

    pub(super) fn handle_web_search_begin(&mut self, payload: &WebSearchBeginEvent) {
        let item = ThreadItem::WebSearch {
            id: payload.call_id.clone(),
            query: String::new(),
            action: None,
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_web_search_end(&mut self, payload: &WebSearchEndEvent) {
        let item = ThreadItem::WebSearch {
            id: payload.call_id.clone(),
            query: payload.query.clone(),
            action: Some(WebSearchAction::from(payload.action.clone())),
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_exec_command_begin(&mut self, payload: &ExecCommandBeginEvent) {
        let item = build_command_execution_begin_item(payload);
        self.upsert_item_in_turn_id(&payload.turn_id, item);
    }

    pub(super) fn handle_exec_command_output_delta(
        &mut self,
        _payload: &ExecCommandOutputDeltaEvent,
    ) {
    }

    pub(super) fn handle_exec_command_end(&mut self, payload: &ExecCommandEndEvent) {
        let item = build_command_execution_end_item(payload);
        self.upsert_item_in_turn_id(&payload.turn_id, item);
    }

    pub(super) fn handle_guardian_assessment(&mut self, payload: &GuardianAssessmentEvent) {
        let status = match payload.status {
            GuardianAssessmentStatus::InProgress => CommandExecutionStatus::InProgress,
            GuardianAssessmentStatus::Denied | GuardianAssessmentStatus::Aborted => {
                CommandExecutionStatus::Declined
            }
            GuardianAssessmentStatus::TimedOut => CommandExecutionStatus::Failed,
            GuardianAssessmentStatus::Approved => return,
        };
        let Some(item) = build_item_from_guardian_event(payload, status) else {
            return;
        };
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    pub(super) fn handle_apply_patch_approval_request(
        &mut self,
        payload: &ApplyPatchApprovalRequestEvent,
    ) {
        let item = build_file_change_approval_request_item(payload);
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    pub(super) fn handle_patch_apply_begin(&mut self, payload: &PatchApplyBeginEvent) {
        let item = build_file_change_begin_item(payload);
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    pub(super) fn handle_patch_apply_end(&mut self, payload: &PatchApplyEndEvent) {
        let item = build_file_change_end_item(payload);
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    pub(super) fn handle_dynamic_tool_call_request(
        &mut self,
        payload: &protocol::dynamic_tools::DynamicToolCallRequest,
    ) {
        let item = ThreadItem::DynamicToolCall {
            id: payload.call_id.clone(),
            namespace: payload.namespace.clone(),
            tool: payload.tool.clone(),
            arguments: payload.arguments.clone(),
            status: DynamicToolCallStatus::InProgress,
            content_items: None,
            success: None,
            duration_ms: None,
        };
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    pub(super) fn handle_dynamic_tool_call_response(
        &mut self,
        payload: &DynamicToolCallResponseEvent,
    ) {
        let status = if payload.success {
            DynamicToolCallStatus::Completed
        } else {
            DynamicToolCallStatus::Failed
        };
        let duration_ms = i64::try_from(payload.duration.as_millis()).ok();
        let item = ThreadItem::DynamicToolCall {
            id: payload.call_id.clone(),
            namespace: payload.namespace.clone(),
            tool: payload.tool.clone(),
            arguments: payload.arguments.clone(),
            status,
            content_items: Some(convert_dynamic_tool_content_items(&payload.content_items)),
            success: Some(payload.success),
            duration_ms,
        };
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    pub(super) fn handle_mcp_tool_call_begin(&mut self, payload: &McpToolCallBeginEvent) {
        let item = ThreadItem::McpToolCall {
            id: payload.call_id.clone(),
            server: payload.invocation.server.clone(),
            tool: payload.invocation.tool.clone(),
            status: McpToolCallStatus::InProgress,
            arguments: payload
                .invocation
                .arguments
                .clone()
                .unwrap_or(serde_json::Value::Null),
            mcp_app_resource_uri: payload.mcp_app_resource_uri.clone(),
            result: None,
            error: None,
            duration_ms: None,
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_mcp_tool_call_end(&mut self, payload: &McpToolCallEndEvent) {
        let status = if payload.is_success() {
            McpToolCallStatus::Completed
        } else {
            McpToolCallStatus::Failed
        };
        let duration_ms = i64::try_from(payload.duration.as_millis()).ok();
        let (result, error) = match &payload.result {
            Ok(value) => (
                Some(Box::new(McpToolCallResult {
                    content: value.content.clone(),
                    structured_content: value.structured_content.clone(),
                    meta: value.meta.clone(),
                })),
                None,
            ),
            Err(message) => (
                None,
                Some(McpToolCallError {
                    message: message.clone(),
                }),
            ),
        };
        let item = ThreadItem::McpToolCall {
            id: payload.call_id.clone(),
            server: payload.invocation.server.clone(),
            tool: payload.invocation.tool.clone(),
            status,
            arguments: payload
                .invocation
                .arguments
                .clone()
                .unwrap_or(serde_json::Value::Null),
            mcp_app_resource_uri: payload.mcp_app_resource_uri.clone(),
            result,
            error,
            duration_ms,
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_view_image_tool_call(&mut self, payload: &ViewImageToolCallEvent) {
        let item = ThreadItem::ImageView {
            id: payload.call_id.clone(),
            path: payload.path.clone(),
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_image_generation_begin(
        &mut self,
        payload: &ImageGenerationBeginEvent,
    ) {
        let item = ThreadItem::ImageGeneration {
            id: payload.call_id.clone(),
            status: String::new(),
            revised_prompt: None,
            result: String::new(),
            saved_path: None,
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_image_generation_end(
        &mut self,
        payload: &ImageGenerationEndEvent,
    ) {
        let item = ThreadItem::ImageGeneration {
            id: payload.call_id.clone(),
            status: payload.status.clone(),
            revised_prompt: payload.revised_prompt.clone(),
            result: payload.result.clone(),
            saved_path: payload.saved_path.clone(),
        };
        self.upsert_item_in_current_turn(item);
    }
}
