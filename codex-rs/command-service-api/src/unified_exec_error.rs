use protocol::exec_output::ExecToolCallOutput;

#[derive(Debug)]
pub enum UnifiedExecError {
    CreateProcess {
        message: String,
    },
    ProcessFailed {
        message: String,
    },
    UnknownProcessId {
        process_id: i32,
    },
    WriteToStdin,
    EmptyStdin,
    StdinClosed,
    MissingCommandLine,
    SandboxDenied {
        message: String,
        output: ExecToolCallOutput,
    },
}

impl UnifiedExecError {
    pub fn create_process(message: String) -> Self {
        Self::CreateProcess { message }
    }

    pub fn process_failed(message: String) -> Self {
        Self::ProcessFailed { message }
    }

    pub fn sandbox_denied(message: String, output: ExecToolCallOutput) -> Self {
        Self::SandboxDenied { message, output }
    }
}

impl std::fmt::Display for UnifiedExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateProcess { message } => {
                write!(f, "Failed to create unified exec process: {message}")
            }
            Self::ProcessFailed { message } => {
                write!(f, "Unified exec process failed: {message}")
            }
            Self::UnknownProcessId { process_id } => {
                write!(f, "Unknown command id {process_id}")
            }
            Self::WriteToStdin => write!(f, "failed to write to stdin"),
            Self::EmptyStdin => write!(
                f,
                "command_write_stdin requires non-empty chars; use command_wait for command completion or output notifications instead of polling for output"
            ),
            Self::StdinClosed => write!(
                f,
                "stdin is closed for this session; rerun exec_command with tty=true to keep stdin open"
            ),
            Self::MissingCommandLine => write!(f, "missing command line for unified exec request"),
            Self::SandboxDenied { message, .. } => {
                write!(f, "Command denied by sandbox: {message}")
            }
        }
    }
}

impl std::error::Error for UnifiedExecError {}
