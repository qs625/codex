use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tool_output_utils::flat_tool_name;
use codex_command_service_api::ExecCommandArgs;
use codex_command_service_api::resolve_exec_command_for_parts;
use codex_protocol::parse_command::ParsedCommand;
use codex_shell_utils::is_safe_command::is_known_safe_command;
use codex_shell_utils::parse_command::parse_command;
use codex_tool_types::ToolName;
use codex_tool_types::ToolPayload;
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

pub(crate) async fn emit_metric_for_tool_read_parts(
    session: &Session,
    turn: &TurnContext,
    tool_name: &ToolName,
    payload: &ToolPayload,
    success: bool,
) {
    let Some((command, _)) = exec_like_command_for_parts(session, turn, tool_name, payload) else {
        return;
    };
    let kinds = memories_usage_kinds_from_command(&command);
    if kinds.is_empty() {
        return;
    }

    let success = if success { "true" } else { "false" };
    let tool_name = flat_tool_name(tool_name);
    for kind in kinds {
        turn.emit_memories_usage_metric(kind.as_tag(), tool_name.as_ref(), success);
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

fn exec_like_command_for_parts(
    session: &Session,
    turn: &TurnContext,
    tool_name: &ToolName,
    payload: &ToolPayload,
) -> Option<(Vec<String>, PathBuf)> {
    let ToolPayload::Function { arguments } = payload else {
        return None;
    };

    match (tool_name.namespace.as_deref(), tool_name.name.as_str()) {
        (None, "exec_command") => serde_json::from_str::<ExecCommandArgs>(arguments)
            .ok()
            .and_then(|params| {
                let model_shell = params.shell.as_ref().map(|shell| {
                    let mut shell = crate::runtime_shell_model::get_shell_by_model_provided_path(
                        &PathBuf::from(shell),
                    );
                    shell.shell_snapshot =
                        crate::runtime_shell_model::empty_shell_snapshot_receiver();
                    shell.to_runtime_shell()
                });
                let session_shell = session.user_shell().as_ref().to_runtime_shell();
                let resolved = resolve_exec_command_for_parts(
                    params.cmd.as_str(),
                    params.login,
                    &session_shell,
                    model_shell.as_ref(),
                    &turn.unified_exec_shell_mode(),
                    turn.allow_login_shell(),
                )
                .ok()?;
                let cwd = turn.resolve_shell_workdir(params.workdir).to_path_buf();
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
