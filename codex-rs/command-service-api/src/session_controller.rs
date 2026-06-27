use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::CommandWaitOutput;
use crate::CommandWaitRequest;
use crate::WriteStdinOutput;
use crate::WriteStdinRequest;

pub type CommandSessionFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSessionError {
    message: String,
}

impl CommandSessionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl Error for CommandSessionError {}

/// In-flight `command_wait` operation returned by a command session controller.
///
/// Implementations own any runtime-specific wait token and complete the wait
/// without exposing process internals to tool handlers.
pub trait CommandWaitOperation: Send {
    fn process_id(&self) -> i32;

    fn wait_timeout(&self) -> Duration;

    fn finish(
        self: Box<Self>,
    ) -> CommandSessionFuture<'static, Result<CommandWaitOutput, CommandSessionError>>;
}

/// Controller for already-running command sessions.
///
/// This trait is the lightweight command-runtime service boundary used by
/// session/tool code. Implementations may own concrete process managers, but
/// callers only see command-runtime DTOs and futures.
pub trait CommandSessionController: Send + Sync {
    fn begin_command_wait<'a>(
        &'a self,
        request: CommandWaitRequest,
    ) -> CommandSessionFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>>;

    fn write_command_stdin<'a>(
        &'a self,
        request: WriteStdinRequest<'a>,
    ) -> CommandSessionFuture<'a, Result<WriteStdinOutput, CommandSessionError>>;
}
