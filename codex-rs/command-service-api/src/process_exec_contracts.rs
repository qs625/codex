use std::collections::HashMap;
use std::time::Duration;

use chardetng::EncodingDetector;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_sandboxing_api::SandboxType;
use codex_utils_absolute_path::AbsolutePathBuf;
use encoding_rs::Encoding;
use encoding_rs::IBM866;
use encoding_rs::WINDOWS_1252;
use protocol::exec_output::ExecToolCallOutput;
use protocol::models::SandboxPermissions;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_EXEC_COMMAND_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_EXEC_OUTPUT_MAX_BYTES: usize = 1024 * 1024;
pub const MAX_EXEC_OUTPUT_DELTAS_PER_CALL: usize = 10_000;
pub const IO_DRAIN_TIMEOUT_MS: u64 = 2_000;

/// Portable process execution request shared by shell/app-server entrypoints.
///
/// This type owns the model/user-facing request shape. Sandbox transformation,
/// environment marker injection, process spawning, and event emission remain in
/// the command service implementation crate.
#[derive(Debug)]
pub struct ExecParams {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub expiration: ExecExpiration,
    pub capture_policy: ExecCapturePolicy,
    pub env: HashMap<String, String>,
    pub network: Option<SharedNetworkProxyRuntime>,
    pub sandbox_permissions: SandboxPermissions,
    pub windows_sandbox_level: protocol::config_types::WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
    pub justification: Option<String>,
    pub arg0: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecCapturePolicy {
    #[default]
    ShellTool,
    FullBuffer,
}

#[derive(Debug)]
pub struct ExecOptions {
    pub expiration: ExecExpiration,
    pub capture_policy: ExecCapturePolicy,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecExpirationOutcome {
    TimedOut,
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

/// Attempts to convert arbitrary bytes to UTF-8 with best-effort encoding detection.
pub fn bytes_to_string_smart(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    if let Ok(utf8_str) = std::str::from_utf8(bytes) {
        return utf8_str.to_owned();
    }

    let encoding = detect_encoding(bytes);
    decode_bytes(bytes, encoding)
}

const WINDOWS_1252_PUNCT_BYTES: [u8; 8] = [0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x99];

fn detect_encoding(bytes: &[u8]) -> &'static Encoding {
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let (encoding, _is_confident) = detector.guess_assess(None, true);

    if encoding == IBM866 && looks_like_windows_1252_punctuation(bytes) {
        return WINDOWS_1252;
    }

    encoding
}

fn decode_bytes(bytes: &[u8], encoding: &'static Encoding) -> String {
    let (decoded, _, had_errors) = encoding.decode(bytes);

    if had_errors {
        return String::from_utf8_lossy(bytes).into_owned();
    }

    decoded.into_owned()
}

fn looks_like_windows_1252_punctuation(bytes: &[u8]) -> bool {
    let mut saw_extended_punctuation = false;
    let mut saw_ascii_word = false;

    for &byte in bytes {
        if byte >= 0xA0 {
            return false;
        }
        if (0x80..=0x9F).contains(&byte) {
            if !WINDOWS_1252_PUNCT_BYTES.contains(&byte) {
                return false;
            }
            saw_extended_punctuation = true;
        }
        if byte.is_ascii_alphabetic() {
            saw_ascii_word = true;
        }
    }

    saw_extended_punctuation && saw_ascii_word
}

/// Conservatively detects output patterns that usually mean the sandbox blocked
/// a command rather than the command itself failing.
pub fn is_likely_sandbox_denied(
    sandbox_type: SandboxType,
    exec_output: &ExecToolCallOutput,
) -> bool {
    if sandbox_type == SandboxType::None || exec_output.exit_code == 0 {
        return false;
    }

    const SANDBOX_DENIED_KEYWORDS: [&str; 7] = [
        "operation not permitted",
        "permission denied",
        "read-only file system",
        "seccomp",
        "sandbox",
        "landlock",
        "failed to write file",
    ];

    let has_sandbox_keyword = [
        &exec_output.stderr.text,
        &exec_output.stdout.text,
        &exec_output.aggregated_output.text,
    ]
    .into_iter()
    .any(|section| {
        let lower = section.to_lowercase();
        SANDBOX_DENIED_KEYWORDS
            .iter()
            .any(|needle| lower.contains(needle))
    });

    if has_sandbox_keyword {
        return true;
    }

    const QUICK_REJECT_EXIT_CODES: [i32; 3] = [2, 126, 127];
    if QUICK_REJECT_EXIT_CODES.contains(&exec_output.exit_code) {
        return false;
    }

    sandbox_type == SandboxType::LinuxSeccomp && exec_output.exit_code == 159
}
