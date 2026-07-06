use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::models::ResponseItem;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactMemoryBundle {
    pub snapshots: Vec<CompactMemorySnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactWindowSummary {
    pub recent_real_user_messages: Vec<String>,
    pub turns_since_last_compact: usize,
    pub recent_file_read_search_count: usize,
    pub recent_tool_output_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftCompactInputs {
    pub usage_ratio: f64,
    pub turns_since_last_compact: usize,
    pub recent_file_read_search_count: usize,
    pub recent_tool_output_bytes: usize,
    pub current_work_completeness: f64,
    pub cooldown_turns_satisfied: bool,
    pub cooldown_bytes_satisfied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftCompactDecision {
    pub should_compact: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactReplacementFile {
    pub path: AbsolutePathBuf,
    pub role: CompactMemoryRole,
    pub label: Option<String>,
    pub token_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactMemoryRole {
    CurrentWork,
    ProjectUnderstanding,
    UserPreferences,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactMemorySnapshot {
    pub role: CompactMemoryRole,
    pub label: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementHistoryInput {
    pub initial_context: Vec<ResponseItem>,
    pub memory_bundle: CompactMemoryBundle,
    pub recent_real_user_messages: Vec<String>,
    pub compact_marker_text: String,
}

impl CompactMemoryBundle {
    pub fn current_work_content(&self) -> Option<&str> {
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.role == CompactMemoryRole::CurrentWork)
            .map(|snapshot| snapshot.content.as_str())
    }
}
