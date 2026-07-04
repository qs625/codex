use std::sync::Arc;
use std::time::Duration;

use codex_utils_absolute_path::AbsolutePathBuf;
use exec_server_api::ExecEnvironment;
use protocol::models::AdditionalPermissionProfile;
use protocol::models::SandboxPermissions;
use serde::Deserialize;
use permissions_service_api::ExecApprovalRequirement;
use tool_config::ToolUserShellType;

use crate::CommandNotificationFilter;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExecCommandApprovalMode {
    #[default]
    ContinueInRuntime,
    AlreadyApproved,
}

#[derive(Debug, Deserialize)]
pub struct ExecCommandArgs {
    pub cmd: String,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub login: Option<bool>,
    #[serde(default = "default_tty")]
    pub tty: bool,
    #[serde(default = "default_exec_yield_time_ms")]
    pub yield_time_ms: u64,
    #[serde(default)]
    pub initial_wait_ms: Option<u64>,
    #[serde(default)]
    pub notify_on: CommandNotifyOnArg,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    #[serde(default)]
    pub sandbox_permissions: SandboxPermissions,
    #[serde(default)]
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    #[serde(default)]
    pub justification: Option<String>,
    #[serde(default)]
    pub prefix_rule: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandNotifyOnArg {
    Output,
    #[default]
    Exit,
}

impl From<CommandNotifyOnArg> for CommandNotificationFilter {
    fn from(value: CommandNotifyOnArg) -> Self {
        match value {
            CommandNotifyOnArg::Output => Self::Output,
            CommandNotifyOnArg::Exit => Self::Exit,
        }
    }
}

#[derive(Clone)]
pub struct ExecCommandRunRequest {
    pub command: Vec<String>,
    pub shell_type: ToolUserShellType,
    pub hook_command: String,
    pub process_id: i32,
    pub yield_time_ms: u64,
    pub max_output_tokens: Option<usize>,
    pub cwd: AbsolutePathBuf,
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub additional_permissions_preapproved: bool,
    pub justification: Option<String>,
    pub prefix_rule: Option<Vec<String>>,
    pub notify_on: CommandNotificationFilter,
    pub approval_mode: ExecCommandApprovalMode,
    pub exec_approval_requirement: ExecApprovalRequirement,
}

pub struct ExecCommandRunOutput {
    pub event_call_id: String,
    pub chunk_id: String,
    pub wall_time: Duration,
    pub raw_output: Vec<u8>,
    pub max_output_tokens: Option<usize>,
    pub process_id: Option<i32>,
    pub exit_code: Option<i32>,
    pub original_token_count: Option<usize>,
    pub hook_command: Option<String>,
}

fn default_exec_yield_time_ms() -> u64 {
    10_000
}

fn default_tty() -> bool {
    false
}
