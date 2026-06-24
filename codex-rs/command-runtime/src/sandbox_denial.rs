use codex_protocol::exec_output::ExecToolCallOutput;
use codex_sandboxing_api::SandboxType;

const LINUX_SIGSYS_CODE: i32 = 31;
const EXIT_CODE_SIGNAL_BASE: i32 = 128;

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

    sandbox_type == SandboxType::LinuxSeccomp
        && exec_output.exit_code == EXIT_CODE_SIGNAL_BASE + LINUX_SIGSYS_CODE
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::exec_output::StreamOutput;
    use std::time::Duration;

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
    fn requires_keywords() {
        let output = make_exec_output(/*exit_code*/ 1, "", "", "");
        assert!(!is_likely_sandbox_denied(
            SandboxType::LinuxSeccomp,
            &output
        ));
    }

    #[test]
    fn identifies_keyword_in_stderr() {
        let output = make_exec_output(/*exit_code*/ 1, "", "Operation not permitted", "");
        assert!(is_likely_sandbox_denied(SandboxType::LinuxSeccomp, &output));
    }

    #[test]
    fn respects_quick_reject_exit_codes() {
        let output = make_exec_output(/*exit_code*/ 127, "", "command not found", "");
        assert!(!is_likely_sandbox_denied(
            SandboxType::LinuxSeccomp,
            &output
        ));
    }

    #[test]
    fn ignores_non_sandbox_mode() {
        let output = make_exec_output(/*exit_code*/ 1, "", "Operation not permitted", "");
        assert!(!is_likely_sandbox_denied(SandboxType::None, &output));
    }

    #[test]
    fn ignores_network_policy_text_in_non_sandbox_mode() {
        let output = make_exec_output(
            /*exit_code*/ 0,
            "",
            "",
            r#"CODEX_NETWORK_POLICY_DECISION {"decision":"ask","reason":"not_allowed","source":"decider","protocol":"http","host":"google.com","port":80}"#,
        );
        assert!(!is_likely_sandbox_denied(SandboxType::None, &output));
    }

    #[test]
    fn uses_aggregated_output() {
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
    fn ignores_network_policy_text_with_zero_exit_code() {
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
    fn flags_sigsys_exit_code() {
        let output = make_exec_output(EXIT_CODE_SIGNAL_BASE + LINUX_SIGSYS_CODE, "", "", "");
        assert!(is_likely_sandbox_denied(SandboxType::LinuxSeccomp, &output));
    }
}
