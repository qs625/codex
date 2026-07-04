use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use command_service_api::ExecCapturePolicy;
use command_service_api::ExecExpiration;
use command_service_api::ExecExpirationOutcome;
use command_service_api::MAX_EXEC_OUTPUT_DELTAS_PER_CALL;
use command_service_api::bytes_to_string_smart;
use command_service_api::is_likely_sandbox_denied;
use protocol::error::CodexErr;
use protocol::error::Result;
use protocol::error::SandboxErr;
use protocol::exec_output::ExecToolCallOutput;
use protocol::exec_output::StreamOutput;
use codex_sandboxing_api::SandboxType;
use codex_utils_pty::process_group::kill_child_process_group;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::sync::mpsc;

const SIGKILL_CODE: i32 = 9;
const TIMEOUT_CODE: i32 = 64;
const EXIT_CODE_SIGNAL_BASE: i32 = 128;
const EXEC_TIMEOUT_EXIT_CODE: i32 = 124;
const READ_CHUNK_SIZE: usize = 8192;
const AGGREGATE_BUFFER_INITIAL_CAPACITY: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug)]
pub struct ProcessOutputChunk {
    pub stream: ProcessOutputStream,
    pub chunk: Vec<u8>,
}

pub type ProcessOutputSender = mpsc::UnboundedSender<ProcessOutputChunk>;

#[derive(Debug, Eq, PartialEq)]
pub struct CapturedStreamOutput {
    pub text: Vec<u8>,
    pub truncated_after_lines: Option<u32>,
}

impl CapturedStreamOutput {
    pub fn from_text(text: Vec<u8>) -> Self {
        Self {
            text,
            truncated_after_lines: None,
        }
    }

    pub fn into_utf8_lossy(self) -> String {
        bytes_to_string_smart(&self.text)
    }
}

#[derive(Debug)]
pub struct CapturedProcessOutput {
    pub exit_status: ExitStatus,
    pub stdout: CapturedStreamOutput,
    pub stderr: CapturedStreamOutput,
    pub aggregated_output: CapturedStreamOutput,
    pub timed_out: bool,
}

pub async fn consume_process_output(
    mut child: Child,
    expiration: ExecExpiration,
    capture_policy: ExecCapturePolicy,
    output_sender: Option<ProcessOutputSender>,
) -> io::Result<CapturedProcessOutput> {
    let stdout_reader = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout pipe was unexpectedly not available"))?;
    let stderr_reader = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr pipe was unexpectedly not available"))?;

    let retained_bytes_cap = capture_policy.retained_bytes_cap();
    let stdout_handle = tokio::spawn(read_process_output(
        BufReader::new(stdout_reader),
        output_sender.clone(),
        ProcessOutputStream::Stdout,
        retained_bytes_cap,
    ));
    let stderr_handle = tokio::spawn(read_process_output(
        BufReader::new(stderr_reader),
        output_sender,
        ProcessOutputStream::Stderr,
        retained_bytes_cap,
    ));

    let expiration_wait = async {
        if capture_policy.uses_expiration() {
            Some(expiration.wait_with_outcome().await)
        } else {
            std::future::pending::<Option<ExecExpirationOutcome>>().await
        }
    };
    tokio::pin!(expiration_wait);
    let (exit_status, timed_out) = tokio::select! {
        status_result = child.wait() => {
            let exit_status = status_result?;
            (exit_status, false)
        }
        outcome = &mut expiration_wait => {
            kill_child_process_group(&mut child)?;
            child.start_kill()?;
            let timed_out = matches!(outcome, Some(ExecExpirationOutcome::TimedOut));
            let exit_status = if timed_out {
                synthetic_exit_status(EXIT_CODE_SIGNAL_BASE + TIMEOUT_CODE)
            } else {
                synthetic_exit_status_for_code(/*code*/ 1)
            };
            (exit_status, timed_out)
        }
        _ = tokio::signal::ctrl_c() => {
            kill_child_process_group(&mut child)?;
            child.start_kill()?;
            (synthetic_exit_status(EXIT_CODE_SIGNAL_BASE + SIGKILL_CODE), false)
        }
    };

    use tokio::task::JoinHandle;

    async fn await_output(
        handle: &mut JoinHandle<io::Result<CapturedStreamOutput>>,
        timeout: Duration,
    ) -> io::Result<CapturedStreamOutput> {
        match tokio::time::timeout(timeout, &mut *handle).await {
            Ok(join_res) => match join_res {
                Ok(io_res) => io_res,
                Err(join_err) => Err(io::Error::other(join_err)),
            },
            Err(_elapsed) => {
                handle.abort();
                Ok(CapturedStreamOutput::from_text(Vec::new()))
            }
        }
    }

    let mut stdout_handle = stdout_handle;
    let mut stderr_handle = stderr_handle;

    let stdout = await_output(&mut stdout_handle, capture_policy.io_drain_timeout()).await?;
    let stderr = await_output(&mut stderr_handle, capture_policy.io_drain_timeout()).await?;
    let aggregated_output = aggregate_output(&stdout, &stderr, retained_bytes_cap);

    Ok(CapturedProcessOutput {
        exit_status,
        stdout,
        stderr,
        aggregated_output,
        timed_out,
    })
}

