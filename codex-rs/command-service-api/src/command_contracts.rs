use std::sync::Arc;
use std::time::Duration;

use codex_utils_absolute_path::AbsolutePathBuf;
use exec_server_api::ExecEnvironment;
use permissions_service_api::ExecApprovalRequirement;
use protocol::models::AdditionalPermissionProfile;
use protocol::models::SandboxPermissions;
use serde::Deserialize;
use tool_config::ToolUserShellType;

use crate::CommandNotificationFilter;
use crate::resolve_output_notification_interval_ms;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExecCommandApprovalMode {
    #[default]
    ContinueInRuntime,
    AlreadyApproved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecCommandTerminalSize {
    pub rows: u16,
    pub cols: u16,
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
    #[serde(
        default,
        deserialize_with = "deserialize_output_notification_interval_ms"
    )]
    pub output_notification_interval_ms: Option<u64>,
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
    pub terminal_size: Option<ExecCommandTerminalSize>,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub additional_permissions_preapproved: bool,
    pub justification: Option<String>,
    pub prefix_rule: Option<Vec<String>>,
    pub notify_on: CommandNotificationFilter,
    pub output_notification_interval: Duration,
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

fn deserialize_output_notification_interval_ms<'de, D>(
    deserializer: D,
) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let interval_ms = Option::<u64>::deserialize(deserializer)?;
    resolve_output_notification_interval_ms(interval_ms)
        .map(|_| interval_ms)
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_OUTPUT_NOTIFICATION_INTERVAL_MS;
    use crate::MAX_OUTPUT_NOTIFICATION_INTERVAL_MS;
    use crate::MIN_OUTPUT_NOTIFICATION_INTERVAL_MS;

    #[test]
    fn exec_command_args_accepts_valid_output_notification_interval() {
        let args: ExecCommandArgs = serde_json::from_str(
            r#"{"cmd":"printf ok","notify_on":"output","output_notification_interval_ms":1000}"#,
        )
        .expect("valid interval should parse");

        assert_eq!(args.output_notification_interval_ms, Some(1000));
        assert_eq!(
            resolve_output_notification_interval_ms(args.output_notification_interval_ms)
                .expect("valid interval should resolve"),
            1000
        );
    }

    #[test]
    fn exec_command_args_defaults_output_notification_interval() {
        let args: ExecCommandArgs =
            serde_json::from_str(r#"{"cmd":"printf ok"}"#).expect("args should parse");

        assert_eq!(args.output_notification_interval_ms, None);
        assert_eq!(
            resolve_output_notification_interval_ms(args.output_notification_interval_ms)
                .expect("default interval should resolve"),
            DEFAULT_OUTPUT_NOTIFICATION_INTERVAL_MS
        );
    }

    #[test]
    fn exec_command_args_rejects_invalid_output_notification_interval() {
        for interval_ms in [
            0,
            MIN_OUTPUT_NOTIFICATION_INTERVAL_MS - 1,
            MAX_OUTPUT_NOTIFICATION_INTERVAL_MS + 1,
        ] {
            let error = serde_json::from_str::<ExecCommandArgs>(&format!(
                r#"{{"cmd":"printf ok","output_notification_interval_ms":{interval_ms}}}"#
            ))
            .expect_err("invalid interval should fail to parse")
            .to_string();

            assert!(
                error.contains("output_notification_interval_ms must be between"),
                "unexpected error: {error}"
            );
        }
    }
}
