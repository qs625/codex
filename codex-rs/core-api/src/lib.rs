//! Public facade for thread management APIs built on `codex-core`.

#![deny(private_bounds, private_interfaces, unreachable_pub)]

use std::sync::Arc;

pub use codex_analytics_api::AnalyticsEventsClient;
pub use codex_app_server_protocol::ServerNotification;
pub use codex_app_server_protocol::item_event_to_server_notification;
pub use codex_arg0::Arg0DispatchPaths;
pub use codex_arg0::arg0_dispatch_or_else;
pub use codex_code_mode_api::DisabledCodeModeRuntimeFactory;
pub use codex_config_types::AuthCredentialsStoreMode;
pub use codex_config_types::History;
pub use codex_config_types::MemoriesConfig;
pub use codex_config_types::ModelAvailabilityNuxConfig;
pub use codex_config_types::Notice;
pub use codex_config_types::OAuthCredentialsStoreMode;
pub use codex_config_types::OtelConfig;
pub use codex_config_types::SessionPickerViewMode;
pub use codex_config_types::ToolSuggestConfig;
pub use codex_config_types::TuiKeymap;
pub use codex_config_types::TuiNotificationSettings;
pub use codex_config_types::TuiPetAnchor;
pub use codex_config_types::UriBasedFileOpener;
pub use codex_core::CodexThread;
pub use codex_core::ForkSnapshot;
pub use codex_core::McpManager;
pub use codex_core::NewThread;
pub use codex_core::StartThreadOptions;
pub use codex_core::ThreadAuthRuntimes;
pub use codex_core::ThreadManager;
pub use codex_core::ThreadShutdownReport;
pub use codex_core::config::Config;
pub use codex_core::config::ConfigLayerStack;
pub use codex_core::config::Constrained;
pub use codex_core::config::GhostSnapshotConfig;
pub use codex_core::config::MultiAgentV2Config;
pub use codex_core::config::Permissions;
pub use codex_core::config::ProjectConfig;
pub use codex_core::config::RealtimeAudioConfig;
pub use codex_core::config::RealtimeConfig;
pub use codex_core::config::TerminalResizeReflowConfig;
pub use codex_core::config::ThreadStoreConfig;
pub use codex_core::config::find_codex_home;
pub use codex_core::resolve_installation_id;
pub use codex_core::skills::SkillsManager;
pub use codex_exec_server::EnvironmentManager;
pub use codex_exec_server::ExecServerRuntimePaths;
pub use codex_extension_api::empty_extension_registry;
pub use codex_features::Feature;
pub use codex_features::Features;
pub use codex_login::AuthManager;
pub use codex_login::default_client::set_default_originator;
pub use codex_login::model_provider_auth_manager;
pub use codex_model_provider::DefaultModelProviderFactory;
pub use codex_model_provider_info::OPENAI_PROVIDER_ID;
pub use codex_model_provider_info::built_in_model_providers;
pub use codex_models_manager_api::RefreshStrategy;
pub use codex_models_manager_api::SharedModelsManager;
pub use codex_protocol::ThreadId;
pub use codex_protocol::config_types::AltScreenMode;
pub use codex_protocol::config_types::ApprovalsReviewer;
pub use codex_protocol::config_types::CollaborationModeMask;
pub use codex_protocol::config_types::ShellEnvironmentPolicy;
pub use codex_protocol::config_types::WebSearchMode;
pub use codex_protocol::dynamic_tools::DynamicToolSpec;
pub use codex_protocol::error::Result as CodexResult;
pub use codex_protocol::models::PermissionProfile;
pub use codex_protocol::openai_models::ModelPreset;
pub use codex_protocol::protocol::AskForApproval;
pub use codex_protocol::protocol::EventMsg;
pub use codex_protocol::protocol::InitialHistory;
pub use codex_protocol::protocol::McpServerRefreshConfig;
pub use codex_protocol::protocol::Op;
pub use codex_protocol::protocol::SessionConfiguredEvent;
pub use codex_protocol::protocol::SessionSource;
pub use codex_protocol::protocol::TurnEnvironmentSelection;
pub use codex_protocol::protocol::W3cTraceContext;
pub use codex_protocol::user_input::UserInput;
pub use codex_rollout::StateDbHandle;
pub use codex_state_api::SharedStateDbRuntime as CoreStateDbHandle;
pub use codex_thread_store::DefaultLiveThreadFactory;
pub use codex_thread_store::ThreadStore;
pub use codex_utils_absolute_path::AbsolutePathBuf;

pub async fn init_state_db(config: &Config) -> Option<StateDbHandle> {
    codex_rollout::state_db::init(config).await
}

pub fn core_state_db_from_state_db(state_db: Option<StateDbHandle>) -> Option<CoreStateDbHandle> {
    state_db.map(|state_db| {
        let state_db: CoreStateDbHandle = state_db;
        state_db
    })
}

pub fn thread_store_from_config(
    config: &Config,
    state_db: Option<StateDbHandle>,
) -> Arc<dyn ThreadStore> {
    match &config.experimental_thread_store {
        ThreadStoreConfig::Local => Arc::new(codex_thread_store::LocalThreadStore::new(
            codex_thread_store::LocalThreadStoreConfig::from_config(config),
            state_db,
        )),
        ThreadStoreConfig::InMemory { id } => codex_thread_store::InMemoryThreadStore::for_id(id),
    }
}
