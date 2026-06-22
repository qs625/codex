use std::fmt;

/// Error returned while executing a model-visible tool invocation.
#[derive(Debug, PartialEq)]
pub enum FunctionCallError {
    RespondToModel(String),
    Fatal(String),
}

impl fmt::Display for FunctionCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RespondToModel(message) => write!(f, "{message}"),
            Self::Fatal(message) => write!(f, "Fatal error: {message}"),
        }
    }
}

impl std::error::Error for FunctionCallError {}
