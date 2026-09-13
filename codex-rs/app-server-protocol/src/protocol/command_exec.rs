use super::PermissionProfile;
use super::SandboxPolicy;
use codex_experimental_api_macros::ExperimentalApi;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

/// PTY size in character cells for `command/exec` PTY sessions.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecTerminalSize {
    /// Terminal height in character cells.
    pub rows: u16,
    /// Terminal width in character cells.
    pub cols: u16,
}

/// Run a standalone command (argv vector) in the server sandbox without
/// creating a thread or turn.
///
/// The final `command/exec` response is deferred until the process exits and is
/// sent only after all `command/exec/outputDelta` notifications for that
/// connection have been emitted.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecParams {
    /// Command argv vector. Empty arrays are rejected.
    pub command: Vec<String>,
    /// Optional client-supplied, connection-scoped process id.
    ///
    /// Required for `tty`, `streamStdin`, `streamStdoutStderr`, and follow-up
    /// `command/exec/write`, `command/exec/resize`, and
    /// `command/exec/terminate` calls. When omitted, buffered execution gets an
    /// internal id that is not exposed to the client.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub process_id: Option<String>,
    /// Enable PTY mode.
    ///
    /// This implies `streamStdin` and `streamStdoutStderr`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tty: bool,
    /// Allow follow-up `command/exec/write` requests to write stdin bytes.
    ///
    /// Requires a client-supplied `processId`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stream_stdin: bool,
    /// Stream stdout/stderr via `command/exec/outputDelta` notifications.
    ///
    /// Streamed bytes are not duplicated into the final response and require a
    /// client-supplied `processId`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stream_stdout_stderr: bool,
    /// Optional per-stream stdout/stderr capture cap in bytes.
    ///
    /// When omitted, the server default applies. Cannot be combined with
    /// `disableOutputCap`.
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub output_bytes_cap: Option<usize>,
    /// Disable stdout/stderr capture truncation for this request.
    ///
    /// Cannot be combined with `outputBytesCap`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disable_output_cap: bool,
    /// Disable the timeout entirely for this request.
    ///
    /// Cannot be combined with `timeoutMs`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disable_timeout: bool,
    /// Optional timeout in milliseconds.
    ///
    /// When omitted, the server default applies. Cannot be combined with
    /// `disableTimeout`.
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub timeout_ms: Option<i64>,
    /// Optional working directory. Defaults to the server cwd.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<PathBuf>,
    /// Optional environment overrides merged into the server-computed
    /// environment.
    ///
    /// Matching names override inherited values. Set a key to `null` to unset
    /// an inherited variable.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub env: Option<HashMap<String, Option<String>>>,
    /// Optional initial PTY size in character cells. Only valid when `tty` is
    /// true.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub size: Option<CommandExecTerminalSize>,
    /// Optional sandbox policy for this command.
    ///
    /// Uses the same shape as thread/turn execution sandbox configuration and
    /// defaults to the user's configured policy when omitted. Cannot be
    /// combined with `permissionProfile`.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub sandbox_policy: Option<SandboxPolicy>,
    /// Optional full permissions profile for this command.
    ///
    /// Defaults to the user's configured permissions when omitted. Cannot be
    /// combined with `sandboxPolicy`.
    #[experimental("command/exec.permissionProfile")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub permission_profile: Option<PermissionProfile>,
}

/// Final buffered result for `command/exec`.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecResponse {
    /// Process exit code.
    pub exit_code: i32,
    /// Buffered stdout capture.
    ///
    /// Empty when stdout was streamed via `command/exec/outputDelta`.
    pub stdout: String,
    /// Buffered stderr capture.
    ///
    /// Empty when stderr was streamed via `command/exec/outputDelta`.
    pub stderr: String,
}

/// Write stdin bytes to a running `command/exec` session, close stdin, or
/// both.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecWriteParams {
    /// Client-supplied, connection-scoped `processId` from the original
    /// `command/exec` request.
    pub process_id: String,
    /// Optional base64-encoded stdin bytes to write.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub delta_base64: Option<String>,
    /// Close stdin after writing `deltaBase64`, if present.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub close_stdin: bool,
}

