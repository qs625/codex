use std::sync::Arc;

use command_service_api::CommandNotificationFilter;
use command_service_api::CommandServiceSessionState;
use command_service_api::RunningCommandSnapshot;
use protocol::ThreadId;
use protocol::subscriptions::PersistedSubscription;
use protocol::subscriptions::ScheduleSpec;
use serde::Deserialize;
use serde::Serialize;
use thread_service_api::ThreadSessionCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;

use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;
use crate::planning::ToolSpec;
use crate::planning::create_list_commands_tool;
use crate::planning::create_list_subscriptions_tool;

const LIST_COMMANDS_TOOL_NAME: &str = "list_commands";
const LIST_SUBSCRIPTIONS_TOOL_NAME: &str = "list_subscriptions";

pub(crate) fn specs(_request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    vec![
        create_list_commands_tool(),
        create_list_subscriptions_tool(),
    ]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            LIST_COMMANDS_TOOL_NAME | LIST_SUBSCRIPTIONS_TOOL_NAME
        )
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) async fn dispatch(
    command_state: Arc<dyn CommandServiceSessionState>,
    session: Arc<dyn ThreadSessionCapability>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let _args: EmptyArgs = parse_arguments(&call)?;
    let result = match call.tool_name.name.as_str() {
        LIST_COMMANDS_TOOL_NAME => {
            let result =
                list_commands_for_thread(command_state.as_ref(), session.conversation_id()).await;
            function_tool_json_output(&result, LIST_COMMANDS_TOOL_NAME)?
        }
        LIST_SUBSCRIPTIONS_TOOL_NAME => {
            let result = list_subscriptions_for_session(session.as_ref()).await;
            function_tool_json_output(&result, LIST_SUBSCRIPTIONS_TOOL_NAME)?
        }
        _ => {
            return Err(FunctionCallError::Fatal(format!(
                "unsupported runtime state tool {}",
                call.tool_name
            )));
        }
    };

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(result),
        post_tool_use_payload: None,
    })
}

async fn list_commands_for_thread(
    command_state: &dyn CommandServiceSessionState,
    thread_id: ThreadId,
) -> ListCommandsOutput {
    let commands = command_state
        .running_processes_for_thread(thread_id)
        .await
        .into_iter()
        .map(RunningCommandSummary::from)
        .collect();
    ListCommandsOutput { commands }
}

async fn list_subscriptions_for_session(
    session: &dyn ThreadSessionCapability,
) -> ListSubscriptionsOutput {
    let subscriptions = session
        .active_subscriptions()
        .await
        .into_iter()
        .map(ActiveSubscriptionSummary::from)
        .collect();
    ListSubscriptionsOutput { subscriptions }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ListCommandsOutput {
    commands: Vec<RunningCommandSummary>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct RunningCommandSummary {
    command_id: i32,
    call_id: String,
    label: String,
    tty: bool,
    notify_on: &'static str,
    cwd: String,
    command_text: String,
}

impl From<RunningCommandSnapshot> for RunningCommandSummary {
    fn from(command: RunningCommandSnapshot) -> Self {
        Self {
            command_id: command.process_id,
            call_id: command.call_id,
            label: command_label(&command.command),
            tty: command.tty,
            notify_on: match command.notify_on {
                CommandNotificationFilter::Output => "output",
                CommandNotificationFilter::Exit => "exit",
            },
            cwd: command.cwd.to_string_lossy().to_string(),
            command_text: command.command,
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ListSubscriptionsOutput {
    subscriptions: Vec<ActiveSubscriptionSummary>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ActiveSubscriptionSummary {
    Fs {
        subscription_id: String,
        label: Option<String>,
        status: &'static str,
        path: String,
        recursive: bool,
    },
    EventCommand {
        subscription_id: String,
        label: Option<String>,
        status: &'static str,
        command_text: String,
        cwd: Option<String>,
    },
    Schedule {
        subscription_id: String,
        label: Option<String>,
        status: &'static str,
        schedule: ScheduleSpec,
        message: Option<String>,
    },
    ProcessExit {
        subscription_id: String,
        label: Option<String>,
        status: &'static str,
        session_id: i32,
    },
}

impl From<PersistedSubscription> for ActiveSubscriptionSummary {
    fn from(subscription: PersistedSubscription) -> Self {
        match subscription {
            PersistedSubscription::Fs {
                subscription_id,
                path,
                recursive,
                label,
            } => Self::Fs {
                subscription_id,
                label,
                status: "active",
                path,
                recursive,
            },
            PersistedSubscription::EventCommand {
                subscription_id,
                command,
                cwd,
                label,
            } => Self::EventCommand {
                subscription_id,
                label,
                status: "active",
                command_text: command,
                cwd,
            },
            PersistedSubscription::Schedule {
                subscription_id,
                schedule,
                label,
                message,
            } => Self::Schedule {
                subscription_id,
                label,
                status: "active",
                schedule,
                message,
            },
            PersistedSubscription::ProcessExit {
                subscription_id,
                session_id,
                label,
            } => Self::ProcessExit {
                subscription_id,
                label,
                status: "active",
                session_id,
            },
        }
    }
}

fn parse_arguments<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(call.function_arguments()?).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to parse {} arguments: {err}",
            call.tool_name
        ))
    })
}

fn function_tool_json_output<T>(
    value: &T,
    tool_name: &str,
) -> Result<FunctionToolOutput, FunctionCallError>
where
    T: Serialize,
{
    serde_json::to_string(value)
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize {tool_name} result: {err}"))
        })
}

