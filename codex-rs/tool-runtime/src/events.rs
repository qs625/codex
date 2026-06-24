use crate::ToolError;
use codex_apply_patch::AppliedPatchDelta;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::items::FileChangeItem;
use codex_protocol::models::CommandExecutionNotificationKind;
use codex_protocol::models::ResponseItem;
use codex_protocol::parse_command::ParsedCommand;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::ExecCommandNotifyOn;
use codex_protocol::protocol::ExecCommandSource;
use codex_protocol::protocol::ExecCommandStatus;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::PatchApplyStatus;
use codex_shell_command::parse_command::parse_command;
use codex_tool_runtime_api::ToolEventHost;
use codex_tool_runtime_api::ToolPatchTrackerUpdate;
use codex_tool_types::FunctionCallError;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::format_exec_output_str;

pub struct ToolEventCtx<'a, Host> {
    pub host: Host,
    pub call_id: &'a str,
}

impl<'a, Host> ToolEventCtx<'a, Host> {
    pub fn new(host: Host, call_id: &'a str) -> Self {
        Self { host, call_id }
    }
}

pub enum ToolEventStage<'a> {
    Begin,
    Success {
        output: ExecToolCallOutput,
        applied_patch_delta: Option<&'a AppliedPatchDelta>,
    },
    Failure(ToolEventFailure<'a>),
}

pub enum ToolEventFailure<'a> {
    Output(ExecToolCallOutput),
    Message(String),
    Rejected {
        message: String,
        applied_patch_delta: Option<&'a AppliedPatchDelta>,
    },
}

fn tracker_update_for_known_delta(delta: &AppliedPatchDelta) -> ToolPatchTrackerUpdate<'_> {
    if delta.is_exact() && delta.is_empty() {
        ToolPatchTrackerUpdate::None
    } else {
        ToolPatchTrackerUpdate::Track(delta)
    }
}

pub async fn emit_exec_command_begin<Host>(
    ctx: ToolEventCtx<'_, Host>,
    command: &[String],
    cwd: &AbsolutePathBuf,
    parsed_cmd: &[ParsedCommand],
    source: ExecCommandSource,
    interaction_input: Option<String>,
    process_id: Option<&str>,
    initial_wait_ms: Option<u64>,
    notify_on: Option<ExecCommandNotifyOn>,
) where
    Host: ToolEventHost,
{
    ctx.host
        .send_exec_command_begin(ExecCommandBeginEvent {
            call_id: ctx.call_id.to_string(),
            process_id: process_id.map(str::to_owned),
            turn_id: ctx.host.turn_id().to_string(),
            started_at_ms: now_unix_timestamp_ms(),
            command: command.to_vec(),
            cwd: cwd.clone(),
            parsed_cmd: parsed_cmd.to_vec(),
            source,
            interaction_input,
            initial_wait_ms,
            notify_on,
        })
        .await;
}

fn now_unix_timestamp_ms() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
// Concrete, allocation-free emitter: avoid trait objects and boxed futures.
pub enum ToolEmitter {
    Shell {
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        source: ExecCommandSource,
        parsed_cmd: Vec<ParsedCommand>,
        freeform: bool,
    },
    ApplyPatch {
        changes: HashMap<PathBuf, FileChange>,
        auto_approved: bool,
    },
    UnifiedExec {
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        source: ExecCommandSource,
        parsed_cmd: Vec<ParsedCommand>,
        process_id: Option<String>,
        initial_wait_ms: Option<u64>,
        notify_on: Option<ExecCommandNotifyOn>,
    },
}

impl ToolEmitter {
    pub fn shell(
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        source: ExecCommandSource,
        freeform: bool,
    ) -> Self {
        let parsed_cmd = parse_command(&command);
        Self::Shell {
            command,
            cwd,
            source,
            parsed_cmd,
            freeform,
        }
    }

    pub fn apply_patch(changes: HashMap<PathBuf, FileChange>, auto_approved: bool) -> Self {
        Self::ApplyPatch {
            changes,
            auto_approved,
        }
    }

    pub fn unified_exec(
        command: &[String],
        cwd: AbsolutePathBuf,
        source: ExecCommandSource,
        process_id: Option<String>,
        initial_wait_ms: u64,
        notify_on: ExecCommandNotifyOn,
    ) -> Self {
        let parsed_cmd = parse_command(command);
        Self::UnifiedExec {
            command: command.to_vec(),
            cwd,
            source,
            parsed_cmd,
            process_id,
            initial_wait_ms: Some(initial_wait_ms),
            notify_on: Some(notify_on),
        }
    }

