//! Lightweight exec-server runtime capability traits.
//!
//! `codex-exec-server-protocol` owns JSON-RPC wire DTOs. This crate owns the
//! trait boundary that higher-level runtimes can depend on without compiling
//! the concrete exec-server implementation, transport clients, HTTP stack, or
//! local process runtime.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use codex_exec_server_protocol::ExecParams;
use codex_exec_server_protocol::HttpRequestParams;
use codex_exec_server_protocol::HttpRequestResponse;
use codex_exec_server_protocol::ProcessId;
use codex_exec_server_protocol::ProcessOutputChunk;
use codex_exec_server_protocol::ReadResponse;
use codex_exec_server_protocol::WriteResponse;
use codex_file_system::ExecutorFileSystem;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use tokio::sync::broadcast;
use tokio::sync::watch;

/// Canonical id for the local execution/filesystem environment.
pub const LOCAL_ENVIRONMENT_ID: &str = "local";

/// Canonical id for the legacy single remote execution/filesystem environment.
pub const REMOTE_ENVIRONMENT_ID: &str = "remote";

pub type ExecRuntimeResult<T> = Result<T, ExecRuntimeError>;

#[derive(Debug, thiserror::Error)]
pub enum ExecRuntimeError {
    #[error("exec-server transport closed")]
    Closed,
    #[error("{0}")]
    Disconnected(String),
    #[error("HTTP request failed: {0}")]
    HttpRequest(String),
    #[error("exec-server protocol error: {0}")]
    Protocol(String),
    #[error("exec-server rejected request ({code}): {message}")]
    Server { code: i64, message: String },
    #[error("{0}")]
    Other(String),
}

/// Request-scoped stream of HTTP response body chunks.
pub trait HttpResponseBody: Send {
    fn recv(&mut self) -> BoxFuture<'_, ExecRuntimeResult<Option<Vec<u8>>>>;
}

pub type HttpResponseBodyStream = Box<dyn HttpResponseBody>;

/// Sends HTTP requests through a runtime-selected transport.
///
/// Implementations may run requests locally or forward them through a remote
/// executor. Callers should not assume a concrete connection or process model.
pub trait HttpClient: Send + Sync {
    fn http_request(
        &self,
        params: HttpRequestParams,
    ) -> BoxFuture<'_, ExecRuntimeResult<HttpRequestResponse>>;

    fn http_request_stream(
        &self,
        params: HttpRequestParams,
    ) -> BoxFuture<'_, ExecRuntimeResult<(HttpRequestResponse, HttpResponseBodyStream)>>;
}

pub struct StartedExecProcess {
    pub process: Arc<dyn ExecProcess>,
}

/// Pushed process events for consumers that follow process output live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecProcessEvent {
    Output(ProcessOutputChunk),
    Exited { seq: u64, exit_code: i32 },
    Closed { seq: u64 },
    Failed(String),
}

/// Replay buffer plus live fan-out for pushed process events.
#[derive(Clone)]
pub struct ExecProcessEventLog {
    inner: Arc<ExecProcessEventLogInner>,
}

struct ExecProcessEventLogInner {
    history: StdMutex<ExecProcessEventHistory>,
    live_tx: broadcast::Sender<ExecProcessEvent>,
    event_capacity: usize,
    byte_capacity: usize,
}

#[derive(Default)]
struct ExecProcessEventHistory {
    events: VecDeque<ExecProcessEvent>,
    retained_bytes: usize,
}

impl ExecProcessEvent {
    pub fn seq(&self) -> Option<u64> {
        match self {
            ExecProcessEvent::Output(chunk) => Some(chunk.seq),
            ExecProcessEvent::Exited { seq, .. } | ExecProcessEvent::Closed { seq } => Some(*seq),
            ExecProcessEvent::Failed(_) => None,
        }
    }

