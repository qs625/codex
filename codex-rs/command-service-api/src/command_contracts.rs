use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codex_exec_server_api::ExecEnvironment;
use codex_file_system::ExecutorFileSystem;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
pub use codex_permissions_runtime::ExecApprovalRequirement;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_tool_config::ToolUserShellType;
use codex_utils_absolute_path::AbsolutePathBuf;

#[derive(Clone, Debug)]
pub struct RuntimeShellSnapshot {
    pub path: AbsolutePathBuf,
    pub cwd: AbsolutePathBuf,
}

#[derive(Clone, Debug)]
pub struct RuntimeShell {
    pub shell_type: ToolUserShellType,
    pub shell_path: PathBuf,
    pub shell_snapshot: Option<RuntimeShellSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkApprovalMode {
    Immediate,
    Deferred,
}

#[derive(Clone, Debug)]
pub struct NetworkApprovalSpec<Trigger> {
    pub network: Option<SharedNetworkProxyRuntime>,
    pub mode: NetworkApprovalMode,
    pub trigger: Trigger,
    pub command: String,
}

#[derive(serde::Serialize, Clone, Debug, Eq, PartialEq, Hash)]
pub struct HookToolName {
    name: String,
    matcher_aliases: Vec<String>,
}

impl HookToolName {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            matcher_aliases: Vec::new(),
        }
    }

    pub fn bash() -> Self {
        Self::new("Bash")
    }

    pub fn apply_patch() -> Self {
        Self {
            name: "apply_patch".to_string(),
            matcher_aliases: vec!["Write".to_string(), "Edit".to_string()],
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn matcher_aliases(&self) -> &[String] {
        &self.matcher_aliases
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequestPayload {
    pub tool_name: HookToolName,
    pub tool_input: serde_json::Value,
}

impl PermissionRequestPayload {
    pub fn bash(command: String, description: Option<String>) -> Self {
        let mut tool_input = serde_json::Map::new();
        tool_input.insert("command".to_string(), serde_json::Value::String(command));
        if let Some(description) = description {
            tool_input.insert(
                "description".to_string(),
                serde_json::Value::String(description),
            );
        }

        Self {
            tool_name: HookToolName::bash(),
            tool_input: serde_json::Value::Object(tool_input),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolPermissionGrants {
    pub session: Option<AdditionalPermissionProfile>,
    pub turn: Option<AdditionalPermissionProfile>,
}

/// Filesystem/environment boundary needed by apply-patch execution.
pub trait ApplyPatchEnvironment: Send + Sync {
    fn environment_id(&self) -> &str;

    fn filesystem(&self) -> Arc<dyn ExecutorFileSystem>;
}

pub struct ToolSandboxContext {
    pub turn_id: String,
    pub telemetry: SharedSessionTelemetry,
    pub file_system_sandbox_policy: FileSystemSandboxPolicy,
    pub network_sandbox_policy: NetworkSandboxPolicy,
    pub permission_profile: PermissionProfile,
    pub managed_network_active: bool,
    pub cwd: AbsolutePathBuf,
    pub codex_linux_sandbox_exe: Option<PathBuf>,
    pub use_legacy_landlock: bool,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
}

pub struct ResolvedApplyPatchEnvironment {
    pub cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ApplyPatchEnvironment>,
}

pub struct ResolvedExecCommandEnvironment {
    pub cwd: AbsolutePathBuf,
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub apply_patch_environment: Arc<dyn ApplyPatchEnvironment>,
}

#[derive(Clone, Debug)]
pub struct ResolvedExecCommand {
    pub command: Vec<String>,
    pub shell_type: ToolUserShellType,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExecCommandApprovalMode {
    #[default]
    ContinueInRuntime,
    AlreadyApproved,
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
    pub notify_on: codex_command_runtime::CommandNotificationFilter,
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

#[derive(serde::Serialize, Clone, Debug, Eq, PartialEq, Hash)]
pub struct UnifiedExecApprovalKey {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
}