    pub async fn emit<Host>(&self, ctx: ToolEventCtx<'_, Host>, stage: ToolEventStage<'_>)
    where
        Host: ToolEventHost,
    {
        match (self, stage) {
            (
                Self::Shell {
                    command,
                    cwd,
                    source,
                    parsed_cmd,
                    ..
                },
                stage,
            ) => {
                emit_exec_stage(
                    ctx,
                    ExecCommandInput::new(
                        command, cwd, parsed_cmd, *source, /*interaction_input*/ None,
                        /*process_id*/ None, /*initial_wait_ms*/ None,
                        /*notify_on*/ None,
                    ),
                    stage,
                )
                .await;
            }

            (
                Self::ApplyPatch {
                    changes,
                    auto_approved,
                    ..
                },
                ToolEventStage::Begin,
            ) => {
                ctx.host
                    .emit_file_change_started(FileChangeItem {
                        id: ctx.call_id.to_string(),
                        changes: changes.clone(),
                        status: None,
                        auto_approved: Some(*auto_approved),
                        stdout: None,
                        stderr: None,
                    })
                    .await;
            }
            (
                Self::ApplyPatch { changes, .. },
                ToolEventStage::Success {
                    output,
                    applied_patch_delta,
                },
            ) => {
                let status = if output.exit_code == 0 {
                    PatchApplyStatus::Completed
                } else {
                    PatchApplyStatus::Failed
                };
                let tracker_update = applied_patch_delta
                    .map(tracker_update_for_known_delta)
                    .unwrap_or(ToolPatchTrackerUpdate::Invalidate);
                emit_patch_end(
                    ctx,
                    changes.clone(),
                    output.stdout.text.clone(),
                    output.stderr.text.clone(),
                    status,
                    tracker_update,
                )
                .await;
            }
            (
                Self::ApplyPatch { changes, .. },
                ToolEventStage::Failure(ToolEventFailure::Output(output)),
            ) => {
                emit_patch_end(
                    ctx,
                    changes.clone(),
                    output.stdout.text.clone(),
                    output.stderr.text.clone(),
                    if output.exit_code == 0 {
                        PatchApplyStatus::Completed
                    } else {
                        PatchApplyStatus::Failed
                    },
                    ToolPatchTrackerUpdate::Invalidate,
                )
                .await;
            }
            (
                Self::ApplyPatch { changes, .. },
                ToolEventStage::Failure(ToolEventFailure::Message(message)),
            ) => {
                emit_patch_end(
                    ctx,
                    changes.clone(),
                    String::new(),
                    (*message).to_string(),
                    PatchApplyStatus::Failed,
                    ToolPatchTrackerUpdate::None,
                )
                .await;
            }
            (
                Self::ApplyPatch { changes, .. },
                ToolEventStage::Failure(ToolEventFailure::Rejected {
                    message,
                    applied_patch_delta,
                }),
            ) => {
                emit_patch_end(
                    ctx,
                    changes.clone(),
                    String::new(),
                    (*message).to_string(),
                    PatchApplyStatus::Declined,
                    applied_patch_delta
                        .map(tracker_update_for_known_delta)
                        .unwrap_or(ToolPatchTrackerUpdate::None),
                )
                .await;
            }
            (
                Self::UnifiedExec {
                    command,
                    cwd,
                    source,
                    parsed_cmd,
                    process_id,
                    initial_wait_ms,
                    notify_on,
                },
                stage,
            ) => {
                emit_exec_stage(
                    ctx,
                    ExecCommandInput::new(
                        command,
                        cwd,
                        parsed_cmd,
                        *source,
                        /*interaction_input*/ None,
                        process_id.as_deref(),
                        *initial_wait_ms,
                        *notify_on,
                    ),
                    stage,
                )
                .await;
            }
        }
    }

