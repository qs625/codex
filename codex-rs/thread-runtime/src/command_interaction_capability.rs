use codex_command_runtime::CommandSessionError;
use codex_command_runtime::CommandWaitOperation;
use codex_command_runtime::CommandWaitRequest;
use codex_command_runtime::WriteStdinOutput;
use codex_command_runtime::WriteStdinRequest;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_thread_api::ThreadCapability;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

impl codex_thread_api::SessionCommandInteractionCaller for Session {
    async fn begin_command_wait(
        &self,
        request: CommandWaitRequest,
    ) -> Result<Box<dyn CommandWaitOperation>, CommandSessionError> {
        Session::begin_command_wait(self, request).await
    }

    async fn write_command_stdin(
        &self,
        request: WriteStdinRequest<'_>,
    ) -> Result<WriteStdinOutput, CommandSessionError> {
        Session::write_command_stdin(self, request).await
    }

    async fn emit_model_item_started_display_event(
        &self,
        turn: &dyn ThreadCapability,
        item: &ResponseItem,
    ) {
        Session::emit_model_item_started_display_event(self, turn_context_from_capability(turn), item)
            .await;
    }

    async fn record_model_items_and_emit_display_events(
        &self,
        turn: &dyn ThreadCapability,
        items: &[ResponseItem],
    ) {
        Session::record_model_items_and_emit_display_events(
            self,
            turn_context_from_capability(turn),
            items,
        )
        .await;
    }

    async fn send_terminal_interaction(
        &self,
        turn: &dyn ThreadCapability,
        event: TerminalInteractionEvent,
    ) {
        self.send_event(turn_context_from_capability(turn), EventMsg::TerminalInteraction(event))
            .await;
    }
}

fn turn_context_from_capability(capability: &dyn ThreadCapability) -> &TurnContext {
    capability
        .as_any()
        .downcast_ref::<TurnContext>()
        .expect("command interaction capability must be backed by TurnContext")
}
