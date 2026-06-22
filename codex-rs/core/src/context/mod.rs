//! Context fragments injected into model input.

mod environment_context;

pub(crate) use codex_context_manager::ApprovedCommandPrefixSaved;
pub(crate) use codex_context_manager::AppsInstructions;
pub(crate) use codex_context_manager::AvailableAgentsInstructions;
pub(crate) use codex_context_manager::AvailablePluginsInstructions;
pub(crate) use codex_context_manager::AvailableSkillsInstructions;
pub(crate) use codex_context_manager::AvailableWorkflowsInstructions;
pub(crate) use codex_context_manager::CollaborationModeInstructions;
pub use codex_context_manager::ContextualUserFragment;
pub(crate) use codex_context_manager::GuardianFollowupReviewReminder;
pub(crate) use codex_context_manager::HookAdditionalContext;
pub(crate) use codex_context_manager::ImageGenerationInstructions;
pub(crate) use codex_context_manager::ModelSwitchInstructions;
pub(crate) use codex_context_manager::MultiagentContext;
pub(crate) use codex_context_manager::NetworkRuleSaved;
pub use codex_context_manager::PermissionsInstructions;
pub(crate) use codex_context_manager::PersonalitySpecInstructions;
pub(crate) use codex_context_manager::PluginInstructions;
pub(crate) use codex_context_manager::RealtimeEndInstructions;
pub(crate) use codex_context_manager::RealtimeStartInstructions;
pub(crate) use codex_context_manager::RealtimeStartWithInstructions;
pub(crate) use codex_context_manager::SkillInstructions;
pub(crate) use codex_context_manager::SubagentNotification;
pub(crate) use codex_context_manager::UserInstructions;
pub(crate) use codex_context_manager::UserShellCommand;
pub(crate) use codex_context_manager::parse_visible_hook_prompt_message;
pub(crate) use environment_context::EnvironmentContext;