    pub async fn begin<Host>(&self, ctx: ToolEventCtx<'_, Host>)
    where
        Host: ToolEventHost,
    {
        self.emit(ctx, ToolEventStage::Begin).await;
    }

    fn format_exec_output_for_model<Host>(
        &self,
        output: &ExecToolCallOutput,
        ctx: &ToolEventCtx<'_, Host>,
    ) -> String
    where
        Host: ToolEventHost,
    {
        match self {
            Self::Shell { freeform: true, .. } => {
                crate::format_exec_output_for_model_freeform(output, ctx.host.truncation_policy())
            }
            _ => {
                crate::format_exec_output_for_model_structured(output, ctx.host.truncation_policy())
            }
        }
    }

    pub async fn finish<Host>(
        &self,
        ctx: ToolEventCtx<'_, Host>,
        out: Result<ExecToolCallOutput, ToolError>,
        applied_patch_delta: Option<&AppliedPatchDelta>,
    ) -> Result<String, FunctionCallError>
    where
        Host: ToolEventHost,
    {
        let (event, result) = match out {
            Ok(output) => {
                let content = self.format_exec_output_for_model(&output, &ctx);
                let exit_code = output.exit_code;
                let event = ToolEventStage::Success {
                    output,
                    applied_patch_delta,
                };
                let result = if exit_code == 0 {
                    Ok(content)
                } else {
                    Err(FunctionCallError::RespondToModel(content))
                };
                (event, result)
            }
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Timeout { output }))) => {
                let response = self.format_exec_output_for_model(&output, &ctx);
                let event = ToolEventStage::Failure(ToolEventFailure::Output(*output));
                let result = Err(FunctionCallError::RespondToModel(response));
                (event, result)
            }
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied { output, .. }))) => {
                let response = self.format_exec_output_for_model(&output, &ctx);
                // apply_patch can be denied after it has already committed a
                // known prefix. Reuse the output-bearing path so the visible
                // item still fails while the turn diff consumes that prefix.
                let event = match (self, applied_patch_delta) {
                    (Self::ApplyPatch { .. }, Some(delta)) => ToolEventStage::Success {
                        output: *output,
                        applied_patch_delta: Some(delta),
                    },
                    _ => ToolEventStage::Failure(ToolEventFailure::Output(*output)),
                };
                let result = Err(FunctionCallError::RespondToModel(response));
                (event, result)
            }
            Err(ToolError::Codex(err)) => {
                let message = format!("execution error: {err:?}");
                let event = ToolEventStage::Failure(ToolEventFailure::Message(message.clone()));
                let result = Err(FunctionCallError::RespondToModel(message));
                (event, result)
            }
            Err(ToolError::Rejected(msg)) => {
                // Normalize common rejection messages for exec tools so tests and
                // users see a clear, consistent phrase.
                //
                // NOTE: ToolError::Rejected is currently used for both user-declined approvals
                // and some operational/runtime rejection paths (for example setup failures).
                // We intentionally map all of them through the "rejected" event path for now,
                // which means a subset of non-user failures may be reported as Declined.
                //
                // TODO: We should add a new ToolError variant for user-declined approvals.
                let normalized = if msg == "rejected by user" {
                    match self {
                        Self::Shell { .. } | Self::UnifiedExec { .. } => {
                            "exec command rejected by user".to_string()
                        }
                        Self::ApplyPatch { .. } => "patch rejected by user".to_string(),
                    }
                } else {
                    msg
                };
                let event = ToolEventStage::Failure(ToolEventFailure::Rejected {
                    message: normalized.clone(),
                    applied_patch_delta,
                });
                let result = Err(FunctionCallError::RespondToModel(normalized));
                (event, result)
            }
        };
        self.emit(ctx, event).await;
        result
    }
}

struct ExecCommandInput<'a> {
    command: &'a [String],
    cwd: &'a AbsolutePathBuf,
    parsed_cmd: &'a [ParsedCommand],
    source: ExecCommandSource,
    interaction_input: Option<&'a str>,
    process_id: Option<&'a str>,
    initial_wait_ms: Option<u64>,
    notify_on: Option<ExecCommandNotifyOn>,
}

impl<'a> ExecCommandInput<'a> {
    fn new(
        command: &'a [String],
        cwd: &'a AbsolutePathBuf,
        parsed_cmd: &'a [ParsedCommand],
        source: ExecCommandSource,
        interaction_input: Option<&'a str>,
        process_id: Option<&'a str>,
        initial_wait_ms: Option<u64>,
        notify_on: Option<ExecCommandNotifyOn>,
    ) -> Self {
        Self {
            command,
            cwd,
            parsed_cmd,
            source,
            interaction_input,
            process_id,
            initial_wait_ms,
            notify_on,
        }
    }
}

struct ExecCommandResult {
    stdout: String,
    stderr: String,
    aggregated_output: String,
    exit_code: i32,
    duration: Duration,
    formatted_output: String,
    status: ExecCommandStatus,
}

