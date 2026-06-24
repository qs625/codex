use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use codex_protocol::items::FileChangeItem;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::TurnDiffEvent;
use codex_tool_runtime_api::ToolEventHost;
use codex_tool_runtime_api::ToolPatchTrackerUpdate;
use codex_utils_output_truncation::TruncationPolicy;

pub(crate) use codex_tool_runtime::ToolEmitter;
pub(crate) use codex_tool_runtime::ToolEventFailure;
pub(crate) use codex_tool_runtime::ToolEventStage;

pub(crate) struct ToolEventCtx;

impl ToolEventCtx {
    pub(crate) fn new<'a>(
        session: &'a Session,
        turn: &'a TurnContext,
        call_id: &'a str,
        turn_diff_tracker: Option<&'a SharedTurnDiffTracker>,
    ) -> codex_tool_runtime::ToolEventCtx<'a, CoreToolEventHost<'a>> {
        codex_tool_runtime::ToolEventCtx::new(
            CoreToolEventHost {
                session,
                turn,
                turn_diff_tracker,
            },
            call_id,
        )
    }
}

pub struct CoreToolEventHost<'a> {
    session: &'a Session,
    turn: &'a TurnContext,
    turn_diff_tracker: Option<&'a SharedTurnDiffTracker>,
}

impl<'a> CoreToolEventHost<'a> {
    pub(crate) fn new(
        session: &'a Session,
        turn: &'a TurnContext,
        turn_diff_tracker: Option<&'a SharedTurnDiffTracker>,
    ) -> Self {
        Self {
            session,
            turn,
            turn_diff_tracker,
        }
    }
}

impl ToolEventHost for CoreToolEventHost<'_> {
    fn turn_id(&self) -> &str {
        &self.turn.sub_id
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        self.turn.truncation_policy
    }

    async fn send_exec_command_begin(&self, event: ExecCommandBeginEvent) {
        self.session
            .send_event(self.turn, EventMsg::ExecCommandBegin(event))
            .await;
    }

    async fn send_exec_command_end(&self, event: ExecCommandEndEvent) {
        self.session
            .send_event(self.turn, EventMsg::ExecCommandEnd(event))
            .await;
    }

    async fn emit_file_change_started(&self, item: FileChangeItem) {
        self.session
            .emit_turn_item_started(self.turn, &TurnItem::FileChange(item))
            .await;
    }

    async fn emit_file_change_completed(&self, item: FileChangeItem) {
        self.session
            .emit_turn_item_completed(self.turn, TurnItem::FileChange(item))
            .await;
    }

    async fn record_model_items_and_emit_display_events(&self, items: Vec<ResponseItem>) {
        self.session
            .record_model_items_and_emit_display_events(self.turn, &items)
            .await;
    }

    async fn update_patch_diff<'a>(&'a self, tracker_update: ToolPatchTrackerUpdate<'a>) {
        let Some(tracker) = self.turn_diff_tracker else {
            return;
        };
        let (should_emit_turn_diff, unified_diff) = {
            let mut guard = tracker.lock().await;
            let previous_diff = guard.get_unified_diff();
            let tracker_changed = match tracker_update {
                ToolPatchTrackerUpdate::Track(delta) => {
                    guard.track_delta(delta);
                    true
                }
                ToolPatchTrackerUpdate::Invalidate => {
                    guard.invalidate();
                    true
                }
                ToolPatchTrackerUpdate::None => false,
            };
            let unified_diff = guard.get_unified_diff();
            (
                tracker_changed && (previous_diff.is_some() || unified_diff.is_some()),
                unified_diff.unwrap_or_default(),
            )
        };
        if should_emit_turn_diff {
            self.session
                .send_event(
                    self.turn,
                    EventMsg::TurnDiff(TurnDiffEvent { unified_diff }),
                )
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::make_session_and_context_with_dynamic_tools_and_rx;
    use crate::tools::sandboxing::ToolError;
    use crate::turn_diff_tracker::TurnDiffTracker;
    use codex_file_system::LOCAL_FS;
    use codex_protocol::error::CodexErr;
    use codex_protocol::error::SandboxErr;
    use codex_protocol::exec_output::ExecToolCallOutput;
    use codex_protocol::protocol::PatchApplyStatus;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    async fn assert_failed_apply_patch_tracks_committed_delta(
        out: Result<ExecToolCallOutput, ToolError>,
        expected_status: PatchApplyStatus,
    ) {
        let (session, turn, rx_event) =
            make_session_and_context_with_dynamic_tools_and_rx(Vec::new()).await;
        let tracker = Arc::new(Mutex::new(TurnDiffTracker::new()));
        let dir = tempdir().expect("tempdir");
        let cwd = AbsolutePathBuf::from_absolute_path(dir.path()).expect("absolute cwd");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let delta = codex_apply_patch::apply_patch(
            "*** Begin Patch\n*** Add File: out/dest.txt\n+after\n*** End Patch",
            &cwd,
            &mut stdout,
            &mut stderr,
            LOCAL_FS.as_ref(),
            /*sandbox*/ None,
        )
        .await
        .expect("apply patch");

        ToolEmitter::apply_patch(HashMap::new(), /*auto_approved*/ false)
            .finish(
                ToolEventCtx::new(session.as_ref(), turn.as_ref(), "call-id", Some(&tracker)),
                out,
                Some(&delta),
            )
            .await
            .expect_err("failed patch");

        let completed = rx_event.recv().await.expect("item completed event");
        assert!(matches!(
            completed.msg,
            EventMsg::ItemCompleted(event)
                if matches!(
                    &event.item,
                    TurnItem::FileChange(FileChangeItem {
                        status: Some(status),
                        ..
                    }) if status == &expected_status
                )
        ));

        let unified_diff = loop {
            let event = tokio::time::timeout(Duration::from_secs(1), rx_event.recv())
                .await
                .expect("turn diff event")
                .expect("channel open");
            if let EventMsg::TurnDiff(TurnDiffEvent { unified_diff }) = event.msg {
                break unified_diff;
            }
        };
        assert!(unified_diff.contains("out/dest.txt"));
        assert!(unified_diff.contains("+after"));
    }

    #[tokio::test]
    async fn denied_apply_patch_tracks_committed_delta() {
        let output = ExecToolCallOutput {
            exit_code: 1,
            ..Default::default()
        };
        assert_failed_apply_patch_tracks_committed_delta(
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                output: Box::new(output),
                network_policy_decision: None,
            }))),
            PatchApplyStatus::Failed,
        )
        .await;
    }

    #[tokio::test]
    async fn rejected_apply_patch_tracks_committed_delta() {
        assert_failed_apply_patch_tracks_committed_delta(
            Err(ToolError::Rejected("rejected by user".to_string())),
            PatchApplyStatus::Declined,
        )
        .await;
    }
}
