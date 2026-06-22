use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::DEFAULT_COMMAND_OUTPUT_MAX_BYTES;

pub const DEFAULT_EXEC_COMMAND_TIMEOUT_MS: u64 = 10_000;

/// Hard cap on bytes retained from exec stdout/stderr/aggregated output.
///
/// This mirrors unified exec's output cap so a single runaway command cannot
/// OOM the process by dumping huge amounts of data to stdout/stderr.
pub const DEFAULT_EXEC_OUTPUT_MAX_BYTES: usize = DEFAULT_COMMAND_OUTPUT_MAX_BYTES;

/// Limit the number of ExecCommandOutputDelta events emitted per exec call.
/// Aggregation still collects full output; only the live event stream is capped.
pub const MAX_EXEC_OUTPUT_DELTAS_PER_CALL: usize = 10_000;

// Wait for the stdout/stderr collection tasks but guard against them
// hanging forever. In the normal case, both pipes are closed once the child
// terminates so the tasks exit quickly. However, if the child process
// spawned grandchildren that inherited its stdout/stderr file descriptors
// those pipes may stay open after we `kill` the direct child on timeout.
// That would cause the read tasks to block on `read()` indefinitely,
// effectively hanging the whole agent.
pub const IO_DRAIN_TIMEOUT_MS: u64 = 2_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecCapturePolicy {
    /// Shell-like execs keep the historical output cap and timeout behavior.
    #[default]
    ShellTool,
    /// Trusted internal helpers can buffer the full child output in memory
    /// without the shell-oriented output cap or exec-expiration behavior.
    FullBuffer,
}

impl ExecCapturePolicy {
    pub fn retained_bytes_cap(self) -> Option<usize> {
        match self {
            Self::ShellTool => Some(DEFAULT_EXEC_OUTPUT_MAX_BYTES),
            Self::FullBuffer => None,
        }
    }

    pub fn io_drain_timeout(self) -> Duration {
        Duration::from_millis(IO_DRAIN_TIMEOUT_MS)
    }

    pub fn uses_expiration(self) -> bool {
        match self {
            Self::ShellTool => true,
            Self::FullBuffer => false,
        }
    }
}

/// Mechanism to terminate an exec invocation before it finishes naturally.
#[derive(Clone, Debug)]
pub enum ExecExpiration {
    Timeout(Duration),
    DefaultTimeout,
    Cancellation(CancellationToken),
    TimeoutOrCancellation {
        timeout: Duration,
        cancellation: CancellationToken,
    },
}

/// Why an `ExecExpiration` completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecExpirationOutcome {
    /// The configured timeout elapsed.
    TimedOut,
    /// The cancellation token was cancelled.
    Cancelled,
}

impl From<Option<u64>> for ExecExpiration {
    fn from(timeout_ms: Option<u64>) -> Self {
        timeout_ms.map_or(ExecExpiration::DefaultTimeout, |timeout_ms| {
            ExecExpiration::Timeout(Duration::from_millis(timeout_ms))
        })
    }
}

impl From<u64> for ExecExpiration {
    fn from(timeout_ms: u64) -> Self {
        ExecExpiration::Timeout(Duration::from_millis(timeout_ms))
    }
}

impl ExecExpiration {
    /// Waits for this expiration and reports whether it timed out or was cancelled.
    pub async fn wait_with_outcome(self) -> ExecExpirationOutcome {
        match self {
            ExecExpiration::Timeout(duration) => {
                tokio::time::sleep(duration).await;
                ExecExpirationOutcome::TimedOut
            }
            ExecExpiration::DefaultTimeout => {
                tokio::time::sleep(Duration::from_millis(DEFAULT_EXEC_COMMAND_TIMEOUT_MS)).await;
                ExecExpirationOutcome::TimedOut
            }
            ExecExpiration::Cancellation(cancel) => {
                cancel.cancelled().await;
                ExecExpirationOutcome::Cancelled
            }
            ExecExpiration::TimeoutOrCancellation {
                timeout,
                cancellation,
            } => {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => ExecExpirationOutcome::Cancelled,
                    _ = tokio::time::sleep(timeout) => ExecExpirationOutcome::TimedOut,
                }
            }
        }
    }

    /// If ExecExpiration is a timeout, returns the timeout in milliseconds.
    pub fn timeout_ms(&self) -> Option<u64> {
        match self {
            ExecExpiration::Timeout(duration) => Some(duration.as_millis() as u64),
            ExecExpiration::DefaultTimeout => Some(DEFAULT_EXEC_COMMAND_TIMEOUT_MS),
            ExecExpiration::Cancellation(_) => None,
            ExecExpiration::TimeoutOrCancellation { timeout, .. } => {
                Some(timeout.as_millis() as u64)
            }
        }
    }

    pub fn with_cancellation(self, cancellation: CancellationToken) -> Self {
        match self {
            ExecExpiration::Timeout(timeout) => ExecExpiration::TimeoutOrCancellation {
                timeout,
                cancellation,
            },
            ExecExpiration::DefaultTimeout => ExecExpiration::TimeoutOrCancellation {
                timeout: Duration::from_millis(DEFAULT_EXEC_COMMAND_TIMEOUT_MS),
                cancellation,
            },
            ExecExpiration::Cancellation(existing) => {
                ExecExpiration::Cancellation(cancel_when_either(existing, cancellation))
            }
            ExecExpiration::TimeoutOrCancellation {
                timeout,
                cancellation: existing,
            } => ExecExpiration::TimeoutOrCancellation {
                timeout,
                cancellation: cancel_when_either(existing, cancellation),
            },
        }
    }
}

pub fn cancel_when_either(
    first: CancellationToken,
    second: CancellationToken,
) -> CancellationToken {
    let combined = CancellationToken::new();
    let cancel = combined.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = first.cancelled() => {}
            _ = second.cancelled() => {}
        }
        cancel.cancel();
    });
    combined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_buffer_capture_policy_disables_caps_and_exec_expiration() {
        assert_eq!(ExecCapturePolicy::FullBuffer.retained_bytes_cap(), None);
        assert_eq!(
            ExecCapturePolicy::FullBuffer.io_drain_timeout(),
            Duration::from_millis(IO_DRAIN_TIMEOUT_MS)
        );
        assert!(!ExecCapturePolicy::FullBuffer.uses_expiration());
    }

    #[test]
    fn shell_capture_policy_uses_default_output_cap_and_expiration() {
        assert_eq!(
            ExecCapturePolicy::ShellTool.retained_bytes_cap(),
            Some(DEFAULT_EXEC_OUTPUT_MAX_BYTES)
        );
        assert!(ExecCapturePolicy::ShellTool.uses_expiration());
    }

    #[test]
    fn timeout_ms_maps_default_timeout_and_cancellation() {
        assert_eq!(
            ExecExpiration::DefaultTimeout.timeout_ms(),
            Some(DEFAULT_EXEC_COMMAND_TIMEOUT_MS)
        );
        assert_eq!(
            ExecExpiration::Cancellation(CancellationToken::new()).timeout_ms(),
            None
        );
    }
}