async fn emit_exec_stage<Host>(
    ctx: ToolEventCtx<'_, Host>,
    exec_input: ExecCommandInput<'_>,
    stage: ToolEventStage<'_>,
) where
    Host: ToolEventHost,
{
    match stage {
        ToolEventStage::Begin => {
            emit_exec_command_begin(
                ctx,
                exec_input.command,
                exec_input.cwd,
                exec_input.parsed_cmd,
                exec_input.source,
                exec_input.interaction_input.map(str::to_owned),
                exec_input.process_id,
                exec_input.initial_wait_ms,
                exec_input.notify_on,
            )
            .await;
        }
        ToolEventStage::Success { output, .. }
        | ToolEventStage::Failure(ToolEventFailure::Output(output)) => {
            let exec_result = ExecCommandResult {
                stdout: output.stdout.text.clone(),
                stderr: output.stderr.text.clone(),
                aggregated_output: output.aggregated_output.text.clone(),
                exit_code: output.exit_code,
                duration: output.duration,
                formatted_output: format_exec_output_str(&output, ctx.host.truncation_policy()),
                status: if output.exit_code == 0 {
                    ExecCommandStatus::Completed
                } else {
                    ExecCommandStatus::Failed
                },
            };
            emit_exec_end(ctx, exec_input, exec_result).await;
        }
        ToolEventStage::Failure(ToolEventFailure::Message(message)) => {
            let text = message.to_string();
            let exec_result = ExecCommandResult {
                stdout: String::new(),
                stderr: text.clone(),
                aggregated_output: text.clone(),
                exit_code: -1,
                duration: Duration::ZERO,
                formatted_output: text,
                status: ExecCommandStatus::Failed,
            };
            emit_exec_end(ctx, exec_input, exec_result).await;
        }
        ToolEventStage::Failure(ToolEventFailure::Rejected { message, .. }) => {
            let text = message.to_string();
            let exec_result = ExecCommandResult {
                stdout: String::new(),
                stderr: text.clone(),
                aggregated_output: text.clone(),
                exit_code: -1,
                duration: Duration::ZERO,
                formatted_output: text,
                status: ExecCommandStatus::Declined,
            };
            emit_exec_end(ctx, exec_input, exec_result).await;
        }
    }
}

async fn emit_exec_end<Host>(
    ctx: ToolEventCtx<'_, Host>,
    exec_input: ExecCommandInput<'_>,
    exec_result: ExecCommandResult,
) where
    Host: ToolEventHost,
{
    let process_id = exec_input.process_id.map(str::to_owned);
    let completed_at_ms = now_unix_timestamp_ms();
    let exit_code = exec_result.exit_code;
    let notification_output = exec_result.aggregated_output.clone();
    ctx.host
        .send_exec_command_end(ExecCommandEndEvent {
            call_id: ctx.call_id.to_string(),
            process_id: process_id.clone(),
            turn_id: ctx.host.turn_id().to_string(),
            completed_at_ms,
            command: exec_input.command.to_vec(),
            cwd: exec_input.cwd.clone(),
            parsed_cmd: exec_input.parsed_cmd.to_vec(),
            source: exec_input.source,
            interaction_input: exec_input.interaction_input.map(str::to_owned),
            initial_wait_ms: exec_input.initial_wait_ms,
            notify_on: exec_input.notify_on,
            stdout: exec_result.stdout,
            stderr: exec_result.stderr,
            aggregated_output: exec_result.aggregated_output,
            exit_code: exec_result.exit_code,
            duration: exec_result.duration,
            formatted_output: exec_result.formatted_output,
            status: exec_result.status,
        })
        .await;
    if process_id.is_some() {
        let item = ResponseItem::CommandExecutionNotification {
            id: Some(format!("{}:notification:exit", ctx.call_id)),
            command_item_id: ctx.call_id.to_string(),
            kind: CommandExecutionNotificationKind::Exit,
            message: "Command exit notification received.".to_string(),
            output: (!notification_output.is_empty()).then_some(notification_output),
            exit_code: Some(exit_code),
            created_at_ms: completed_at_ms,
        };
        ctx.host
            .record_model_items_and_emit_display_events(vec![item])
            .await;
    }
}

async fn emit_patch_end<Host>(
    ctx: ToolEventCtx<'_, Host>,
    changes: HashMap<PathBuf, FileChange>,
    stdout: String,
    stderr: String,
    status: PatchApplyStatus,
    tracker_update: ToolPatchTrackerUpdate<'_>,
) where
    Host: ToolEventHost,
{
    ctx.host
        .emit_file_change_completed(FileChangeItem {
            id: ctx.call_id.to_string(),
            changes,
            status: Some(status),
            auto_approved: None,
            stdout: Some(stdout),
            stderr: Some(stderr),
        })
        .await;

    ctx.host.update_patch_diff(tracker_update).await;
}
