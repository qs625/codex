use crate::output::format_exec_output_for_model_structured;
use crate::support::ToolError;
use codex_apply_patch::AppliedPatchDelta;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::items::FileChangeItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::PatchApplyStatus;
use codex_tool_types::FunctionCallError;
use codex_utils_output_truncation::TruncationPolicy;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;

pub(crate) trait ToolEventHost {
    fn turn_id(&self) -> &str;
    fn truncation_policy(&self) -> TruncationPolicy;
    fn emit_file_change_started(&self, item: FileChangeItem) -> impl Future<Output = ()> + Send;
    fn emit_file_change_completed(&self, item: FileChangeItem) -> impl Future<Output = ()> + Send;
    fn record_model_items_and_emit_display_events(&self, items: Vec<ResponseItem>) -> impl Future<Output = ()> + Send;
    fn update_patch_diff<'a>(&'a self, tracker_update: ToolPatchTrackerUpdate<'a>) -> impl Future<Output = ()> + Send + 'a;
}

pub(crate) enum ToolPatchTrackerUpdate<'a> {
    Track(&'a AppliedPatchDelta),
    Invalidate,
    None,
}

pub(crate) struct ToolEventCtx<'a, Host> {
    pub(crate) host: Host,
    pub(crate) call_id: &'a str,
}

impl<'a, Host> ToolEventCtx<'a, Host> {
    pub(crate) fn new(host: Host, call_id: &'a str) -> Self {
        Self { host, call_id }
    }
}

pub(crate) enum ToolEmitter {
    ApplyPatch {
        changes: HashMap<PathBuf, FileChange>,
        auto_approved: bool,
    },
}

impl ToolEmitter {
    pub(crate) fn apply_patch(changes: HashMap<PathBuf, FileChange>, auto_approved: bool) -> Self {
        Self::ApplyPatch {
            changes,
            auto_approved,
        }
    }

    pub(crate) async fn begin<Host>(&self, ctx: ToolEventCtx<'_, Host>)
    where
        Host: ToolEventHost,
    {
        match self {
            Self::ApplyPatch {
                changes,
                auto_approved,
            } => {
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
        }
    }

    pub(crate) async fn finish<Host>(
        &self,
        ctx: ToolEventCtx<'_, Host>,
        out: Result<ExecToolCallOutput, ToolError>,
        applied_patch_delta: Option<&AppliedPatchDelta>,
    ) -> Result<String, FunctionCallError>
    where
        Host: ToolEventHost,
    {
        let (status, stdout, stderr, tracker_update, result) = match out {
            Ok(output) => {
                let content = format_exec_output_for_model_structured(&output, ctx.host.truncation_policy());
                (
                    if output.exit_code == 0 {
                        PatchApplyStatus::Completed
                    } else {
                        PatchApplyStatus::Failed
                    },
                    output.stdout.text,
                    output.stderr.text,
                    applied_patch_delta
                        .map(tracker_update_for_known_delta)
                        .unwrap_or(ToolPatchTrackerUpdate::Invalidate),
                    if output.exit_code == 0 {
                        Ok(content)
                    } else {
                        Err(FunctionCallError::RespondToModel(content))
                    },
                )
            }
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Timeout { output }))) => (
                PatchApplyStatus::Failed,
                output.stdout.text.clone(),
                output.stderr.text.clone(),
                ToolPatchTrackerUpdate::Invalidate,
                Err(FunctionCallError::RespondToModel(
                    format_exec_output_for_model_structured(&output, ctx.host.truncation_policy()),
                )),
            ),
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied { output, .. }))) => {
                let tracker_update = applied_patch_delta
                    .map(tracker_update_for_known_delta)
                    .unwrap_or(ToolPatchTrackerUpdate::Invalidate);
                (
                    PatchApplyStatus::Failed,
                    output.stdout.text.clone(),
                    output.stderr.text.clone(),
                    tracker_update,
                    Err(FunctionCallError::RespondToModel(
                        format_exec_output_for_model_structured(&output, ctx.host.truncation_policy()),
                    )),
                )
            }
            Err(ToolError::Codex(err)) => (
                PatchApplyStatus::Failed,
                String::new(),
                format!("execution error: {err:?}"),
                ToolPatchTrackerUpdate::None,
                Err(FunctionCallError::RespondToModel(format!("execution error: {err:?}"))),
            ),
            Err(ToolError::Rejected(message)) => (
                PatchApplyStatus::Declined,
                String::new(),
                if message == "rejected by user" {
                    "patch rejected by user".to_string()
                } else {
                    message.clone()
                },
                applied_patch_delta
                    .map(tracker_update_for_known_delta)
                    .unwrap_or(ToolPatchTrackerUpdate::None),
                Err(FunctionCallError::RespondToModel(if message == "rejected by user" {
                    "patch rejected by user".to_string()
                } else {
                    message
                })),
            ),
        };

        let Self::ApplyPatch { changes, .. } = self;
        ctx.host
            .emit_file_change_completed(FileChangeItem {
                id: ctx.call_id.to_string(),
                changes: changes.clone(),
                status: Some(status),
                auto_approved: None,
                stdout: Some(stdout),
                stderr: Some(stderr),
            })
            .await;
        ctx.host.update_patch_diff(tracker_update).await;
        result
    }
}

fn tracker_update_for_known_delta(delta: &AppliedPatchDelta) -> ToolPatchTrackerUpdate<'_> {
    if delta.is_exact() && delta.is_empty() {
        ToolPatchTrackerUpdate::None
    } else {
        ToolPatchTrackerUpdate::Track(delta)
    }
}
