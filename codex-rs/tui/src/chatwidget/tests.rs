//! Exercises `ChatWidget` event handling and rendering invariants.
//!
//! These tests cover both app-server-native inputs and focused widget helpers. Many assertions are
//! snapshot-based so that layout regressions and status/header changes show up as stable,
//! reviewable diffs.

pub(super) use super::*;
pub(super) use crate::app_command::AppCommand as Op;
pub(super) use crate::app_event::AppEvent;
pub(super) use crate::app_event::ExitMode;
#[cfg(not(target_os = "linux"))]
pub(super) use crate::app_event::RealtimeAudioDeviceKind;
pub(super) use crate::app_event_sender::AppEventSender;
pub(super) use crate::approval_events::ApplyPatchApprovalRequestEvent;
pub(super) use crate::approval_events::ExecApprovalRequestEvent;
pub(super) use crate::bottom_pane::LocalImageAttachment;
pub(super) use crate::bottom_pane::MentionBinding;
pub(super) use crate::bottom_pane::QueuedInputAction;
pub(super) use crate::chatwidget::realtime::RealtimeConversationPhase;
pub(super) use crate::diff_model::FileChange;
pub(super) use crate::history_cell::UserHistoryCell;
pub(super) use crate::legacy_core::config::Config;
pub(super) use crate::legacy_core::config::ConfigBuilder;
pub(super) use crate::legacy_core::config::Constrained;
pub(super) use crate::legacy_core::config::ConstraintError;
pub(super) use crate::model_catalog::ModelCatalog;
pub(super) use crate::test_backend::VT100Backend;
pub(super) use crate::test_support::PathBufExt;
pub(super) use crate::test_support::test_path_buf;
pub(super) use crate::test_support::test_path_display;
pub(super) use crate::token_usage::TokenUsage;
pub(super) use crate::token_usage::TokenUsageInfo;
pub(super) use crate::tui::FrameRequester;
pub(super) use ::protocol::ThreadId;
pub(super) use ::protocol::account::PlanType;
pub(super) use ::protocol::approvals::GuardianAssessmentAction;
pub(super) use ::protocol::approvals::GuardianAssessmentDecisionSource;
pub(super) use ::protocol::approvals::GuardianAssessmentEvent;
pub(super) use ::protocol::approvals::GuardianAssessmentStatus;
pub(super) use ::protocol::approvals::GuardianCommandSource;
pub(super) use ::protocol::approvals::GuardianRiskLevel;
pub(super) use ::protocol::approvals::GuardianUserAuthorization;
pub(super) use ::protocol::config_types::ApprovalsReviewer;
pub(super) use ::protocol::config_types::CollaborationMode;
pub(super) use ::protocol::config_types::ModeKind;
pub(super) use ::protocol::config_types::Personality;
pub(super) use ::protocol::config_types::ServiceTier;
pub(super) use ::protocol::models::FileSystemPermissions;
pub(super) use ::protocol::models::MessagePhase;
pub(super) use ::protocol::models::NetworkPermissions;
pub(super) use ::protocol::models::PermissionProfile;
pub(super) use ::protocol::openai_models::ModelInfo;
pub(super) use ::protocol::openai_models::ModelPreset;
pub(super) use ::protocol::openai_models::ModelsResponse;
pub(super) use ::protocol::openai_models::ReasoningEffortPreset;
pub(super) use ::protocol::openai_models::default_input_modalities;
pub(super) use ::protocol::parse_command::ParsedCommand;
pub(super) use ::protocol::plan_tool::PlanItemArg;
pub(super) use ::protocol::plan_tool::StepStatus;
pub(super) use ::protocol::plan_tool::UpdatePlanArgs;
pub(super) use ::protocol::request_permissions::RequestPermissionProfile;
pub(super) use ::protocol::user_input::TextElement;
pub(super) use app_server_protocol::AddCreditsNudgeCreditType;
pub(super) use app_server_protocol::AddCreditsNudgeEmailStatus;
pub(super) use app_server_protocol::AdditionalFileSystemPermissions as AppServerAdditionalFileSystemPermissions;
pub(super) use app_server_protocol::AdditionalNetworkPermissions as AppServerAdditionalNetworkPermissions;
pub(super) use app_server_protocol::AdditionalPermissionProfile as AppServerAdditionalPermissionProfile;
pub(super) use app_server_protocol::AppSummary;
pub(super) use app_server_protocol::AutoReviewDecisionSource as AppServerGuardianApprovalReviewDecisionSource;
pub(super) use app_server_protocol::CodexErrorInfo;
pub(super) use app_server_protocol::CollabAgentState as AppServerCollabAgentState;
pub(super) use app_server_protocol::CollabAgentStatus as AppServerCollabAgentStatus;
pub(super) use app_server_protocol::CollabAgentTool as AppServerCollabAgentTool;
pub(super) use app_server_protocol::CollabAgentToolCallStatus as AppServerCollabAgentToolCallStatus;
pub(super) use app_server_protocol::CommandAction as AppServerCommandAction;
pub(super) use app_server_protocol::CommandExecutionRequestApprovalParams as AppServerCommandExecutionRequestApprovalParams;
pub(super) use app_server_protocol::CommandExecutionSource as ExecCommandSource;
pub(super) use app_server_protocol::CommandExecutionSource as AppServerCommandExecutionSource;
pub(super) use app_server_protocol::CommandExecutionStatus as AppServerCommandExecutionStatus;
pub(super) use app_server_protocol::ConfigWarningNotification;
pub(super) use app_server_protocol::CreditsSnapshot;
pub(super) use app_server_protocol::ErrorNotification;
pub(super) use app_server_protocol::ExecPolicyAmendment;
pub(super) use app_server_protocol::FileUpdateChange;
pub(super) use app_server_protocol::GuardianApprovalReview;
pub(super) use app_server_protocol::GuardianApprovalReviewAction as AppServerGuardianApprovalReviewAction;
pub(super) use app_server_protocol::GuardianApprovalReviewStatus;
pub(super) use app_server_protocol::GuardianCommandSource as AppServerGuardianCommandSource;
pub(super) use app_server_protocol::GuardianRiskLevel as AppServerGuardianRiskLevel;
pub(super) use app_server_protocol::GuardianUserAuthorization as AppServerGuardianUserAuthorization;
pub(super) use app_server_protocol::GuardianWarningNotification;
pub(super) use app_server_protocol::HookCompletedNotification as AppServerHookCompletedNotification;
pub(super) use app_server_protocol::HookEventName as AppServerHookEventName;
pub(super) use app_server_protocol::HookExecutionMode as AppServerHookExecutionMode;
pub(super) use app_server_protocol::HookHandlerType as AppServerHookHandlerType;
pub(super) use app_server_protocol::HookOutputEntry as AppServerHookOutputEntry;
pub(super) use app_server_protocol::HookOutputEntryKind as AppServerHookOutputEntryKind;
pub(super) use app_server_protocol::HookRunStatus as AppServerHookRunStatus;
pub(super) use app_server_protocol::HookRunSummary as AppServerHookRunSummary;
pub(super) use app_server_protocol::HookScope as AppServerHookScope;
pub(super) use app_server_protocol::HookStartedNotification as AppServerHookStartedNotification;
pub(super) use app_server_protocol::ItemCompletedNotification;
pub(super) use app_server_protocol::ItemGuardianApprovalReviewCompletedNotification;
pub(super) use app_server_protocol::ItemGuardianApprovalReviewStartedNotification;
pub(super) use app_server_protocol::ItemStartedNotification;
pub(super) use app_server_protocol::MarketplaceAddResponse;
pub(super) use app_server_protocol::MarketplaceInterface;
pub(super) use app_server_protocol::MarketplaceUpgradeErrorInfo;
pub(super) use app_server_protocol::MarketplaceUpgradeResponse;
pub(super) use app_server_protocol::McpServerStartupState;
pub(super) use app_server_protocol::McpServerStatusDetail;
pub(super) use app_server_protocol::McpServerStatusUpdatedNotification;
pub(super) use app_server_protocol::ModelVerification as AppServerModelVerification;
pub(super) use app_server_protocol::ModelVerificationNotification;
pub(super) use app_server_protocol::NonSteerableTurnKind;
pub(super) use app_server_protocol::PatchApplyStatus as AppServerPatchApplyStatus;
pub(super) use app_server_protocol::PatchChangeKind;
pub(super) use app_server_protocol::PermissionsRequestApprovalParams as AppServerPermissionsRequestApprovalParams;
pub(super) use app_server_protocol::PluginAuthPolicy;
pub(super) use app_server_protocol::PluginDetail;
pub(super) use app_server_protocol::PluginInstallPolicy;
pub(super) use app_server_protocol::PluginInterface;
pub(super) use app_server_protocol::PluginListResponse;
pub(super) use app_server_protocol::PluginMarketplaceEntry;
pub(super) use app_server_protocol::PluginReadResponse;
pub(super) use app_server_protocol::PluginSource;
pub(super) use app_server_protocol::PluginSummary;
pub(super) use app_server_protocol::RateLimitReachedType;
pub(super) use app_server_protocol::RateLimitSnapshot;
pub(super) use app_server_protocol::RateLimitWindow;
pub(super) use app_server_protocol::ReasoningSummaryTextDeltaNotification;
pub(super) use app_server_protocol::ReviewTarget;
pub(super) use app_server_protocol::ServerNotification;
pub(super) use app_server_protocol::SkillSummary;
pub(super) use app_server_protocol::ThreadClosedNotification;
pub(super) use app_server_protocol::ThreadItem as AppServerThreadItem;
pub(super) use app_server_protocol::ThreadRealtimeClosedNotification;
pub(super) use app_server_protocol::ThreadRealtimeErrorNotification;
pub(super) use app_server_protocol::ToolRequestUserInputOption;
pub(super) use app_server_protocol::ToolRequestUserInputParams;
pub(super) use app_server_protocol::ToolRequestUserInputQuestion;
pub(super) use app_server_protocol::Turn as AppServerTurn;
pub(super) use app_server_protocol::TurnCompletedNotification;
pub(super) use app_server_protocol::TurnError as AppServerTurnError;
pub(super) use app_server_protocol::TurnStartedNotification;
pub(super) use app_server_protocol::TurnStatus as AppServerTurnStatus;
pub(super) use app_server_protocol::UserInput;
pub(super) use app_server_protocol::UserInput as AppServerUserInput;
pub(super) use app_server_protocol::WarningNotification;
pub(super) use assert_matches::assert_matches;
pub(super) use config_service::AppRequirementToml;
pub(super) use config_service::AppsRequirementsToml;
pub(super) use config_service::ConfigRequirements;
pub(super) use config_service::ConfigRequirementsToml;
pub(super) use config_service::ConfigLayerStack;
pub(super) use codex_config_types::Notifications;
pub(super) use codex_config_types::RequirementSource;
#[cfg(target_os = "windows")]
pub(super) use codex_config_types::WindowsSandboxModeToml;
pub(super) use codex_features::FEATURES;
pub(super) use codex_features::Feature;
pub(super) use codex_git_info::CommitLogEntry;
pub(super) use codex_otel::SessionTelemetry;
pub(super) use codex_terminal_detection::Multiplexer;
pub(super) use codex_terminal_detection::TerminalInfo;
pub(super) use codex_terminal_detection::TerminalName;
pub(super) use codex_utils_absolute_path::AbsolutePathBuf;
pub(super) use codex_utils_approval_presets::builtin_approval_presets;
pub(super) use crossterm::event::KeyCode;
pub(super) use crossterm::event::KeyEvent;
pub(super) use crossterm::event::KeyModifiers;
pub(super) use insta::assert_snapshot;
pub(super) use metrics_api::RuntimeMetricsSummary;
pub(super) use plugin_service::OPENAI_CURATED_MARKETPLACE_NAME;
pub(super) use serde_json::json;
#[cfg(target_os = "windows")]
pub(super) use serial_test::serial;
pub(super) use skill_service::model::SkillMetadata;
pub(super) use std::collections::BTreeMap;
pub(super) use std::collections::HashMap;
pub(super) use std::path::PathBuf;
pub(super) use tempfile::NamedTempFile;
pub(super) use tempfile::tempdir;
pub(super) use tokio::sync::mpsc::error::TryRecvError;
pub(super) use tokio::sync::mpsc::unbounded_channel;
pub(super) use toml::Value as TomlValue;

