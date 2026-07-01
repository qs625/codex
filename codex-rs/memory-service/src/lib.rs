//! Codex memory service implementation.
//!
//! This crate owns memory read-path prompt injection, memory citations,
//! read-only memory MCP access, extension-facing memory tools, and the startup
//! write/consolidation pipeline.

pub mod mcp;

mod prompts;
#[path = "write/control.rs"]
mod control;
#[path = "write/extensions/mod.rs"]
mod extensions;
#[path = "write/guard.rs"]
mod guard;
#[path = "write/metrics.rs"]
mod metrics;
#[path = "write/phase1.rs"]
mod phase1;
#[path = "write/phase2.rs"]
mod phase2;
#[path = "write/prompts.rs"]
mod write_prompts;
#[path = "write/runtime.rs"]
mod runtime;
#[path = "write/start.rs"]
mod start;
#[path = "write/storage.rs"]
mod storage;
#[path = "write/workspace.rs"]
pub mod workspace;

pub use memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider;
pub use memory_service_api::MemoryReadFuture;
pub use memory_service_api::MemoryToolDeveloperInstructionsProvider;
pub use memory_service_api::SharedMemoryToolDeveloperInstructionsProvider;
pub use memory_service_api::citations;
pub use memory_service_api::memory_root;
pub use mcp::MEMORY_TOOLS_NAMESPACE;
pub use mcp::READ_TOOL_NAME;
pub use mcp::SEARCH_TOOL_NAME;
pub use mcp::LIST_TOOL_NAME;
pub use mcp::LocalMemoriesBackend;
pub use mcp::MemoriesBackend;
pub use mcp::MemoriesBackendError;
pub use mcp::MemoriesMcpServer;
pub use mcp::memory_extension_tools;
pub use mcp::memory_extension_tool_name;
pub use mcp::run_server;
pub use mcp::run_stdio_server;
pub use prompts::build_memory_tool_developer_instructions;
pub use prompts::FsMemoryToolDeveloperInstructionsProvider;
pub use control::clear_memory_roots_contents;
pub use extensions::prune_old_extension_resources;
pub use memory_service_api::MemoryConsolidationAgent;
pub use memory_service_api::MemoryRuntimeFuture;
pub use memory_service_api::MemoryStartupRuntime;
pub use memory_service_api::MemoryStartupSettings;
pub use memory_service_api::StageOnePromptRequest;
pub use memory_service_api::StageOneRequestContext;
pub use start::start_memories_startup_task;
pub use storage::rebuild_raw_memories_file_from_memories;
pub use storage::rollout_summary_file_stem;
pub use storage::sync_rollout_summaries_from_memories;
pub use write_prompts::build_consolidation_prompt;
pub use write_prompts::build_stage_one_input_message;

const MEMORY_TOOL_DEVELOPER_INSTRUCTIONS_SUMMARY_TOKEN_LIMIT: usize = 5_000;

mod artifacts {
    pub(super) const EXTENSIONS_SUBDIR: &str = "extensions";
    pub(super) const ROLLOUT_SUMMARIES_SUBDIR: &str = "rollout_summaries";
    pub(super) const RAW_MEMORIES_FILENAME: &str = "raw_memories.md";
}

mod extension_resources {
    pub(super) const FILENAME_TS_FORMAT: &str = "%Y-%m-%dT%H-%M-%S";
    pub(super) const RETENTION_DAYS: i64 = 7;
}

mod guard_limits {
    pub(super) const CODEX_LIMIT_ID: &str = "codex";
}

mod prompt_blocks {
    pub(super) const EXTENSIONS_FOLDER_STRUCTURE: &str = r#"
Memory extensions (under {{ memory_extensions_root }}/):

- <extension_name>/instructions.md
  - Source-specific guidance for interpreting additional memory signals. If an
    extension folder exists, you must read its instructions.md to determine how to use this memory
    source.

If the user has any memory extensions, you MUST read the instructions for each extension to
determine how to use the memory source. If the workspace diff shows deleted extension resource files,
remove stale memories derived only from those resources. If it has no extension folders, continue
with the standard memory inputs only.
"#;

    pub(super) const EXTENSIONS_PRIMARY_INPUTS: &str = r#"
Optional source-specific inputs:
Under `{{ memory_extensions_root }}/`:

- `<extension_name>/instructions.md`
  - If extension folders exist, read each instructions.md first and follow it when interpreting
    that extension's memory source.

If the workspace diff shows deleted memory extension resources, use that extension-specific deletion
signal to remove stale memories derived only from those resources.
"#;
}

mod stage_one {
    pub(super) const MODEL: &str = "gpt-5.4-mini";
    pub(super) const REASONING_EFFORT: codex_protocol::openai_models::ReasoningEffort =
        codex_protocol::openai_models::ReasoningEffort::Low;
    pub(super) const CONCURRENCY_LIMIT: usize = 8;
    pub(super) const JOB_LEASE_SECONDS: i64 = 3_600;
    pub(super) const JOB_RETRY_DELAY_SECONDS: i64 = 3_600;
    pub(super) const THREAD_SCAN_LIMIT: usize = 5_000;
    pub(super) const PRUNE_BATCH_SIZE: usize = 200;
    pub(super) const PROMPT: &str = include_str!("../templates/memories/stage_one_system.md");
    pub(super) const DEFAULT_ROLLOUT_TOKEN_LIMIT: usize = 150_000;
    pub(super) const CONTEXT_WINDOW_PERCENT: i64 = 70;
}

mod stage_two {
    pub(super) const MODEL: &str = "gpt-5.4";
    pub(super) const REASONING_EFFORT: codex_protocol::openai_models::ReasoningEffort =
        codex_protocol::openai_models::ReasoningEffort::Medium;
    pub(super) const JOB_LEASE_SECONDS: i64 = 3_600;
    pub(super) const JOB_RETRY_DELAY_SECONDS: i64 = 3_600;
    pub(super) const JOB_HEARTBEAT_SECONDS: u64 = 90;
}

mod workspace_diff {
    pub(super) const FILENAME: &str = "phase2_workspace_diff.md";
    pub(super) const MAX_BYTES: usize = 4 * 1024 * 1024;
}

pub fn rollout_summaries_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(artifacts::ROLLOUT_SUMMARIES_SUBDIR)
}

pub fn memory_extensions_root(root: &std::path::Path) -> std::path::PathBuf {
    root.join(artifacts::EXTENSIONS_SUBDIR)
}

pub fn raw_memories_file(root: &std::path::Path) -> std::path::PathBuf {
    root.join(artifacts::RAW_MEMORIES_FILENAME)
}

pub async fn ensure_layout(root: &std::path::Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(rollout_summaries_dir(root)).await
}
