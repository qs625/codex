//! Shared shell parsing, normalization, and safety utilities.

mod approval_canonicalization;
pub mod bash;
pub mod is_dangerous_command;
pub mod is_safe_command;
pub mod parse_command;
pub mod powershell;
mod powershell_parser;
mod shell_detect;
#[cfg(windows)]
mod windows_dangerous_commands;
mod windows_safe_commands;

use std::path::PathBuf;

pub use approval_canonicalization::canonicalize_command_for_approval;

pub fn shlex_join(tokens: &[String]) -> String {
    shlex::try_join(tokens.iter().map(String::as_str))
        .unwrap_or_else(|_| "<command included NUL byte>".to_string())
}

pub fn resolve_executable_in_path(binary_name: &str) -> Option<PathBuf> {
    which::which(binary_name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shlex_join_handles_nul_byte() {
        let command = vec!["echo".to_string(), "bad\0arg".to_string()];

        assert_eq!(shlex_join(&command), "<command included NUL byte>");
    }

    #[test]
    fn returns_none_for_missing_executable() {
        let binary_name = "codex-shell-utils-test-definitely-missing-executable";

        assert_eq!(resolve_executable_in_path(binary_name), None);
    }
}
