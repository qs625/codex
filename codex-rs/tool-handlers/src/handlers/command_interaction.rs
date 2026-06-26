use crate::FunctionToolOutput;
use codex_command_runtime::CommandNotificationKind;
use codex_command_runtime::CommandWaitRequest;
use codex_command_runtime::CommandWaitStatus;
use codex_command_runtime::WriteStdinRequest;
use codex_protocol::models::CommandWaitNotificationKind as ResponseCommandWaitNotificationKind;
use codex_protocol::models::CommandWaitStatus as ResponseCommandWaitStatus;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_thread_api::SessionCommandInteractionCaller;
use codex_thread_api::ThreadCapability;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_command_wait_tool;
use codex_tool_planning::create_write_stdin_tool;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolPayload;
use serde::Deserialize;
use serde::Serialize;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const WRITE_STDIN_EMPTY_INPUT_ERROR: &str = "command_write_stdin requires non-empty `chars`; use command_wait for command completion or output notifications instead of polling for output.";

#[derive(Debug, Deserialize)]
struct CommandWaitArgs {
    command_id: i32,
}

pub struct CommandWaitHandler;

impl CommandWaitHandler {
    pub fn new() -> Self {
        Self
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for CommandWaitHandler
where
    Session: SessionCommandInteractionCaller,
    Turn: ThreadCapability,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("command_wait")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_command_wait_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let payload = metadata.payload;

            let arguments = match payload {
                ToolPayload::Function { arguments } => arguments,
                _ => {
                    return Err(FunctionCallError::RespondToModel(
                        "command_wait handler received unsupported payload".to_string(),
                    ));
                }
            };

            let args: CommandWaitArgs = parse_arguments(&arguments)?;
            let item_id = format!("response-item-{}", uuid::Uuid::new_v4());
            let created_at_ms = now_unix_timestamp_ms();
            let command_wait = session
                .begin_command_wait(CommandWaitRequest {
                    process_id: args.command_id,
                })
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!("command_wait failed: {err}"))
                })?;
            let wait_timeout = command_wait.wait_timeout();
            let started_item = command_wait_item(CommandWaitItemInput {
                id: item_id.clone(),
                command_id: command_wait.process_id(),
                status: CommandWaitStatus::Running,
                notification: None,
                exit_code: None,
                wall_time: Duration::ZERO,
                wait_timeout,
                created_at_ms,
            });
            session
                .emit_model_item_started_display_event(&turn, &started_item)
                .await;

            let output = command_wait.finish().await.map_err(|err| {
                FunctionCallError::RespondToModel(format!("command_wait failed: {err}"))
            })?;

            let response_item = command_wait_item(CommandWaitItemInput {
                id: item_id,
                command_id: output.process_id,
                status: output.status.clone(),
                notification: output.notification,
                exit_code: output.exit_code,
                wall_time: output.wall_time,
                wait_timeout: output.wait_timeout,
                created_at_ms,
            });
            session
                .record_model_items_and_emit_display_events(
                    &turn,
                    std::slice::from_ref(&response_item),
                )
                .await;

            let response = CommandWaitResponse {
                command_id: output.process_id,
                status: match &output.status {
                    CommandWaitStatus::Running => "running",
                    CommandWaitStatus::Completed => "completed",
                },
                notification: output.notification.map(|kind| match kind {
                    CommandNotificationKind::Output => "output",
                    CommandNotificationKind::Exit => "exit",
                }),
                exit_code: output.exit_code,
                wall_time_seconds: output.wall_time.as_secs_f64(),
                wait_timeout_ms: output.wait_timeout.as_millis() as i64,
            };
            let text = serde_json::to_string(&response)
                .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
            Ok(FunctionToolOutput {
                body: vec![FunctionCallOutputContentItem::InputText { text }],
                success: Some(true),
                post_tool_use_response: None,
            })
        })
    }
}

impl<Session, Turn, Tracker, DiffContext> ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext>
    for CommandWaitHandler
where
    Session: SessionCommandInteractionCaller,
    Turn: ThreadCapability,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Serialize)]
struct CommandWaitResponse<'a> {
    command_id: i32,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    wall_time_seconds: f64,
    wait_timeout_ms: i64,
}

struct CommandWaitItemInput {
    id: String,
    command_id: i32,
    status: CommandWaitStatus,
    notification: Option<CommandNotificationKind>,
    exit_code: Option<i32>,
    wall_time: Duration,
    wait_timeout: Duration,
    created_at_ms: i64,
}

