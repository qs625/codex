//! Unified Exec: interactive process execution orchestrated with approvals + sandboxing.
//!
//! Responsibilities
//! - Manages interactive processes (create, reuse, buffer output with caps).
//! - 在 command-service 内直接处理 approval、sandbox 选择和 retry 逻辑。
//! - Spawns the PTY from a sandbox-transformed `ExecRequest`; on sandbox denial,
//!   retries without sandbox when policy allows (no re‑prompt thanks to caching).
//! - Uses the shared `is_likely_sandbox_denied` heuristic to keep denial messages
//!   consistent with other exec paths.
//!
//! Flow at a glance (open process)
//! 1) Build a small request `{ command, cwd }`.
//! 2) Orchestrator: approval (bypass/cache/prompt) → select sandbox → run.
//! 3) Runtime: transform `SandboxTransformRequest` -> `ExecRequest` -> spawn PTY.
//! 4) If denial, orchestrator retries with `SandboxType::None`.
//! 5) Process handle is returned with streaming output + metadata.
//!
//! This keeps policy logic and user interaction centralized while the PTY/process
//! concerns remain isolated here. The implementation is split between:
//! - `process.rs`: PTY process lifecycle + output buffering.
//! - `process_manager.rs`: orchestration (approvals, sandboxing, reuse) and request handling.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;

pub(crate) use codex_command_service_api::CommandNotificationFilter;
pub(crate) use codex_command_service_api::CommandNotificationKind;
pub(crate) use codex_command_service_api::CommandWaitOutput;
pub(crate) use codex_command_service_api::CommandWaitRequest;
pub(crate) use codex_command_service_api::CommandWaitStatus;
pub(crate) use codex_command_service_api::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
#[cfg(test)]
pub(crate) use codex_command_service_api::MIN_YIELD_TIME_MS;
pub(crate) use codex_command_service_api::WaitBackoffState;
pub(crate) use codex_command_service_api::WriteStdinOutput;
pub(crate) use codex_command_service_api::WriteStdinRequest;
pub(crate) use codex_command_service_api::clamp_yield_time;
pub(crate) use codex_command_service_api::generate_chunk_id;
use codex_exec_server_api::ExecEnvironment;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_tool_config::ToolUserShellType;
use codex_utils_absolute_path::AbsolutePathBuf;
use tokio::sync::Mutex;

use crate::exec_request::SandboxPermissions;
mod async_watcher;
mod exec_server_env;
mod events;
mod output;
mod process_manager;
mod runtime_types;
mod unified_exec_process;

pub(crate) use output::HeadTailBuffer;
pub(crate) use runtime_types::CommandNotificationSnapshot;
pub(crate) use runtime_types::CommandNotificationState;
pub(crate) use runtime_types::CommandProcessIdAllocator;
pub(crate) use runtime_types::CommandProcessPruneMeta;
pub(crate) use runtime_types::ProcessState;
pub(crate) use runtime_types::command_process_id_to_prune;

pub(crate) fn set_deterministic_process_ids_for_tests(enabled: bool) {
    process_manager::set_deterministic_process_ids_for_tests(enabled);
}

pub(crate) use exec_server_env::ExecServerEnvConfig;
pub(crate) use exec_server_env::ExecServerSpawnRequest;
pub(crate) use exec_server_env::apply_unified_exec_env;
pub(crate) use exec_server_env::exec_env_policy_from_shell_policy;
pub(crate) use exec_server_env::exec_server_spawn_params;
pub(crate) use codex_command_service_api::ExecApprovalRequirement;
pub(crate) use codex_command_service_api::ExecCommandApprovalMode;
pub(crate) use codex_command_service_api::ExecCommandRunRequest;
pub(crate) use output::collect_output_until_deadline;
pub(crate) use output::resolve_aggregated_output;
pub(crate) use output::split_valid_utf8_prefix;
pub(crate) use unified_exec_process::NoopSpawnLifecycle;
pub(crate) use unified_exec_process::SpawnLifecycleHandle;
pub(crate) use unified_exec_process::UnifiedExecProcess;
pub(crate) use codex_command_service_api::UnifiedExecError;
pub(crate) use process_manager::UnifiedExecCommandSessionController;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ToolRuntimeNetworkApprovalHandle;

pub(crate) const MAX_UNIFIED_EXEC_PROCESSES: usize = 64;
pub(crate) const DEFAULT_COMMAND_OUTPUT_MAX_TOKENS: usize =
    codex_command_service_api::DEFAULT_COMMAND_OUTPUT_MAX_BYTES / 4;

pub(crate) struct UnifiedExecContext {
    pub session: Arc<dyn ThreadSessionCapability>,
    pub turn: Arc<dyn ThreadRuntimeCapability>,
    pub call_id: String,
}

