use protocol::exec_output::ExecToolCallOutput;
use protocol::models::ResponseItem;

use crate::session::turn_context::TurnContext;
use crate::tool_output_utils::format_exec_output_str;
use codex_context_manager::ContextualUserFragment;
use codex_context_manager::UserShellCommand;

fn user_shell_command_fragment(
    command: &str,
    exec_output: &ExecToolCallOutput,
    turn_context: &TurnContext,
) -> UserShellCommand {
    let output = format_exec_output_str(exec_output, turn_context.truncation_policy);
    UserShellCommand::new(command, exec_output.exit_code, exec_output.duration, output)
}

#[cfg(test)]
pub fn format_user_shell_command_record(
    command: &str,
    exec_output: &ExecToolCallOutput,
    turn_context: &TurnContext,
) -> String {
    user_shell_command_fragment(command, exec_output, turn_context).render()
}

pub fn user_shell_command_record_item(
    command: &str,
    exec_output: &ExecToolCallOutput,
    turn_context: &TurnContext,
) -> ResponseItem {
    ContextualUserFragment::into(user_shell_command_fragment(
        command,
        exec_output,
        turn_context,
    ))
}

#[cfg(test)]
#[path = "user_shell_command_tests.rs"]
mod tests;
