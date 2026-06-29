//! Display-only command parsing used by app-server protocol projections.

pub mod bash;
pub mod parse_command;
pub mod powershell;
#[cfg(test)]
mod powershell_parser;
mod shell_detect;
