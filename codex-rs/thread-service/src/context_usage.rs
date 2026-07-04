use crate::TurnContext;
use codex_context_manager::ContextManager;
use codex_context_usage::ContextUsageSkillDetection;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::ThreadSkill;

pub(crate) fn build_thread_context_usage(
    history: &ContextManager,
    turn_context: &TurnContext,
    thread_skills: &[ThreadSkill],
) -> ThreadContextUsage {
    let total_skills = turn_context
        .turn_skills
        .outcome
        .skills_with_enabled()
        .filter(|(_, enabled)| *enabled)
        .count();
    codex_context_usage::build_thread_context_usage(
        history,
        thread_skills,
        Some(ContextUsageSkillDetection {
            outcome: &turn_context.turn_skills.outcome,
            cwd: selected_turn_cwd(turn_context),
            total_count: Some(u32::try_from(total_skills).unwrap_or(u32::MAX)),
        }),
        crate::compact::is_summary_message,
    )
}

pub(crate) fn build_thread_context_usage_from_history(
    history: &ContextManager,
    thread_skills: &[ThreadSkill],
) -> ThreadContextUsage {
    codex_context_usage::build_thread_context_usage_from_history(
        history,
        thread_skills,
        crate::compact::is_summary_message,
    )
}

fn selected_turn_cwd(turn_context: &TurnContext) -> &AbsolutePathBuf {
    turn_context
        .environments
        .turn_environments
        .first()
        .map(|turn_environment| &turn_environment.cwd)
        .unwrap_or(&turn_context.config.cwd)
}