fn command_wait_item(input: CommandWaitItemInput) -> ResponseItem {
    ResponseItem::CommandWait {
        id: Some(input.id),
        command_id: input.command_id.to_string(),
        status: match input.status {
            CommandWaitStatus::Running => ResponseCommandWaitStatus::Running,
            CommandWaitStatus::Completed => ResponseCommandWaitStatus::Completed,
        },
        notification: input.notification.map(|kind| match kind {
            CommandNotificationKind::Output => ResponseCommandWaitNotificationKind::Output,
            CommandNotificationKind::Exit => ResponseCommandWaitNotificationKind::Exit,
        }),
        exit_code: input.exit_code,
        wall_time_seconds: input.wall_time.as_secs_f64(),
        wait_timeout_ms: input.wait_timeout.as_millis() as i64,
        created_at_ms: input.created_at_ms,
    }
}

#[derive(Debug, Deserialize)]
struct WriteStdinArgs {
    command_id: i32,
    #[serde(default)]
    chars: Option<String>,
}

pub struct WriteStdinHandler;

impl WriteStdinHandler {
    pub fn new() -> Self {
        Self
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for WriteStdinHandler
where
    Session: SessionCommandInteractionCaller,
    Turn: ThreadCapability,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("command_write_stdin")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_write_stdin_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let payload = metadata.payload;

            let arguments = match payload {
                ToolPayload::Function { arguments } => arguments,
                _ => {
                    return Err(FunctionCallError::RespondToModel(
                        "command_write_stdin handler received unsupported payload".to_string(),
                    ));
                }
            };

            let args: WriteStdinArgs = parse_arguments(&arguments)?;
            let Some(chars) = args.chars else {
                return Err(FunctionCallError::RespondToModel(
                    WRITE_STDIN_EMPTY_INPUT_ERROR.to_string(),
                ));
            };
            if chars.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    WRITE_STDIN_EMPTY_INPUT_ERROR.to_string(),
                ));
            }

            let response = session
                .write_command_stdin(WriteStdinRequest {
                    process_id: args.command_id,
                    input: &chars,
                })
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!("command_write_stdin failed: {err}"))
                })?;

            session
                .send_terminal_interaction(
                    &turn,
                    TerminalInteractionEvent {
                        call_id: response.call_id.clone(),
                        process_id: response.process_id.to_string(),
                        stdin: chars.clone(),
                    },
                )
                .await;

            let response_item = ResponseItem::CommandWriteStdin {
                id: None,
                command_id: response.process_id.to_string(),
                bytes_written: response.bytes_written,
                contains_newline: chars.contains('\n'),
                created_at_ms: now_unix_timestamp_ms(),
            };
            session
                .record_model_items_and_emit_display_events(
                    &turn,
                    std::slice::from_ref(&response_item),
                )
                .await;

            let text = serde_json::to_string(&CommandWriteStdinResponse {
                command_id: response.process_id,
                bytes_written: response.bytes_written,
            })
            .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
            Ok(FunctionToolOutput {
                body: vec![FunctionCallOutputContentItem::InputText { text }],
                success: Some(true),
                post_tool_use_response: None,
            })
        })
    }
}

impl<Session, Turn, Tracker, DiffContext> ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext>
    for WriteStdinHandler
where
    Session: SessionCommandInteractionCaller,
    Turn: ThreadCapability,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Serialize)]
