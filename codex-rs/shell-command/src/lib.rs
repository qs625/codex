//! Command parsing and safety utilities shared across Codex crates.

mod approval_canonicalization;
pub mod bash;
pub(crate) mod command_safety;
pub mod parse_command;
pub mod powershell;

pub use approval_canonicalization::canonicalize_command_for_approval;
pub use codex_shell_utils::resolve_executable_in_path;
pub use command_safety::is_dangerous_command;
pub use command_safety::is_safe_command;

#[cfg(test)]
mod tests {
    use super::resolve_executable_in_path;

    #[test]
    fn returns_none_for_missing_executable() {
        let binary_name = "codex-shell-command-test-definitely-missing-executable";

        assert_eq!(resolve_executable_in_path(binary_name), None);
    }
}
