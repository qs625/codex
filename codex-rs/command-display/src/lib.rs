//! Command display parsing shared by runtime and app-server presentation crates.
//!
//! This crate owns lossy command metadata extraction for UI/display surfaces. Shell
//! safety, approval canonicalization, process lookup, and execution helpers stay in
//! `codex-shell-command`.

pub mod bash;
pub mod parse_command;
pub mod powershell;
mod powershell_parser;
mod shell_detect;
