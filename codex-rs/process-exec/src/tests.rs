use crate::DEFAULT_EXEC_OUTPUT_MAX_BYTES as EXEC_OUTPUT_MAX_BYTES;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::exec_output::StreamOutput;
use codex_sandboxing_api::SandboxType;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

use super::*;

fn make_exec_output(
    exit_code: i32,
    stdout: &str,
    stderr: &str,
    aggregated: &str,
) -> ExecToolCallOutput {
    ExecToolCallOutput {
        exit_code,
        stdout: StreamOutput::new(stdout.to_string()),
        stderr: StreamOutput::new(stderr.to_string()),
        aggregated_output: StreamOutput::new(aggregated.to_string()),
        duration: Duration::from_millis(1),
        timed_out: false,
    }
}

#[test]
fn sandbox_detection_requires_keywords() {
    let output = make_exec_output(/*exit_code*/ 1, "", "", "");
    assert!(!is_likely_sandbox_denied(
        SandboxType::LinuxSeccomp,
        &output
    ));
}

#[test]
fn sandbox_detection_identifies_keyword_in_stderr() {
    let output = make_exec_output(/*exit_code*/ 1, "", "Operation not permitted", "");
    assert!(is_likely_sandbox_denied(SandboxType::LinuxSeccomp, &output));
}

#[test]
fn sandbox_detection_respects_quick_reject_exit_codes() {
    let output = make_exec_output(/*exit_code*/ 127, "", "command not found", "");
    assert!(!is_likely_sandbox_denied(
        SandboxType::LinuxSeccomp,
        &output
    ));
}

#[test]
fn sandbox_detection_ignores_non_sandbox_mode() {
    let output = make_exec_output(/*exit_code*/ 1, "", "Operation not permitted", "");
    assert!(!is_likely_sandbox_denied(SandboxType::None, &output));
}

#[test]
fn sandbox_detection_ignores_network_policy_text_in_non_sandbox_mode() {
    let output = make_exec_output(
        /*exit_code*/ 0,
        "",
        "",
        r#"CODEX_NETWORK_POLICY_DECISION {"decision":"ask","reason":"not_allowed","source":"decider","protocol":"http","host":"google.com","port":80}"#,
    );
    assert!(!is_likely_sandbox_denied(SandboxType::None, &output));
}

#[test]
fn sandbox_detection_uses_aggregated_output() {
    let output = make_exec_output(
        /*exit_code*/ 101,
        "",
        "",
        "cargo failed: Read-only file system when writing target",
    );
    assert!(is_likely_sandbox_denied(
        SandboxType::MacosSeatbelt,
        &output
    ));
}

#[test]
fn sandbox_detection_ignores_network_policy_text_with_zero_exit_code() {
    let output = make_exec_output(
        /*exit_code*/ 0,
        "",
        "",
        r#"CODEX_NETWORK_POLICY_DECISION {"decision":"ask","source":"decider","protocol":"http","host":"google.com","port":80}"#,
    );

    assert!(!is_likely_sandbox_denied(
        SandboxType::LinuxSeccomp,
        &output
    ));
}

#[test]
fn sandbox_detection_flags_sigsys_exit_code() {
    let exit_code = EXIT_CODE_SIGNAL_BASE + LINUX_SIGSYS_CODE;
    let output = make_exec_output(exit_code, "", "", "");
    assert!(is_likely_sandbox_denied(SandboxType::LinuxSeccomp, &output));
}

#[tokio::test]
async fn read_output_limits_retained_bytes_for_shell_capture() {
    let (mut writer, reader) = tokio::io::duplex(1024);
    let bytes = vec![b'a'; EXEC_OUTPUT_MAX_BYTES.saturating_add(128 * 1024)];
    tokio::spawn(async move {
        writer.write_all(&bytes).await.expect("write");
    });

    let out = read_process_output(
        reader,
        /*output_sender*/ None,
        ProcessOutputStream::Stdout,
        Some(EXEC_OUTPUT_MAX_BYTES),
    )
    .await
    .expect("read");
    assert_eq!(out.text.len(), EXEC_OUTPUT_MAX_BYTES);
}

#[test]
fn aggregate_output_prefers_stderr_on_contention() {
    let stdout = CapturedStreamOutput {
        text: vec![b'a'; EXEC_OUTPUT_MAX_BYTES],
        truncated_after_lines: None,
    };
    let stderr = CapturedStreamOutput {
        text: vec![b'b'; EXEC_OUTPUT_MAX_BYTES],
        truncated_after_lines: None,
    };

    let aggregated = aggregate_output(&stdout, &stderr, Some(EXEC_OUTPUT_MAX_BYTES));
    let stdout_cap = EXEC_OUTPUT_MAX_BYTES / 3;
    let stderr_cap = EXEC_OUTPUT_MAX_BYTES.saturating_sub(stdout_cap);

    assert_eq!(aggregated.text.len(), EXEC_OUTPUT_MAX_BYTES);
    assert_eq!(aggregated.text[..stdout_cap], vec![b'a'; stdout_cap]);
    assert_eq!(aggregated.text[stdout_cap..], vec![b'b'; stderr_cap]);
}