    fn retained_len(&self) -> usize {
        match self {
            ExecProcessEvent::Output(chunk) => chunk.chunk.0.len(),
            ExecProcessEvent::Failed(message) => message.len(),
            ExecProcessEvent::Exited { .. } | ExecProcessEvent::Closed { .. } => 0,
        }
    }
}

impl ExecProcessEventLog {
    pub fn new(event_capacity: usize, byte_capacity: usize) -> Self {
        let (live_tx, _live_rx) = broadcast::channel(event_capacity);
        Self {
            inner: Arc::new(ExecProcessEventLogInner {
                history: StdMutex::new(ExecProcessEventHistory::default()),
                live_tx,
                event_capacity,
                byte_capacity,
            }),
        }
    }

    pub fn publish(&self, event: ExecProcessEvent) {
        let mut history = self
            .inner
            .history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        history.retained_bytes += event.retained_len();
        history.events.push_back(event.clone());
        while history.events.len() > self.inner.event_capacity
            || history.retained_bytes > self.inner.byte_capacity
        {
            let Some(evicted) = history.events.pop_front() else {
                break;
            };
            history.retained_bytes = history
                .retained_bytes
                .saturating_sub(evicted.retained_len());
        }

        let _ = self.inner.live_tx.send(event);
    }

    pub fn subscribe(&self) -> ExecProcessEventReceiver {
        let history = self
            .inner
            .history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let live_rx = self.inner.live_tx.subscribe();
        let replay = history.events.iter().cloned().collect();

        ExecProcessEventReceiver { replay, live_rx }
    }
}

pub struct ExecProcessEventReceiver {
    replay: VecDeque<ExecProcessEvent>,
    live_rx: broadcast::Receiver<ExecProcessEvent>,
}

impl ExecProcessEventReceiver {
    pub fn empty() -> Self {
        let (_live_tx, live_rx) = broadcast::channel(1);
        Self {
            replay: VecDeque::new(),
            live_rx,
        }
    }

    pub async fn recv(&mut self) -> Result<ExecProcessEvent, broadcast::error::RecvError> {
        if let Some(event) = self.replay.pop_front() {
            return Ok(event);
        }

        self.live_rx.recv().await
    }
}

/// Handle for an executor-managed process.
///
/// Implementations must support retained-output reads and pushed events.
pub trait ExecProcess: Send + Sync {
    fn process_id(&self) -> &ProcessId;

    fn subscribe_wake(&self) -> watch::Receiver<u64>;

    fn subscribe_events(&self) -> ExecProcessEventReceiver;

    fn read(
        &self,
        after_seq: Option<u64>,
        max_bytes: Option<usize>,
        wait_ms: Option<u64>,
    ) -> BoxFuture<'_, ExecRuntimeResult<ReadResponse>>;

    fn write(&self, chunk: Vec<u8>) -> BoxFuture<'_, ExecRuntimeResult<WriteResponse>>;

    fn terminate(&self) -> BoxFuture<'_, ExecRuntimeResult<()>>;
}

/// Starts executor-managed processes.
pub trait ExecBackend: Send + Sync {
    fn start(&self, params: ExecParams) -> BoxFuture<'_, ExecRuntimeResult<StartedExecProcess>>;
}

/// Selected execution/filesystem environment exposed to higher-level runtimes.
///
/// Implementations own the concrete process, HTTP, and filesystem backends.
/// Callers should depend on this capability trait when they only need to run
/// work in an already-selected environment, and leave environment discovery or
/// registry management to a separate provider boundary.
pub trait ExecEnvironment: std::fmt::Debug + Send + Sync {
    fn is_remote(&self) -> bool;

    fn exec_server_url(&self) -> Option<&str>;

    fn local_runtime_paths(&self) -> Option<&ExecServerRuntimePaths>;

    fn get_exec_backend(&self) -> Arc<dyn ExecBackend>;

    fn get_http_client(&self) -> Arc<dyn HttpClient>;

    fn get_filesystem(&self) -> Arc<dyn ExecutorFileSystem>;
}