fn command_label(command: &str) -> String {
    const MAX_LEN: usize = 80;
    let trimmed = command.trim();
    if trimmed.chars().count() <= MAX_LEN {
        return trimmed.to_string();
    }

    let mut out = trimmed.chars().take(MAX_LEN).collect::<String>();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use command_service_api::CommandServiceFuture;
    use command_service_api::CommandSessionError;
    use command_service_api::CommandWaitRequest;
    use command_service_api::ExecCommandRunRequest;
    use command_service_api::UnifiedExecError;
    use command_service_api::WriteStdinOutput;
    use command_service_api::WriteStdinRequest;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use std::path::PathBuf;

    #[derive(Default)]
    struct StubCommandState {
        current_thread_id: Option<ThreadId>,
        commands: Vec<RunningCommandSnapshot>,
    }

    impl CommandServiceSessionState for StubCommandState {
        fn allocate_process_id<'a>(&'a self) -> CommandServiceFuture<'a, i32> {
            Box::pin(async { 1 })
        }

        fn release_process_id<'a>(&'a self, _process_id: i32) -> CommandServiceFuture<'a, ()> {
            Box::pin(async {})
        }

        fn has_running_process_for_thread<'a>(
            &'a self,
            thread_id: ThreadId,
        ) -> CommandServiceFuture<'a, bool> {
            Box::pin(async move { self.current_thread_id == Some(thread_id) })
        }

        fn running_processes_for_thread<'a>(
            &'a self,
            thread_id: ThreadId,
        ) -> CommandServiceFuture<'a, Vec<RunningCommandSnapshot>> {
            Box::pin(async move {
                if self.current_thread_id == Some(thread_id) {
                    self.commands.clone()
                } else {
                    Vec::new()
                }
            })
        }

        fn terminate_all_processes<'a>(&'a self) -> CommandServiceFuture<'a, ()> {
            Box::pin(async {})
        }

        fn run_exec_command<'a>(
            &'a self,
            _session: Arc<dyn ThreadSessionCapability>,
            _approval_session: Arc<dyn codex_approval_service_api::ApprovalSessionCapability>,
            _turn: Arc<dyn thread_service_api::ThreadRuntimeCapability>,
            _call_id: String,
            _request: ExecCommandRunRequest,
        ) -> CommandServiceFuture<
            'a,
            Result<command_service_api::ExecCommandRunOutput, UnifiedExecError>,
        > {
            Box::pin(async { unreachable!("list_commands does not run commands") })
        }

        fn begin_command_wait<'a>(
            &'a self,
            _request: CommandWaitRequest,
        ) -> CommandServiceFuture<
            'a,
            Result<Box<dyn command_service_api::CommandWaitOperation>, CommandSessionError>,
        > {
            Box::pin(async { unreachable!("list_commands does not wait") })
        }

        fn write_command_stdin<'a>(
            &'a self,
            _request: WriteStdinRequest<'a>,
        ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>> {
            Box::pin(async { unreachable!("list_commands does not write stdin") })
        }
    }

    #[tokio::test]
    async fn list_commands_for_thread_filters_to_current_thread_and_excludes_output_tail() {
        let current_thread_id = ThreadId::new();
        let other_thread_id = ThreadId::new();
        let state = StubCommandState {
            current_thread_id: Some(current_thread_id),
            commands: vec![RunningCommandSnapshot {
                process_id: 7,
                call_id: "call_abc".to_string(),
                command: "rtk sleep 100".to_string(),
                cwd: AbsolutePathBuf::try_from(PathBuf::from("/repo")).expect("absolute path"),
                tty: false,
                notify_on: CommandNotificationFilter::Output,
                latest_output_tail: Some("secret recent output".to_string()),
            }],
        };

        let current = list_commands_for_thread(&state, current_thread_id).await;
        let other = list_commands_for_thread(&state, other_thread_id).await;

        assert_eq!(current.commands.len(), 1);
        assert!(other.commands.is_empty());
        let serialized = serde_json::to_string(&current).expect("output serializes");
        assert!(serialized.contains("\"command_id\":7"));
        assert!(serialized.contains("\"command_text\":\"rtk sleep 100\""));
        assert!(!serialized.contains("latest_output_tail"));
        assert!(!serialized.contains("secret recent output"));
    }

    #[test]
    fn subscription_summary_serializes_all_active_subscription_kinds() {
        let subscriptions = ListSubscriptionsOutput {
            subscriptions: vec![
                ActiveSubscriptionSummary::from(PersistedSubscription::Fs {
                    subscription_id: "fs-1".to_string(),
                    path: "/tmp/out.log".to_string(),
                    recursive: true,
                    label: Some("logs".to_string()),
                }),
                ActiveSubscriptionSummary::from(PersistedSubscription::EventCommand {
                    subscription_id: "event-1".to_string(),
                    command: "rtk cargo test".to_string(),
                    cwd: Some("/repo".to_string()),
                    label: None,
                }),
                ActiveSubscriptionSummary::from(PersistedSubscription::Schedule {
                    subscription_id: "schedule-1".to_string(),
                    schedule: ScheduleSpec::EveryInterval { interval_ms: 1000 },
                    label: Some("tick".to_string()),
                    message: Some("check".to_string()),
                }),
                ActiveSubscriptionSummary::from(PersistedSubscription::ProcessExit {
                    subscription_id: "process-1".to_string(),
                    session_id: 9,
                    label: None,
                }),
            ],
        };

        let value = serde_json::to_value(subscriptions).expect("output serializes");

        assert_eq!(value["subscriptions"][0]["type"], "fs");
        assert_eq!(value["subscriptions"][1]["type"], "event_command");
        assert_eq!(value["subscriptions"][2]["type"], "schedule");
        assert_eq!(value["subscriptions"][3]["type"], "process_exit");
        for subscription in value["subscriptions"].as_array().expect("subscriptions") {
            assert_eq!(subscription["status"], "active");
        }
    }
}
