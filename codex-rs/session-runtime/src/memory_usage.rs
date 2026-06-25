use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::flat_tool_name;
use crate::unified_exec::ExecCommandArgs;
use codex_protocol::models::ShellCommandToolCallParams;
use codex_protocol::parse_command::ParsedCommand;
use codex_shell_command::is_safe_command::is_known_safe_command;
use codex_shell_command::parse_command::parse_command;
use codex_tool_runtime_api::resolve_exec_command_for_parts;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MemoriesUsageKind {
    MemoryMd,
    MemorySummary,
    RawMemories,
    RolloutSummaries,
    Skills,
}

impl MemoriesUsageKind {
    fn as_tag(self) -> &'static str {
        match self {
            Self::MemoryMd => "memory_md",
            Self::MemorySummary => "memory_summary",
            Self::RawMemories => "raw_memories",
            Self::RolloutSummaries => "rollout_summaries",
            Self::Skills => "skills",
        }
    }
}

pub(crate) async fn emit_metric_for_tool_read(invocation: &ToolInvocation, success: bool) {
    let Some((command, _)) = shell_command_for_invocation(invocation) else {
        return;
    };
    let kinds = memories_usage_kinds_from_command(&command);
    if kinds.is_empty() {
        return;
    }

    let success = if success { "true" } else { "false" };
    let tool_name = flat_tool_name(&invocation.tool_name);
    for kind in kinds {
        invocation
            .turn
            .emit_memories_usage_metric(kind.as_tag(), tool_name.as_ref(), success);
    }
}

fn memories_usage_kinds_from_command(command: &[String]) -> Vec<MemoriesUsageKind> {
    if !is_known_safe_command(command) {
        return Vec::new();
    }

    parse_command(command)
        .into_iter()
        .filter_map(|command| match command {
            ParsedCommand::Read { path, .. } => get_memory_kind(path.display().to_string()),
            ParsedCommand::Search { path, .. } => path.and_then(get_memory_kind),
            ParsedCommand::ListFiles { .. } | ParsedCommand::Unknown { .. } => None,
        })
        .collect()
}

fn get_memory_kind(path: String) -> Option<MemoriesUsageKind> {
    if path.contains("memories/MEMORY.md") {
        Some(MemoriesUsageKind::MemoryMd)
    } else if path.contains("memories/memory_summary.md") {
        Some(MemoriesUsageKind::MemorySummary)
    } else if path.contains("memories/raw_memories.md") {
        Some(MemoriesUsageKind::RawMemories)
    } else if path.contains("memories/rollout_summaries/") {
        Some(MemoriesUsageKind::RolloutSummaries)
    } else if path.contains("memories/skills/") {
        Some(MemoriesUsageKind::Skills)
    } else {
        None
    }
}

fn shell_command_for_invocation(invocation: &ToolInvocation) -> Option<(Vec<String>, PathBuf)> {
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return None;
    };

    match (
        invocation.tool_name.namespace.as_deref(),
        invocation.tool_name.name.as_str(),
    ) {
        (None, "shell_command") => serde_json::from_str::<ShellCommandToolCallParams>(arguments)
            .ok()
            .map(|params| {
                let allow_login_shell = invocation.turn.allow_login_shell();
                if !allow_login_shell && params.login == Some(true) {
                    let cwd = invocation
                        .turn
                        .resolve_shell_workdir(params.workdir)
                        .to_path_buf();
                    return (Vec::new(), cwd);
                }
                let use_login_shell = params.login.unwrap_or(allow_login_shell);
                let command = invocation
                    .session
                    .derive_shell_exec_args(&params.command, use_login_shell);
                let cwd = invocation
                    .turn
                    .resolve_shell_workdir(params.workdir)
                    .to_path_buf();
                (command, cwd)
            }),
        (None, "exec_command") => serde_json::from_str::<ExecCommandArgs>(arguments)
            .ok()
            .and_then(|params| {
                let model_shell = params.shell.as_ref().map(|shell| {
                    let mut shell =
                        crate::shell::get_shell_by_model_provided_path(&PathBuf::from(shell));
                    shell.shell_snapshot = crate::shell::empty_shell_snapshot_receiver();
                    crate::tools::runtimes::runtime_shell(&shell)
                });
                let resolved = resolve_exec_command_for_parts(
                    params.cmd.as_str(),
                    params.login,
                    &invocation.session.runtime_shell(),
                    model_shell.as_ref(),
                    &invocation.turn.unified_exec_shell_mode(),
                    invocation.turn.allow_login_shell(),
                )
                .ok()?;
                let cwd = invocation
                    .turn
                    .resolve_shell_workdir(params.workdir)
                    .to_path_buf();
                Some((resolved.command, cwd))
            }),
        (Some(_), _) | (None, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn classifies_safe_memory_reads() {
        let kinds =
            memories_usage_kinds_from_command(&command(&["cat", "/tmp/codex/memories/MEMORY.md"]));

        assert_eq!(kinds, vec![MemoriesUsageKind::MemoryMd]);
    }

    #[test]
    fn ignores_commands_that_are_not_known_safe() {
        let kinds =
            memories_usage_kinds_from_command(&command(&["rm", "/tmp/codex/memories/MEMORY.md"]));

        assert_eq!(kinds, Vec::new());
    }
}
