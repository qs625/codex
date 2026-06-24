use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::flat_tool_name;
use crate::unified_exec::ExecCommandArgs;
use codex_protocol::models::ShellCommandToolCallParams;
use codex_protocol::parse_command::ParsedCommand;
use codex_shell_command::is_safe_command::is_known_safe_command;
use codex_shell_command::parse_command::parse_command;
use std::path::PathBuf;

const MEMORIES_USAGE_METRIC: &str = "codex_turn_memories_usage";

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
        invocation.turn.session_telemetry.counter(
            MEMORIES_USAGE_METRIC,
            /*inc*/ 1,
            &[
                ("kind", kind.as_tag()),
                ("tool", tool_name.as_ref()),
                ("success", success),
            ],
        );
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
                if !invocation.turn.tools_config.allow_login_shell && params.login == Some(true) {
                    #[allow(deprecated)]
                    let cwd = invocation.turn.resolve_path(params.workdir).to_path_buf();
                    return (Vec::new(), cwd);
                }
                let use_login_shell = params
                    .login
                    .unwrap_or(invocation.turn.tools_config.allow_login_shell);
                let command = invocation
                    .session
                    .user_shell()
                    .derive_exec_args(&params.command, use_login_shell);
                #[allow(deprecated)]
                let cwd = invocation.turn.resolve_path(params.workdir).to_path_buf();
                (command, cwd)
            }),
        (None, "exec_command") => serde_json::from_str::<ExecCommandArgs>(arguments)
            .ok()
            .and_then(|params| {
                let command = crate::unified_exec::get_command(
                    &params,
                    invocation.session.user_shell(),
                    &invocation.turn.tools_config.unified_exec_shell_mode,
                    invocation.turn.tools_config.allow_login_shell,
                )
                .ok()?;
                #[allow(deprecated)]
                let cwd = invocation.turn.resolve_path(params.workdir).to_path_buf();
                Some((command.command, cwd))
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