impl UnifiedExecContext {
    pub fn new(
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
    ) -> Self {
        Self {
            session,
            turn,
            call_id,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ExecCommandRequest {
    pub command: Vec<String>,
    pub shell_type: ToolUserShellType,
    pub hook_command: String,
    pub process_id: i32,
    pub yield_time_ms: u64,
    pub max_output_tokens: Option<usize>,
    pub cwd: AbsolutePathBuf,
    #[allow(dead_code)]
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub network: Option<SharedNetworkProxyRuntime>,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    #[allow(dead_code)]
    pub additional_permissions_preapproved: bool,
    pub justification: Option<String>,
    #[allow(dead_code)]
    pub prefix_rule: Option<Vec<String>>,
    pub notify_on: CommandNotificationFilter,
    pub approval_mode: ExecCommandApprovalMode,
    pub exec_approval_requirement: ExecApprovalRequirement,
}

impl ExecCommandRequest {
    pub(crate) fn from_run_request(
        request: ExecCommandRunRequest,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Self {
        Self {
            command: request.command,
            shell_type: request.shell_type,
            hook_command: request.hook_command,
            process_id: request.process_id,
            yield_time_ms: request.yield_time_ms,
            max_output_tokens: request.max_output_tokens,
            cwd: request.cwd,
            sandbox_cwd: request.sandbox_cwd,
            environment: request.environment,
            network,
            tty: request.tty,
            sandbox_permissions: request.sandbox_permissions,
            additional_permissions: request.additional_permissions,
            additional_permissions_preapproved: request.additional_permissions_preapproved,
            justification: request.justification,
            prefix_rule: request.prefix_rule,
            notify_on: request.notify_on,
            approval_mode: match request.approval_mode {
                codex_command_service_api::ExecCommandApprovalMode::ContinueInRuntime => {
                    ExecCommandApprovalMode::ContinueInRuntime
                }
                codex_command_service_api::ExecCommandApprovalMode::AlreadyApproved => {
                    ExecCommandApprovalMode::AlreadyApproved
                }
            },
            exec_approval_requirement: request.exec_approval_requirement,
        }
    }
}

pub(crate) fn command_notification_filter_to_protocol(
    value: CommandNotificationFilter,
) -> codex_protocol::protocol::ExecCommandNotifyOn {
    match value {
        CommandNotificationFilter::Output => codex_protocol::protocol::ExecCommandNotifyOn::Output,
        CommandNotificationFilter::Exit => codex_protocol::protocol::ExecCommandNotifyOn::Exit,
    }
}

#[derive(Default)]
pub(crate) struct ProcessStore {
    processes: HashMap<i32, ProcessEntry>,
    process_ids: CommandProcessIdAllocator,
}

impl ProcessStore {
    fn remove(&mut self, process_id: i32) -> Option<ProcessEntry> {
        self.process_ids.release_reservation(process_id);
        let entry = self.processes.remove(&process_id)?;
        if entry.process.has_exited() || entry.process.exit_code().is_some() {
            self.process_ids
                .mark_completed(process_id, entry.process.exit_code());
        }
        Some(entry)
    }
}

pub(crate) struct UnifiedExecProcessManager {
    process_store: Mutex<ProcessStore>,
    command_wait_hard_cap: std::time::Duration,
}

#[derive(Clone)]
pub struct UnifiedExecManagerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    manager: Weak<UnifiedExecProcessManager>,
}

impl UnifiedExecManagerHandle {
    pub(crate) fn new(manager: Weak<UnifiedExecProcessManager>) -> Self {
        Self { manager }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn upgrade(&self) -> Option<Arc<UnifiedExecProcessManager>> {
        self.manager.upgrade()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ProcessExitSubscription {
    process: Arc<UnifiedExecProcess>,
    cancellation_token: tokio_util::sync::CancellationToken,
    transcript: Arc<Mutex<HeadTailBuffer>>,
}

impl ProcessExitSubscription {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn wait(&self) -> Option<i32> {
        self.cancellation_token.cancelled().await;
        self.process.exit_code()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn wait_with_retained_output(&self) -> (Option<i32>, String) {
        self.cancellation_token.cancelled().await;
        let output = {
            let guard = self.transcript.lock().await;
            String::from_utf8_lossy(&guard.to_bytes()).to_string()
        };
        (self.process.exit_code(), output)
    }
}

impl UnifiedExecProcessManager {
    pub(crate) fn new(max_write_stdin_yield_time_ms: u64) -> Self {
        Self {
            process_store: Mutex::new(ProcessStore::default()),
            command_wait_hard_cap: std::time::Duration::from_millis(max_write_stdin_yield_time_ms),
        }
    }
}

impl Default for UnifiedExecProcessManager {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS)
    }
}

struct ProcessEntry {
    process: Arc<UnifiedExecProcess>,
    call_id: String,
    process_id: i32,
    tty: bool,
    network_approval: Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>,
    session: Weak<dyn ThreadSessionCapability>,
    last_used: tokio::time::Instant,
    #[allow(dead_code)]
    transcript: Arc<Mutex<HeadTailBuffer>>,
    notification_state: Arc<CommandNotificationState>,
    command_wait_backoff: WaitBackoffState,
}

#[cfg(test)]
#[cfg(unix)]
#[path = "mod_tests.rs"]
mod tests;