#[test]
fn aggregate_output_fills_remaining_capacity_with_stderr() {
    let stdout_len = EXEC_OUTPUT_MAX_BYTES / 10;
    let stdout = CapturedStreamOutput {
        text: vec![b'a'; stdout_len],
        truncated_after_lines: None,
    };
    let stderr = CapturedStreamOutput {
        text: vec![b'b'; EXEC_OUTPUT_MAX_BYTES],
        truncated_after_lines: None,
    };

    let aggregated = aggregate_output(&stdout, &stderr, Some(EXEC_OUTPUT_MAX_BYTES));
    let stderr_cap = EXEC_OUTPUT_MAX_BYTES.saturating_sub(stdout_len);

    assert_eq!(aggregated.text.len(), EXEC_OUTPUT_MAX_BYTES);
    assert_eq!(aggregated.text[..stdout_len], vec![b'a'; stdout_len]);
    assert_eq!(aggregated.text[stdout_len..], vec![b'b'; stderr_cap]);
}

#[test]
fn aggregate_output_rebalances_when_stderr_is_small() {
    let stdout = CapturedStreamOutput {
        text: vec![b'a'; EXEC_OUTPUT_MAX_BYTES],
        truncated_after_lines: None,
    };
    let stderr = CapturedStreamOutput {
        text: vec![b'b'; 1],
        truncated_after_lines: None,
    };

    let aggregated = aggregate_output(&stdout, &stderr, Some(EXEC_OUTPUT_MAX_BYTES));
    let stdout_len = EXEC_OUTPUT_MAX_BYTES.saturating_sub(1);

    assert_eq!(aggregated.text.len(), EXEC_OUTPUT_MAX_BYTES);
    assert_eq!(aggregated.text[..stdout_len], vec![b'a'; stdout_len]);
    assert_eq!(aggregated.text[stdout_len..], vec![b'b'; 1]);
}

#[test]
fn aggregate_output_keeps_stdout_then_stderr_when_under_cap() {
    let stdout = CapturedStreamOutput {
        text: vec![b'a'; 4],
        truncated_after_lines: None,
    };
    let stderr = CapturedStreamOutput {
        text: vec![b'b'; 3],
        truncated_after_lines: None,
    };

    let aggregated = aggregate_output(&stdout, &stderr, Some(EXEC_OUTPUT_MAX_BYTES));
    let mut expected = Vec::new();
    expected.extend_from_slice(&stdout.text);
    expected.extend_from_slice(&stderr.text);

    assert_eq!(aggregated.text, expected);
    assert_eq!(aggregated.truncated_after_lines, None);
}

#[tokio::test]
async fn read_output_retains_all_bytes_for_full_buffer_capture() {
    let (mut writer, reader) = tokio::io::duplex(1024);
    let bytes = vec![b'a'; EXEC_OUTPUT_MAX_BYTES.saturating_add(128 * 1024)];
    let expected_len = bytes.len();
    tokio::spawn(async move {
        writer.write_all(&bytes).await.expect("write");
    });

    let out = read_process_output(
        reader,
        /*output_sender*/ None,
        ProcessOutputStream::Stdout,
        /*max_bytes*/ None,
    )
    .await
    .expect("read");
    assert_eq!(out.text.len(), expected_len);
}

#[test]
fn aggregate_output_keeps_all_bytes_when_uncapped() {
    let stdout = CapturedStreamOutput {
        text: vec![b'a'; EXEC_OUTPUT_MAX_BYTES],
        truncated_after_lines: None,
    };
    let stderr = CapturedStreamOutput {
        text: vec![b'b'; EXEC_OUTPUT_MAX_BYTES],
        truncated_after_lines: None,
    };

    let aggregated = aggregate_output(&stdout, &stderr, /*max_bytes*/ None);

    assert_eq!(aggregated.text.len(), EXEC_OUTPUT_MAX_BYTES * 2);
    assert_eq!(
        aggregated.text[..EXEC_OUTPUT_MAX_BYTES],
        vec![b'a'; EXEC_OUTPUT_MAX_BYTES]
    );
    assert_eq!(
        aggregated.text[EXEC_OUTPUT_MAX_BYTES..],
        vec![b'b'; EXEC_OUTPUT_MAX_BYTES]
    );
}
