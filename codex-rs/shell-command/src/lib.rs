//! Command parsing and safety utilities shared across Codex crates.

mod shell_detect;

mod approval_canonicalization;
pub mod bash;
pub(crate) mod command_safety;
pub mod parse_command;
pub mod powershell;

pub use approval_canonicalization::canonicalize_command_for_approval;
pub use command_safety::is_dangerous_command;
pub use command_safety::is_safe_command;
