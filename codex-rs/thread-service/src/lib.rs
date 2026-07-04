//! Thread service implementation.

// Prevent accidental direct writes to stdout/stderr in library code. All
// user-visible output must go through the appropriate abstraction.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::CollaborationMode;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use protocol::config_types::WindowsSandboxLevel;
use protocol::models::ActivePermissionProfile;
use protocol::models::PermissionProfile;
use protocol::protocol::AskForApproval;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::TurnEnvironmentSelection;
use serde_json::Value;

mod apply_patch_environment;
mod apps;
mod arc_monitor;
mod client;
mod client_common;
mod config_lock;
mod realtime_context;
mod realtime_conversation;
pub mod session;
pub use session::session::Session as ThreadSession;
pub(crate) use session::turn_context::TurnContext;
pub use session::turn_context::TurnContext as ThreadTurnContext;
mod agent;
pub use ::thread_service_api::ActiveEventSubscriptionTracker;
mod attestation;
pub(crate) mod code_mode_turn_bridge;
mod codex_delegate;
pub mod config;
mod context;
mod context_usage;
mod environment_selection;
mod goal;
mod turn_runtime_capability;
pub use state_api::ExternalGoalPreviousStatus;
pub use state_api::ExternalGoalSet;
#[cfg(test)]
mod guardian;
mod hook_runtime;
mod installation_id;
mod mailbox;
pub(crate) mod mention_syntax;
mod original_image_detail;
pub(crate) mod utils;
pub use mention_syntax::PLUGIN_TEXT_MENTION_SIGIL;
pub use mention_syntax::TOOL_MENTION_SIGIL;
pub use utils::path_utils;
pub mod personality_migration;
#[doc(hidden)]
pub(crate) mod prompt_debug;
#[doc(hidden)]
pub use prompt_debug::build_prompt_input;
pub(crate) mod mentions {
    pub(crate) use crate::turn_plugin_mentions::collect_explicit_plugin_mentions;
    pub(crate) use skill_service_api::build_connector_slug_counts;
    pub(crate) use skill_service_api::build_skill_name_counts;
}
mod runtime_shell_detect;
mod session_prefix;
mod session_rollout_init_error;
mod session_settings;
mod session_startup_prewarm;
pub mod skills;
pub(crate) use skills::build_skill_service_input_from_config;
pub(crate) use skills::emit_thread_skills_update;
pub(crate) use skills::resolve_skill_dependencies_for_turn;
mod event_mapping;
pub mod review_format;
pub mod review_prompts;
mod stream_events_utils;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod thread;
mod thread_service_api;
mod thread_skills;
pub(crate) mod web_search;
pub use rollout_api::ForkSnapshot;
pub use thread::CodexThread;
pub use thread::CodexThreadTurnContextOverrides;
pub use thread::NewThread;
pub use thread::StartThreadOptions;
pub use thread::ThreadAuthRuntimes;
pub use thread::ThreadConfigSnapshot;
pub use thread::ThreadCreatedEvent;
pub use thread::ThreadRuntimeStatus;
pub use thread::ThreadService;
pub use thread::ThreadShutdownReport;
pub use web_search::web_search_action_detail;
pub use web_search::web_search_detail;
pub(crate) mod agents_md;
pub use agents_md::AgentsMdManager;
pub use agents_md::DEFAULT_AGENTS_MD_FILENAME;
pub use agents_md::LOCAL_AGENTS_MD_FILENAME;
mod rollout;
pub mod runtime_shell_model;
pub(crate) mod runtime_shell_snapshot;
pub mod spawn;
pub(crate) mod state_db_bridge;
pub(crate) use state_db_bridge::StateDbHandle;
mod session_capability;
mod state;
mod tasks;
mod tool_dispatch_trace;
mod tool_output_utils;
mod turn_context_item;
mod turn_metadata;
mod turn_plugin_injection;
mod turn_plugin_mentions;
mod turn_resolved_config;
mod turn_state;
mod turn_timing;
mod user_shell_command;
mod user_turn_input;
pub mod util;

