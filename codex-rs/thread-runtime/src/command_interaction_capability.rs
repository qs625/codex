use codex_command_runtime::CommandSessionError;
use codex_command_runtime::CommandWaitOperation;
use codex_command_runtime::CommandWaitRequest;
use codex_command_runtime::WriteStdinOutput;
use codex_command_runtime::WriteStdinRequest;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_command_service_api::SessionCommandInteractionCaller;
use codex_thread_api::ThreadCapability;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

impl SessionCommandInteractionCaller for Session {
    fn begin_command_wait<'a>(
        &'a self,
        request: CommandWaitRequest,
    ) -> codex_command_service_api::CommandServiceFuture<
        'a,
        Result<Box<dyn CommandWaitOperation>, CommandSessionError>,
    > {
        Box::pin(async move { Session::begin_command_wait(self, request).await })
    }

    fn write_command_stdin<'a>(
        &'a self,
        request: WriteStdinRequest<'a>,
    ) -> codex_command_service_api::CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>> {
        Box::pin(async move { Session::write_command_stdin(self, request).await })
    }

    fn emit_model_item_started_display_event<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        item: &'a ResponseItem,
    ) -> codex_command_service_api::CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            Session::emit_model_item_started_display_event(
                self,
                turn_context_from_capability(turn),
                item,
            )
            .await;
        })
    }

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        items: &'a [ResponseItem],
    ) -> codex_command_service_api::CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            Session::record_model_items_and_emit_display_events(
                self,
                turn_context_from_capability(turn),
                items,
            )
            .await;
        })
    }

    fn send_terminal_interaction<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        event: TerminalInteractionEvent,
    ) -> codex_command_service_api::CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            self.send_event(
                turn_context_from_capability(turn),
                EventMsg::TerminalInteraction(event),
            )
            .await;
        })
    }
}

fn turn_context_from_capability(capability: &dyn ThreadCapability) -> &TurnContext {
    capability
        .as_any()
        .downcast_ref::<TurnContext>()
        .expect("command interaction capability must be backed by TurnContext")
}