pub fn finalize_captured_process_output(
    raw_output_result: std::result::Result<CapturedProcessOutput, CodexErr>,
    sandbox_type: SandboxType,
    duration: Duration,
) -> Result<ExecToolCallOutput> {
    match raw_output_result {
        Ok(raw_output) => {
            #[allow(unused_mut)]
            let mut timed_out = raw_output.timed_out;

            #[cfg(target_family = "unix")]
            {
                use std::os::unix::process::ExitStatusExt;

                if let Some(signal) = raw_output.exit_status.signal() {
                    if signal == TIMEOUT_CODE {
                        timed_out = true;
                    } else {
                        return Err(CodexErr::Sandbox(SandboxErr::Signal(signal)));
                    }
                }
            }

            let mut exit_code = raw_output.exit_status.code().unwrap_or(-1);
            if timed_out {
                exit_code = EXEC_TIMEOUT_EXIT_CODE;
            }

            let stdout = decode_stream_output(raw_output.stdout);
            let stderr = decode_stream_output(raw_output.stderr);
            let aggregated_output = decode_stream_output(raw_output.aggregated_output);
            let exec_output = ExecToolCallOutput {
                exit_code,
                stdout,
                stderr,
                aggregated_output,
                duration,
                timed_out,
            };

            if timed_out {
                return Err(CodexErr::Sandbox(SandboxErr::Timeout {
                    output: Box::new(exec_output),
                }));
            }

            if is_likely_sandbox_denied(sandbox_type, &exec_output) {
                return Err(CodexErr::Sandbox(SandboxErr::Denied {
                    output: Box::new(exec_output),
                    network_policy_decision: None,
                }));
            }

            Ok(exec_output)
        }
        Err(err) => {
            tracing::error!("exec error: {err}");
            Err(err)
        }
    }
}

fn decode_stream_output(output: CapturedStreamOutput) -> StreamOutput<String> {
    let truncated_after_lines = output.truncated_after_lines;
    StreamOutput {
        text: output.into_utf8_lossy(),
        truncated_after_lines,
    }
}

pub async fn read_process_output<R: AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    output_sender: Option<ProcessOutputSender>,
    stream: ProcessOutputStream,
    max_bytes: Option<usize>,
) -> io::Result<CapturedStreamOutput> {
    let mut buf = Vec::with_capacity(
        max_bytes.map_or(AGGREGATE_BUFFER_INITIAL_CAPACITY, |max_bytes| {
            AGGREGATE_BUFFER_INITIAL_CAPACITY.min(max_bytes)
        }),
    );
    let mut tmp = [0u8; READ_CHUNK_SIZE];
    let mut emitted_deltas: usize = 0;

    loop {
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }

        if let Some(sender) = &output_sender
            && emitted_deltas < MAX_EXEC_OUTPUT_DELTAS_PER_CALL
        {
            let _ = sender.send(ProcessOutputChunk {
                stream,
                chunk: tmp[..n].to_vec(),
            });
            emitted_deltas += 1;
        }

        if let Some(max_bytes) = max_bytes {
            append_capped(&mut buf, &tmp[..n], max_bytes);
        } else {
            buf.extend_from_slice(&tmp[..n]);
        }
    }

    Ok(CapturedStreamOutput::from_text(buf))
}

fn append_capped(dst: &mut Vec<u8>, src: &[u8], max_bytes: usize) {
    if dst.len() >= max_bytes {
        return;
    }
    let remaining = max_bytes.saturating_sub(dst.len());
    let take = remaining.min(src.len());
    dst.extend_from_slice(&src[..take]);
}

pub fn aggregate_output(
    stdout: &CapturedStreamOutput,
    stderr: &CapturedStreamOutput,
    max_bytes: Option<usize>,
) -> CapturedStreamOutput {
    let Some(max_bytes) = max_bytes else {
        let total_len = stdout.text.len().saturating_add(stderr.text.len());
        let mut aggregated = Vec::with_capacity(total_len);
        aggregated.extend_from_slice(&stdout.text);
        aggregated.extend_from_slice(&stderr.text);
        return CapturedStreamOutput::from_text(aggregated);
    };

    let total_len = stdout.text.len().saturating_add(stderr.text.len());
    let mut aggregated = Vec::with_capacity(total_len.min(max_bytes));

    if total_len <= max_bytes {
        aggregated.extend_from_slice(&stdout.text);
        aggregated.extend_from_slice(&stderr.text);
        return CapturedStreamOutput::from_text(aggregated);
    }

    let want_stdout = stdout.text.len().min(max_bytes / 3);
    let want_stderr = stderr.text.len();
    let stderr_take = want_stderr.min(max_bytes.saturating_sub(want_stdout));
    let remaining = max_bytes.saturating_sub(want_stdout + stderr_take);
    let stdout_take = want_stdout + remaining.min(stdout.text.len().saturating_sub(want_stdout));

    aggregated.extend_from_slice(&stdout.text[..stdout_take]);
    aggregated.extend_from_slice(&stderr.text[..stderr_take]);

    CapturedStreamOutput::from_text(aggregated)
}

#[cfg(unix)]
pub fn synthetic_exit_status(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code)
}

#[cfg(unix)]
fn synthetic_exit_status_for_code(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code << 8)
}

#[cfg(windows)]
pub fn synthetic_exit_status(code: i32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code as u32)
}

#[cfg(windows)]
fn synthetic_exit_status_for_code(code: i32) -> ExitStatus {
    synthetic_exit_status(code)
}
