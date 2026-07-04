use protocol::ThreadId;
use std::fmt;

/// Result type returned by thread-store operations.
pub type ThreadStoreResult<T> = Result<T, ThreadStoreError>;

/// Error type shared by thread-store implementations.
#[derive(Debug)]
pub enum ThreadStoreError {
    /// The requested thread does not exist in this store.
    ThreadNotFound {
        /// Thread id requested by the caller.
        thread_id: ThreadId,
    },

    /// The caller supplied invalid request data.
    InvalidRequest {
        /// User-facing explanation of the invalid request.
        message: String,
    },

    /// The operation conflicted with current store state.
    Conflict {
        /// User-facing explanation of the conflict.
        message: String,
    },

    /// The store implementation does not support this operation yet.
    Unsupported {
        /// Stable operation name for callers that need to map unsupported operations.
        operation: &'static str,
    },

    /// Catch-all for implementation failures that do not fit a more specific category.
    Internal {
        /// User-facing explanation of the implementation failure.
        message: String,
    },
}

impl fmt::Display for ThreadStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ThreadNotFound { thread_id } => write!(f, "thread {thread_id} not found"),
            Self::InvalidRequest { message } => {
                write!(f, "invalid thread-store request: {message}")
            }
            Self::Conflict { message } => write!(f, "thread-store conflict: {message}"),
            Self::Unsupported { operation } => {
                write!(f, "thread-store unsupported operation: {operation}")
            }
            Self::Internal { message } => write!(f, "thread-store internal error: {message}"),
        }
    }
}

impl std::error::Error for ThreadStoreError {}
