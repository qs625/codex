use codex_apply_patch::AppliedPatchDelta;
use codex_protocol::items::FileChangeItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_utils_output_truncation::TruncationPolicy;
use std::future::Future;

/// Host-side event sink used by tool runtime display emitters.
///
/// Implementations bridge tool-domain lifecycle facts to the host conversation
/// history, live display events, and patch diff tracker.
pub trait ToolEventHost {
    fn turn_id(&self) -> &str;
    fn truncation_policy(&self) -> TruncationPolicy;

    fn send_exec_command_begin(
        &self,
        event: ExecCommandBeginEvent,
    ) -> impl Future<Output = ()> + Send;

    fn send_exec_command_end(&self, event: ExecCommandEndEvent) -> impl Future<Output = ()> + Send;

    fn emit_file_change_started(&self, item: FileChangeItem) -> impl Future<Output = ()> + Send;

    fn emit_file_change_completed(&self, item: FileChangeItem) -> impl Future<Output = ()> + Send;

    fn record_model_items_and_emit_display_events(
        &self,
        items: Vec<ResponseItem>,
    ) -> impl Future<Output = ()> + Send;

    fn update_patch_diff<'a>(
        &'a self,
        tracker_update: ToolPatchTrackerUpdate<'a>,
    ) -> impl Future<Output = ()> + Send + 'a;
}

pub enum ToolPatchTrackerUpdate<'a> {
    Track(&'a AppliedPatchDelta),
    Invalidate,
    None,
}