/// Empty success response for `command/exec/write`.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecWriteResponse {}

/// Terminate a running `command/exec` session.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecTerminateParams {
    /// Client-supplied, connection-scoped `processId` from the original
    /// `command/exec` request.
    pub process_id: String,
}

/// Empty success response for `command/exec/terminate`.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecTerminateResponse {}

/// Resize a running PTY-backed `command/exec` session.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecResizeParams {
    /// Client-supplied, connection-scoped `processId` from the original
    /// `command/exec` request.
    pub process_id: String,
    /// New PTY size in character cells.
    pub size: CommandExecTerminalSize,
}

/// Empty success response for `command/exec/resize`.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecResizeResponse {}

/// Stream label for `command/exec/outputDelta` notifications.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CommandExecOutputStream {
    /// stdout stream. PTY mode multiplexes terminal output here.
    Stdout,
    /// stderr stream.
    Stderr,
}
/// Base64-encoded output chunk emitted for a streaming `command/exec` request.
///
/// These notifications are delivered to the session's currently attached
/// connection. PTY sessions can rebind delivery through `terminal/session/list`
/// after reconnect; other streaming commands remain connection-scoped.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecOutputDeltaNotification {
    /// Client-supplied, connection-scoped `processId` from the original
    /// `command/exec` request.
    pub process_id: String,
    /// Runtime generation for this process id.
    pub generation: String,
    /// Monotonic byte-chunk sequence within this generation.
    pub sequence: u64,
    /// Output stream for this chunk.
    pub stream: CommandExecOutputStream,
    /// Base64-encoded output bytes.
    pub delta_base64: String,
    /// `true` on the final streamed chunk for a stream when `outputBytesCap`
    /// truncated later output on that stream.
    pub cap_reached: bool,
}

/// PTY lifecycle notification emitted once a user-owned terminal has started.
///
/// `generation` is allocated by the server for this concrete process lifetime;
/// clients must use it for subsequent terminal control requests.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecStartedNotification {
    pub process_id: String,
    pub generation: String,
    /// Server-generated capability required to reattach this user PTY from a
    /// different app-server connection.
    pub resume_token: String,
}

/// Terminal lifecycle notification for a client-owned command session.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecExitedNotification {
    pub process_id: String,
    pub generation: String,
    pub exit_code: i32,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum TerminalSessionOrigin {
    User,
    Model,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionDescriptor {
    pub session_id: String,
    pub generation: String,
    pub origin: TerminalSessionOrigin,
    pub thread_id: Option<String>,
    pub command_item_id: Option<String>,
    pub process_id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub replay_base64: Option<String>,
    pub replay_truncated: bool,
    /// Sequence of the newest chunk included in `replayBase64`.
    pub replay_through_sequence: u64,
    pub can_resize: bool,
    pub can_write: bool,
    pub can_terminate: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionListParams {
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub thread_id: Option<String>,
    /// Server-generated capabilities for user terminals this client is
    /// authorized to reattach.
    ///
    /// A caller can always see terminals it originally created. A different
    /// connection must explicitly prove knowledge of one of these tokens; listing
    /// never claims every live user PTY.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_resume_tokens: Vec<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionListResponse {
    pub data: Vec<TerminalSessionDescriptor>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionRef {
    pub session_id: String,
    pub generation: String,
    pub origin: TerminalSessionOrigin,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub thread_id: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub command_item_id: Option<String>,
    pub process_id: String,
    /// Server-generated capability for a user terminal reattachment.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub resume_token: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionWriteParams {
    #[serde(flatten)]
    pub target: TerminalSessionRef,
    pub delta_base64: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionResizeParams {
    #[serde(flatten)]
    pub target: TerminalSessionRef,
    pub size: CommandExecTerminalSize,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalPreferredSizeUpdateParams {
    pub thread_id: protocol::ThreadId,
    pub size: CommandExecTerminalSize,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalPreferredSizeUpdateResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionTerminateParams {
    #[serde(flatten)]
    pub target: TerminalSessionRef,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionWriteResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionResizeResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TerminalSessionTerminateResponse {}
