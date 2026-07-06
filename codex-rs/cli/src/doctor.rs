//! Implements the `codex doctor` diagnostic report.
//!
//! Doctor is intentionally read-mostly: checks inspect the current installation,
//! configuration, authentication, terminal, state paths, and bounded reachability
//! probes without attempting repair or starting long-lived services. Each check
//! returns a redacted, serializable row so the same data can back the human
//! summary and `--json` support report.
//!
//! A failing check should describe the problem and remediation, but it should not
//! mutate user state. That keeps the command safe to run before filing a support
//! issue or while diagnosing a broken local installation.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::future::Future;
use std::io::IsTerminal;
use std::io::Read;
use std::net::IpAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_config_types::CONFIG_TOML_FILE;
use codex_config_types::McpServerConfig;
use codex_config_types::McpServerTransportConfig;
use codex_features::FEATURES;
use codex_install_context::InstallContext;
use codex_install_context::StandalonePlatform;
use codex_login::AuthDotJson;
use codex_login::AuthManager;
use codex_login::CODEX_ACCESS_TOKEN_ENV_VAR;
use codex_login::CODEX_API_KEY_ENV_VAR;
use codex_login::OPENAI_API_KEY_ENV_VAR;
use codex_login::default_client::build_reqwest_client;
use codex_login::default_client::default_headers;
use codex_login::load_auth_dot_json;
use codex_terminal_detection::Multiplexer;
use codex_terminal_detection::TerminalInfo;
use codex_terminal_detection::TerminalName;
use codex_terminal_detection::terminal_info;
use codex_tui::Cli as TuiCli;
use codex_utils_cli::CliConfigOverrides;
use http::HeaderMap;
use http::HeaderValue;
use model_service::ResponsesWebsocketClient;
use model_service::create_model_provider;
use model_service_api::ApiError;
use model_service_api::is_azure_responses_provider;
use protocol::protocol::AskForApproval;
use serde::Serialize;
use supports_color::Stream;
use thread_service::config::Config;
use thread_service::config::ConfigOverrides;
use thread_service::config::find_codex_home;

mod background;
mod output;
mod progress;
mod runtime;
mod updates;

use background::background_server_check;
use output::HumanOutputOptions;
use output::redact_detail;
use output::render_human_report;
use progress::DoctorProgress;
use progress::doctor_progress;
use runtime::runtime_check;
use runtime::search_check;
use updates::updates_check;

const OPENAI_BETA_HEADER: &str = "OpenAI-Beta";
const RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE: &str = "responses_websockets=2026-02-06";
const WEBSOCKET_IMMEDIATE_CLOSE_GRACE: Duration = Duration::from_millis(250);
const SLOW_CHECK_PROGRESS_THRESHOLD: Duration = Duration::from_secs(2);
const SLOW_CHECK_PROGRESS_INTERVAL: Duration = Duration::from_secs(1);
const PROXY_ENV_VARS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];
const COLOR_ENV_VARS: &[&str] = &[
    "COLORTERM",
    "NO_COLOR",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "FORCE_COLOR",
    "COLORFGBG",
];
const TERMINAL_DIMENSION_ENV_VARS: &[&str] = &["COLUMNS", "LINES"];
const TERMINFO_ENV_VARS: &[&str] = &["TERMINFO", "TERMINFO_DIRS"];
const LOCALE_ENV_VARS: &[&str] = &["LC_ALL", "LC_CTYPE", "LANG"];
const REMOTE_TERMINAL_ENV_VARS: &[&str] = &[
    "SSH_TTY",
    "SSH_CONNECTION",
    "SSH_CLIENT",
    "MOSH_IP",
    "WSL_DISTRO_NAME",
    "WSL_INTEROP",
    "VSCODE_INJECTION",
    "VSCODE_IPC_HOOK_CLI",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "WT_SESSION",
];
const TMUX_OPTION_NAMES: &[&str] = &[
    "extended-keys",
    "xterm-keys",
    "allow-passthrough",
    "set-clipboard",
    "focus-events",
];
const NARROW_TERMINAL_COLUMNS: u16 = 80;
const NARROW_TERMINAL_ROWS: u16 = 24;

/// Options for building a local Codex diagnostic report.
///
/// The command always runs the full bounded diagnostic set. Human output includes
/// detailed diagnostics by default; --summary keeps the terminal output compact.

include!("doctor_parts/report_types.rs");
include!("doctor_parts/auth_and_terminal.rs");
include!("doctor_parts/provider_checks.rs");
include!("doctor_parts/remaining_checks.rs");
