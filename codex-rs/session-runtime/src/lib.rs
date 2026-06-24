use std::path::PathBuf;

mod config_lock;
mod session_settings;
mod steer_input;
mod thread_skills;
mod turn_context_item;
mod turn_resolved_config;
mod user_turn_input;

use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::ActivePermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::TurnEnvironmentSelection;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;

pub use config_lock::ConfigLockBuildInput;
pub use config_lock::ConfigLockMultiAgentV2ResolvedConfig;
pub use config_lock::ConfigLockResolvedConfigFields;
pub use config_lock::ConfigLockSessionResolvedFields;
pub use config_lock::build_config_lockfile_toml;
pub use config_lock::config_lock_to_pretty_toml;
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
pub use thread_skills::initial_thread_skills;
pub use thread_skills::merge_thread_skills;
pub use turn_context_item::TurnContextItemBuildInput;
pub use turn_context_item::build_turn_context_item;
pub use turn_resolved_config::TurnResolvedConfigFactInput;
pub use turn_resolved_config::build_turn_resolved_config_fact;
pub use user_turn_input::UserTurnSubmission;
pub use user_turn_input::user_turn_submission_from_op;

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
