//! Unified Exec: interactive process execution orchestrated with approvals + sandboxing.
//!
//! Responsibilities
//! - Manages interactive processes (create, reuse, buffer output with caps).
//! - Uses the shared ToolOrchestrator to handle approval, sandbox selection, and
//!   retry semantics in a single, descriptive flow.
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

pub(crate) use codex_command_runtime::CommandNotificationFilter;
pub(crate) use codex_command_runtime::CommandNotificationKind;
pub(crate) use codex_command_runtime::CommandNotificationState;
pub(crate) use codex_command_runtime::CommandProcessIdAllocator;
pub(crate) use codex_command_runtime::CommandWaitOutput;
pub(crate) use codex_command_runtime::CommandWaitRequest;
pub(crate) use codex_command_runtime::CommandWaitStatus;
pub(crate) use codex_command_runtime::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
pub(crate) use codex_command_runtime::HeadTailBuffer;
#[cfg(test)]
pub(crate) use codex_command_runtime::MIN_YIELD_TIME_MS;
pub(crate) use codex_command_runtime::WaitBackoffState;
pub(crate) use codex_command_runtime::WriteStdinOutput;
pub(crate) use codex_command_runtime::WriteStdinRequest;
pub(crate) use codex_command_runtime::clamp_yield_time;
pub(crate) use codex_command_runtime::generate_chunk_id;
use codex_exec_server_api::ExecEnvironment;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_utils_absolute_path::AbsolutePathBuf;
use tokio::sync::Mutex;

use crate::network_approval::DeferredNetworkApproval;
use crate::sandboxing::SandboxPermissions;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::shell::ShellType;

mod async_watcher;
mod process_manager;
mod tool_host;

pub(crate) fn set_deterministic_process_ids_for_tests(enabled: bool) {
    process_manager::set_deterministic_process_ids_for_tests(enabled);
}

pub(crate) use codex_command_runtime::ExecServerEnvConfig;
#[cfg(test)]
pub(crate) use codex_command_runtime::NoopSpawnLifecycle;
#[cfg(unix)]
pub(crate) use codex_command_runtime::SpawnLifecycle;
pub(crate) use codex_command_runtime::SpawnLifecycleHandle;
pub(crate) use codex_command_runtime::UnifiedExecError;
pub(crate) use codex_command_runtime::UnifiedExecProcess;
pub(crate) use codex_tool_runtime_api::ExecCommandArgs;
pub(crate) use process_manager::UnifiedExecCommandSessionController;
pub(crate) use tool_host::get_command;

pub(crate) const MAX_UNIFIED_EXEC_PROCESSES: usize = 64;

pub(crate) struct UnifiedExecContext {
    pub session: Arc<Session>,
    pub turn: Arc<TurnContext>,
    pub call_id: String,
}

impl UnifiedExecContext {
    pub fn new(session: Arc<Session>, turn: Arc<TurnContext>, call_id: String) -> Self {
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
    pub shell_type: ShellType,
    pub hook_command: String,
    pub process_id: i32,
    pub yield_time_ms: u64,
    pub max_output_tokens: Option<usize>,
    pub cwd: AbsolutePathBuf,
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub network: Option<SharedNetworkProxyRuntime>,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub additional_permissions_preapproved: bool,
    pub justification: Option<String>,
    pub prefix_rule: Option<Vec<String>>,
    pub notify_on: CommandNotificationFilter,
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

pub struct UnifiedExecProcessManager {
    process_store: Mutex<ProcessStore>,
    command_wait_hard_cap: std::time::Duration,
}

#[derive(Clone)]
pub struct UnifiedExecManagerHandle {
    manager: Weak<UnifiedExecProcessManager>,
}

impl UnifiedExecManagerHandle {
    pub fn new(manager: Weak<UnifiedExecProcessManager>) -> Self {
        Self { manager }
    }

    pub fn upgrade(&self) -> Option<Arc<UnifiedExecProcessManager>> {
        self.manager.upgrade()
    }
}

pub struct ProcessExitSubscription {
    process: Arc<UnifiedExecProcess>,
    cancellation_token: tokio_util::sync::CancellationToken,
    transcript: Arc<Mutex<HeadTailBuffer>>,
}

impl ProcessExitSubscription {
    pub async fn wait(&self) -> Option<i32> {
        self.cancellation_token.cancelled().await;
        self.process.exit_code()
    }

    pub async fn wait_with_retained_output(&self) -> (Option<i32>, String) {
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
    network_approval: Option<DeferredNetworkApproval>,
    session: Weak<Session>,
    last_used: tokio::time::Instant,
    transcript: Arc<Mutex<HeadTailBuffer>>,
    notification_state: Arc<CommandNotificationState>,
    command_wait_backoff: WaitBackoffState,
}

#[cfg(test)]
#[cfg(unix)]
#[path = "mod_tests.rs"]
mod tests;