struct CommandWriteStdinResponse {
    command_id: i32,
    bytes_written: usize,
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn now_unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_command_runtime::CommandSessionError;
    use codex_command_runtime::CommandWaitOperation;
    use codex_command_runtime::WriteStdinOutput;
    use codex_thread_api::SessionCommandInteractionCaller;
    use codex_tool_runtime_api::CommandInteractionHost;
    use pretty_assertions::assert_eq;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone, Copy)]
    struct StubCommandInteractionHost;

    #[derive(Clone, Copy)]
    struct StubCommandInteractionSession;

    #[derive(Clone, Copy)]
    struct StubCommandInteractionTurn;

    impl ThreadCapability for StubCommandInteractionTurn {
        fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
            self
        }
    }

    impl CommandInteractionHost for StubCommandInteractionHost {
        type Session = StubCommandInteractionSession;
        type Turn = ();
        type Tracker = ();
        type DiffContext = ();

        fn new_response_item_id(&self) -> String {
            "response-item-test".to_string()
        }

        async fn begin_command_wait(
            &self,
            _session: &Self::Session,
            _request: CommandWaitRequest,
        ) -> Result<Box<dyn CommandWaitOperation>, CommandSessionError> {
            panic!("begin_command_wait should not be called by argument validation tests")
        }

        async fn write_command_stdin(
            &self,
            _session: &Self::Session,
            _request: WriteStdinRequest<'_>,
        ) -> Result<WriteStdinOutput, CommandSessionError> {
            panic!("write_command_stdin should not be called by argument validation tests")
        }

        async fn emit_model_item_started_display_event(
            &self,
            _session: &Self::Session,
            _turn: &Self::Turn,
            _item: &ResponseItem,
        ) {
        }

        async fn record_model_items_and_emit_display_events(
            &self,
            _session: &Self::Session,
            _turn: &Self::Turn,
            _items: &[ResponseItem],
        ) {
        }

        async fn send_terminal_interaction(
            &self,
            _session: &Self::Session,
            _turn: &Self::Turn,
            _event: TerminalInteractionEvent,
        ) {
        }
    }

    impl SessionCommandInteractionCaller for StubCommandInteractionSession {
        fn begin_command_wait(
            &self,
            _request: CommandWaitRequest,
        ) -> impl std::future::Future<
            Output = Result<Box<dyn CommandWaitOperation>, CommandSessionError>,
        > + Send {
            async {
                panic!("begin_command_wait should not be called by argument validation tests")
            }
        }

        fn write_command_stdin<'a>(
            &'a self,
            _request: WriteStdinRequest<'a>,
        ) -> impl std::future::Future<Output = Result<WriteStdinOutput, CommandSessionError>> + Send + 'a
        {
            async {
                panic!("write_command_stdin should not be called by argument validation tests")
            }
        }

        fn emit_model_item_started_display_event<'a>(
            &'a self,
            _turn: &'a dyn ThreadCapability,
            _item: &'a ResponseItem,
        ) -> impl std::future::Future<Output = ()> + Send + 'a {
            async {}
        }

        fn record_model_items_and_emit_display_events<'a>(
            &'a self,
            _turn: &'a dyn ThreadCapability,
            _items: &'a [ResponseItem],
        ) -> impl std::future::Future<Output = ()> + Send + 'a {
            async {}
        }

        fn send_terminal_interaction<'a>(
            &'a self,
            _turn: &'a dyn ThreadCapability,
            _event: TerminalInteractionEvent,
        ) -> impl std::future::Future<Output = ()> + Send + 'a {
            async {}
        }
    }

    #[test]
    fn command_wait_started_and_completed_items_reuse_id_and_window() {
        let id = "response-item-wait-1".to_string();
        let started = command_wait_item(CommandWaitItemInput {
            id: id.clone(),
            command_id: 7,
            status: CommandWaitStatus::Running,
            notification: None,
            exit_code: None,
            wall_time: Duration::ZERO,
            wait_timeout: Duration::from_millis(750),
            created_at_ms: 1234,
        });
        let completed = command_wait_item(CommandWaitItemInput {
            id: id.clone(),
            command_id: 7,
            status: CommandWaitStatus::Completed,
            notification: Some(CommandNotificationKind::Exit),
            exit_code: Some(0),
            wall_time: Duration::from_millis(25),
            wait_timeout: Duration::from_millis(750),
            created_at_ms: 1234,
        });

        let ResponseItem::CommandWait {
            id: started_id,
            status: started_status,
            wait_timeout_ms: started_wait_timeout_ms,
            ..
        } = started
        else {
            panic!("expected command wait item");
        };
        let ResponseItem::CommandWait {
            id: completed_id,
            status: completed_status,
            wait_timeout_ms: completed_wait_timeout_ms,
            ..
        } = completed
        else {
            panic!("expected command wait item");
        };

        assert_eq!(started_id, Some(id.clone()));
        assert_eq!(completed_id, Some(id));
        assert_eq!(started_status, ResponseCommandWaitStatus::Running);
        assert_eq!(completed_status, ResponseCommandWaitStatus::Completed);
        assert_eq!(started_wait_timeout_ms, 750);
        assert_eq!(completed_wait_timeout_ms, 750);
    }

    #[tokio::test]
    async fn command_write_stdin_rejects_missing_chars() {
        let result = write_stdin_result_for_arguments(serde_json::json!({
            "command_id": 45
        }))
        .await;
        let Err(err) = result else {
            panic!("missing chars should be rejected");
        };

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(WRITE_STDIN_EMPTY_INPUT_ERROR.to_string())
        );
    }

    #[tokio::test]
    async fn command_write_stdin_rejects_empty_chars() {
        let result = write_stdin_result_for_arguments(serde_json::json!({
            "command_id": 45,
            "chars": ""
        }))
        .await;
        let Err(err) = result else {
            panic!("empty chars should be rejected");
        };

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(WRITE_STDIN_EMPTY_INPUT_ERROR.to_string())
        );
    }

    async fn write_stdin_result_for_arguments(
        arguments: serde_json::Value,
    ) -> Result<FunctionToolOutput, FunctionCallError> {
        let handler = WriteStdinHandler::new();
        handler
            .handle(ToolInvocation {
                session: StubCommandInteractionSession,
                turn: StubCommandInteractionTurn,
                cancellation_token: CancellationToken::new(),
                tracker: (),
                metadata: codex_tool_types::ToolInvocationMetadata {
                    call_id: "write-stdin-call".to_string(),
                    tool_name: ToolName::plain("command_write_stdin"),
                    source: codex_tool_types::ToolCallSource::Direct,
                    payload: ToolPayload::Function {
                        arguments: arguments.to_string(),
                    },
                },
            })
            .await
    }
}