pub(super) fn chatwidget_snapshot_dir() -> PathBuf {
    let snapshot_file = codex_utils_cargo_bin::find_resource!(
        "src/chatwidget/snapshots/codex_tui__chatwidget__tests__chatwidget_tall.snap"
    )
    .expect("snapshot file");
    snapshot_file
        .parent()
        .unwrap_or_else(|| panic!("snapshot file has no parent: {}", snapshot_file.display()))
        .to_path_buf()
}

macro_rules! assert_chatwidget_snapshot {
    ($name:expr, $value:expr $(,)?) => {{
        let mut settings = insta::Settings::clone_current();
        settings.set_prepend_module_to_snapshot(false);
        settings.set_snapshot_path(crate::chatwidget::tests::chatwidget_snapshot_dir());
        settings.bind(|| {
            insta::assert_snapshot!(format!("codex_tui__chatwidget__tests__{}", $name), $value);
        });
    }};
    ($name:expr, $value:expr, @$snapshot:literal $(,)?) => {{
        let mut settings = insta::Settings::clone_current();
        settings.set_prepend_module_to_snapshot(false);
        settings.set_snapshot_path(crate::chatwidget::tests::chatwidget_snapshot_dir());
        settings.bind(|| {
            insta::assert_snapshot!(
                format!("codex_tui__chatwidget__tests__{}", $name),
                &($value),
                @$snapshot
            );
        });
    }};
}

mod app_server;
mod approval_requests;
mod composer_submission;
mod exec_flow;
mod goal_menu;
mod goal_validation;
mod guardian;
mod helpers;
mod history_replay;
mod mcp_startup;
mod permissions;
mod plan_mode;
mod popups_and_settings;
mod review_mode;
mod side;
mod slash_commands;
mod status_and_layout;
mod status_command_tests;
mod status_surface_previews;
mod terminal_title;

pub(crate) use helpers::make_chatwidget_manual_with_sender;
pub(crate) use helpers::set_chatgpt_auth;
pub(crate) use helpers::set_fast_mode_test_catalog;
pub(super) use helpers::*;