/// Registry view for resolving configured execution environments.
///
/// Implementations own discovery, storage, and concrete environment
/// construction. Higher-level runtime code should use this trait when it only
/// needs to resolve already-configured environments and should leave provider
/// loading or registry mutation to composition-root code.
pub trait ExecEnvironmentProvider: std::fmt::Debug + Send + Sync {
    fn default_environment(&self) -> Option<Arc<dyn ExecEnvironment>>;

    fn default_environment_ids(&self) -> Vec<String>;

    fn local_environment(&self) -> Arc<dyn ExecEnvironment>;

    fn get_environment(&self, environment_id: &str) -> Option<Arc<dyn ExecEnvironment>>;
}

impl<T: ExecEnvironmentProvider + ?Sized> ExecEnvironmentProvider for Arc<T> {
    fn default_environment(&self) -> Option<Arc<dyn ExecEnvironment>> {
        self.as_ref().default_environment()
    }

    fn default_environment_ids(&self) -> Vec<String> {
        self.as_ref().default_environment_ids()
    }

    fn local_environment(&self) -> Arc<dyn ExecEnvironment> {
        self.as_ref().local_environment()
    }

    fn get_environment(&self, environment_id: &str) -> Option<Arc<dyn ExecEnvironment>> {
        self.as_ref().get_environment(environment_id)
    }
}

/// Runtime paths needed by exec-server child processes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecServerRuntimePaths {
    /// Stable path to the Codex executable used to launch hidden helper modes.
    pub codex_self_exe: AbsolutePathBuf,
    /// Path to the Linux sandbox helper alias used when the platform sandbox
    /// needs to re-enter Codex by argv0.
    pub codex_linux_sandbox_exe: Option<AbsolutePathBuf>,
}

impl ExecServerRuntimePaths {
    pub fn from_optional_paths(
        codex_self_exe: Option<PathBuf>,
        codex_linux_sandbox_exe: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let codex_self_exe = codex_self_exe.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Codex executable path is not configured",
            )
        })?;
        Self::new(codex_self_exe, codex_linux_sandbox_exe)
    }

    pub fn new(
        codex_self_exe: PathBuf,
        codex_linux_sandbox_exe: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            codex_self_exe: absolute_path(codex_self_exe)?,
            codex_linux_sandbox_exe: codex_linux_sandbox_exe.map(absolute_path).transpose()?,
        })
    }
}

fn absolute_path(path: PathBuf) -> std::io::Result<AbsolutePathBuf> {
    AbsolutePathBuf::from_absolute_path(path.as_path())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
}

#[cfg(test)]
mod tests {
    use super::ExecProcessEvent;
    use super::ExecProcessEventLog;
    use codex_exec_server_protocol::ExecOutputStream;
    use codex_exec_server_protocol::ProcessOutputChunk;
    use pretty_assertions::assert_eq;
    use tokio::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn event_history_replay_is_bounded_by_retained_bytes() {
        let log = ExecProcessEventLog::new(/*event_capacity*/ 8, /*byte_capacity*/ 3);

        log.publish(ExecProcessEvent::Output(ProcessOutputChunk {
            seq: 1,
            stream: ExecOutputStream::Stdout,
            chunk: b"large".to_vec().into(),
        }));
        log.publish(ExecProcessEvent::Exited {
            seq: 2,
            exit_code: 0,
        });
        log.publish(ExecProcessEvent::Closed { seq: 3 });

        let mut events = log.subscribe();
        let replay = vec![
            timeout(Duration::from_secs(1), events.recv())
                .await
                .expect("exit event replay should not time out")
                .expect("exit event replay should be available"),
            timeout(Duration::from_secs(1), events.recv())
                .await
                .expect("closed event replay should not time out")
                .expect("closed event replay should be available"),
        ];

        assert_eq!(
            replay,
            vec![
                ExecProcessEvent::Exited {
                    seq: 2,
                    exit_code: 0,
                },
                ExecProcessEvent::Closed { seq: 3 },
            ]
        );
    }
}
