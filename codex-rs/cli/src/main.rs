use app_server_daemon::BootstrapOptions as AppServerBootstrapOptions;
use app_server_daemon::LifecycleCommand as AppServerLifecycleCommand;
use app_server_daemon::RemoteControlMode as AppServerRemoteControlMode;
use clap::Args;
use clap::CommandFactory;
use clap::Parser;
use clap_complete::Shell;
use clap_complete::generate;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_chatgpt::apply_command::ApplyCommand;
use codex_chatgpt::apply_command::run_apply_command;
use codex_cli::LandlockCommand;
use codex_cli::SeatbeltCommand;
use codex_cli::WindowsCommand;
use codex_cli::read_access_token_from_stdin;
use codex_cli::read_api_key_from_stdin;
use codex_cli::run_login_status;
use codex_cli::run_login_with_access_token;
use codex_cli::run_login_with_api_key;
use codex_cli::run_login_with_chatgpt;
use codex_cli::run_login_with_device_code;
use codex_cli::run_logout;
use codex_cloud_tasks::Cli as CloudTasksCli;
use codex_exec::Cli as ExecCli;
use codex_exec::Command as ExecCommand;
use codex_exec::ReviewArgs;
use codex_responses_api_proxy::Args as ResponsesApiProxyArgs;
use codex_tui::AppExitInfo;
use codex_tui::Cli as TuiCli;
use codex_tui::ExitReason;
use codex_tui::UpdateAction;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_cli::CliConfigOverrides;
use codex_utils_cli::ProfileV2Name;
use codex_utils_cli::resume_command;
use owo_colors::OwoColorize;
use rollout_trace::REDUCED_STATE_FILE_NAME;
use rollout_trace::replay_bundle;
use state::StateRuntime;
use state::state_db_path;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use supports_color::Stream;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod app_cmd;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod desktop_app;
mod doctor;
mod execpolicy_cmd;
mod marketplace_cmd;
mod mcp_cmd;
mod plugin_cmd;
mod state_db_recovery;
#[cfg(not(windows))]
mod wsl_paths;

use crate::execpolicy_cmd::ExecPolicyCheckCommand;
use crate::mcp_cmd::McpCli;
use crate::plugin_cmd::PluginCli;
use crate::plugin_cmd::PluginSubcommand;
use doctor::DoctorCommand;
use state_db_recovery as local_state_db;

use codex_config_types::CONFIG_TOML_FILE;
use codex_exec_server::EnvironmentManager;
use codex_features::FEATURES;
use codex_features::Stage;
use codex_features::is_known_feature_key;
use codex_login::AuthManager;
use codex_terminal_detection::TerminalName;
use config_service::LoaderOverrides;
use config_service::LocalConfigLayerLoader;
use exec_server_api::ExecServerRuntimePaths;
use memory_service::clear_memory_roots_contents;
use model_service::ModelService;
use model_service::ModelServiceRuntimeDeps;
use model_service::bundled_models_response;
use model_service_api::ModelCatalogRefresh;
use model_service_api::ModelServiceApi;
use protocol::protocol::AskForApproval;
use protocol::user_input::UserInput;
use rollout::StateDbHandle;
use thread_service::ThreadAuthRuntimes;
use thread_service::config::Config;
use thread_service::config::ConfigBuilder;
use thread_service::config::ConfigOverrides;
use thread_service::config::ThreadStoreConfig;
use thread_service::config::edit::ConfigEditsBuilder;
use thread_service::config::find_codex_home;
use thread_service::config::resolve_profile_v2_config_path;

pub(crate) fn config_builder() -> ConfigBuilder {
    ConfigBuilder::default().config_layer_loader(Arc::new(LocalConfigLayerLoader::default()))
}

include!("main_parts/cli_definitions.rs");
include!("main_parts/config_and_validation.rs");
include!("main_parts/commands_and_entry.rs");
