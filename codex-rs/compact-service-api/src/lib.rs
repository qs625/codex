use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::models::ResponseItem;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactMemoryLayout {
    pub shared_memory_root: Option<AbsolutePathBuf>,
    pub worktree_memory_root: AbsolutePathBuf,
    pub write_policy: CompactWritePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactWritePolicy {
    LocalCurrentWorkOnly,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactMemoryBundle {
    pub user_preferences: Option<String>,
    pub project_understanding: Option<String>,
    pub current_work: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactFileNote {
    pub path: String,
    pub reason: String,
    pub conclusion: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revisit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactCurrentWork {
    pub goal: String,
    pub status: String,
    #[serde(default)]
    pub recent_progress: Vec<String>,
    #[serde(default)]
    pub files_read: Vec<CompactFileNote>,
    #[serde(default)]
    pub key_findings: Vec<String>,
    #[serde(default)]
    pub skip_files: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub next_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactModelOutput {
    pub current_work: CompactCurrentWork,
    #[serde(default)]
    pub shared_fact_candidates: Vec<String>,
    pub handoff_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedCompactMemory {
    pub bundle: CompactMemoryBundle,
    pub current_work_markdown: String,
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
pub struct CompactPromptSpec {
    pub prompt_text: String,
    pub output_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementHistoryInput {
    pub initial_context: Vec<ResponseItem>,
    pub memory_bundle: CompactMemoryBundle,
    pub recent_real_user_messages: Vec<String>,
    pub compact_marker_text: String,
}