pub use ::thread_service_api::PendingInputItem;
pub use attestation::AttestationContext;
pub use attestation::AttestationProvider;
pub use attestation::GenerateAttestationFuture;
pub use client::ModelClient;
pub use client::ModelClientSession;
pub use client_common::Prompt;
pub use client_common::REVIEW_PROMPT;
pub use client_common::ResponseStream;
pub use permissions_service::EmptyExecPolicyLoader;
pub use permissions_service::ExecPolicyLoadResult;
pub use permissions_service::ExecPolicyLoader;
pub use config_lock::ConfigLockBuildInput;
pub use config_lock::ConfigLockMultiAgentV2ResolvedConfig;
pub use config_lock::ConfigLockResolvedConfigFields;
pub use config_lock::ConfigLockSessionResolvedFields;
pub use config_lock::build_config_lockfile_toml;
pub use config_lock::config_lock_to_pretty_toml;
pub use event_mapping::parse_turn_item;
pub use installation_id::resolve_installation_id;
pub use mailbox::Mailbox;
pub use mailbox::MailboxDeliveryPhase;
pub use mailbox::MailboxReceiver;
#[doc(hidden)]
pub type SharedTurnDiffTracker =
    std::sync::Arc<tokio::sync::Mutex<::thread_service_api::TurnDiffTracker>>;
pub(crate) type ToolServiceApi = dyn tool_service_api::ToolServiceApi;
pub type ThreadToolServiceApi = dyn tool_service_api::ToolServiceApi;
pub use session_settings::SessionPermissionProfileUpdate;
pub use session_settings::SessionSettingsApplyCurrent;
pub use session_settings::SessionSettingsApplyPlan;
pub use session_settings::build_session_settings_apply_plan;
pub use session_settings::is_enterprise_default_service_tier_plan;
pub use session_settings::legacy_permission_profile_for_cwd;
pub use session_settings::legacy_permission_profile_needs_cwd_rebind;
pub use session_settings::model_provider_update_for_collaboration_mode;
pub use session_settings::normalize_service_tier_update;
pub use session_settings::permission_profile_preserving_deny_reads;
pub use session_settings::resolve_session_service_tier;
pub use session_settings::retarget_workspace_roots_for_cwd_update;
pub use steer_input::ActiveSteerTurn;
pub use steer_input::SteerInputError;
pub use steer_input::SteerableTaskKind;
pub use steer_input::ValidatedSteerInput;
pub use steer_input::validate_steer_input;
pub use task_kind::TaskKind;
pub use thread_skills::initial_thread_skills;
pub use thread_skills::merge_thread_skills;
pub use turn_context_item::TurnContextItemBuildInput;
pub use turn_context_item::build_turn_context_item;
pub use turn_metadata::build_turn_metadata_header;
pub use turn_resolved_config::TurnResolvedConfigFactInput;
pub use turn_resolved_config::build_turn_resolved_config_fact;
pub use turn_state::PendingRequestPermissions;
pub use turn_state::TurnState;
pub use user_turn_input::UserTurnSubmission;
pub use user_turn_input::user_turn_submission_from_op;
pub mod compact;
mod memory_usage;
mod steer_input;
mod task_kind;

/// Session configuration overrides supplied by thread resume, turn override,
/// or app-server settings update paths.
#[derive(Default, Clone)]
pub struct SessionSettingsUpdate {
    pub cwd: Option<PathBuf>,
    pub workspace_roots: Option<Vec<AbsolutePathBuf>>,
    pub profile_workspace_roots: Option<Vec<AbsolutePathBuf>>,
    pub approval_policy: Option<AskForApproval>,
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    pub sandbox_policy: Option<SandboxPolicy>,
    pub permission_profile: Option<PermissionProfile>,
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub windows_sandbox_level: Option<WindowsSandboxLevel>,
    pub model_provider: Option<String>,
    pub collaboration_mode: Option<CollaborationMode>,
    pub reasoning_summary: Option<ReasoningSummaryConfig>,
    pub service_tier: Option<Option<String>>,
    pub final_output_json_schema: Option<Option<Value>>,
    /// Turn-local environment override. `None` inherits the sticky thread
    /// environments stored on the session configuration; `Some([])` explicitly
    /// disables environments for this turn.
    pub environments: Option<Vec<TurnEnvironmentSelection>>,
    pub personality: Option<Personality>,
    pub app_server_client_name: Option<String>,
    pub app_server_client_version: Option<String>,
}
