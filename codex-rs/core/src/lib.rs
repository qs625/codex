//! Root of the `codex-core` library.

// Prevent accidental direct writes to stdout/stderr in library code. All
// user-visible output must go through the appropriate abstraction (e.g.,
// the TUI or the tracing stack).
#![deny(clippy::print_stdout, clippy::print_stderr)]

mod active_event_subscriptions;
mod apply_patch;
mod apps;
mod arc_monitor;
mod client;
mod client_common;
mod realtime_context;
mod realtime_conversation;
pub(crate) mod session;
pub use session::SteerInputError;
mod codex_thread;
mod compact_remote;
mod compact_remote_v2;
pub use codex_thread::CodexThread;
pub use codex_thread::CodexThreadTurnContextOverrides;
pub use codex_thread::ThreadConfigSnapshot;
pub use codex_thread::ThreadRuntimeStatus;
pub use session::turn_context::TurnContext;
mod agent;
pub use active_event_subscriptions::ActiveEventSubscriptionTracker;
mod attestation;
mod codex_delegate;
pub mod config;
pub mod connectors;
pub mod context;
mod context_manager;
mod context_usage;
mod environment_selection;
pub mod exec;
pub mod exec_env;
mod exec_policy;
#[cfg(test)]
mod git_info_tests;
mod goals;
pub use codex_state_api::ExternalGoalPreviousStatus;
pub use codex_state_api::ExternalGoalSet;
mod guardian;
mod hook_runtime;
mod installation_id;
pub(crate) mod mcp;
mod mcp_skill_dependencies;
mod mcp_tool_exposure;
mod network_policy_decision;
pub use codex_mcp_runtime::McpManager;
mod mcp_openai_file;
mod mcp_tool_call;
pub(crate) mod mention_syntax;
mod original_image_detail;
pub(crate) mod utils;
pub use mention_syntax::PLUGIN_TEXT_MENTION_SIGIL;
pub use mention_syntax::TOOL_MENTION_SIGIL;
pub use utils::path_utils;
mod pending_input;
pub mod personality_migration;
pub(crate) mod plugins;
#[doc(hidden)]
pub(crate) mod prompt_debug;
#[doc(hidden)]
pub use prompt_debug::build_prompt_input;
pub(crate) mod mentions {
    pub(crate) use crate::plugins::build_connector_slug_counts;
    pub(crate) use crate::plugins::build_skill_name_counts;
    pub(crate) use crate::plugins::collect_explicit_app_ids;
    pub(crate) use crate::plugins::collect_explicit_plugin_mentions;
    pub(crate) use crate::plugins::collect_tool_mentions_from_messages;
}
pub mod sandboxing;
mod session_prefix;
mod session_startup_prewarm;
mod shell_detect;
pub mod skills;
pub(crate) use skills::SkillInjections;
pub(crate) use skills::SkillLoadOutcome;
pub(crate) use skills::SkillMetadata;
pub(crate) use skills::build_available_skills;
pub(crate) use skills::build_skill_injections;
pub(crate) use skills::build_skill_name_counts;
pub(crate) use skills::collect_env_var_dependencies;
pub(crate) use skills::collect_explicit_skill_mentions;
pub(crate) use skills::default_skill_metadata_budget;
pub(crate) use skills::emit_thread_skills_update;
pub(crate) use skills::injection;
pub(crate) use skills::maybe_emit_implicit_skill_invocation;
pub(crate) use skills::resolve_skill_dependencies_for_turn;
pub(crate) use skills::skills_load_input_from_config;
mod event_mapping;
pub mod review_format;
pub mod review_prompts;
mod stream_events_utils;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod thread_manager;
mod unified_exec;
pub(crate) mod web_search;
pub mod windows_sandbox;
pub(crate) mod windows_sandbox_read_grants;
pub mod workflow_runs;
pub mod workflows;
pub use codex_rollout_api::ForkSnapshot;
pub use thread_manager::NewThread;
pub use thread_manager::StartThreadOptions;
pub use thread_manager::ThreadAuthRuntimes;
pub use thread_manager::ThreadCreatedEvent;
pub use thread_manager::ThreadManager;
pub use thread_manager::ThreadShutdownReport;
pub use thread_manager::build_models_manager;
pub use unified_exec::ProcessExitSubscription;
pub use unified_exec::UnifiedExecManagerHandle;
pub use unified_exec::UnifiedExecProcessManager;
pub use web_search::web_search_action_detail;
pub use web_search::web_search_detail;
pub use windows_sandbox_read_grants::grant_read_root_non_elevated;
#[deprecated(note = "use ThreadManager")]
pub type ConversationManager = ThreadManager;
#[deprecated(note = "use NewThread")]
pub type NewConversation = NewThread;
#[deprecated(note = "use CodexThread")]
pub type CodexConversation = CodexThread;
pub(crate) mod agents_md;
pub use agents_md::AgentsMdManager;
pub use agents_md::DEFAULT_AGENTS_MD_FILENAME;
pub use agents_md::LOCAL_AGENTS_MD_FILENAME;
mod rollout;
pub(crate) mod safety;
mod session_rollout_init_error;
pub mod shell;
pub(crate) mod shell_snapshot;
pub mod spawn;
pub(crate) mod state_db_bridge;
pub(crate) use state_db_bridge::StateDbHandle;
mod function_tool;
mod state;
mod tasks;
mod tools;
pub(crate) mod turn_diff_tracker;
mod turn_metadata;
mod turn_timing;
mod user_shell_command;
pub mod util;

pub use attestation::AttestationContext;
pub use attestation::AttestationProvider;
pub use attestation::GenerateAttestationFuture;
pub use client::ModelClient;
pub use client::ModelClientSession;
pub use client_common::Prompt;
pub use client_common::REVIEW_PROMPT;
pub use client_common::ResponseStream;
pub use compact::content_items_to_text;
pub use event_mapping::parse_turn_item;
pub use exec_policy::EmptyExecPolicyLoader;
pub use exec_policy::ExecPolicyLoadResult;
pub use exec_policy::ExecPolicyLoader;
pub use installation_id::resolve_installation_id;
pub use turn_metadata::build_turn_metadata_header;
pub mod compact;
mod memory_usage;
