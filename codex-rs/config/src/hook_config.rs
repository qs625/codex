pub use codex_config_types::HookEventsToml;
pub use codex_config_types::HookHandlerConfig;
pub use codex_config_types::HookStateToml;
pub use codex_config_types::HooksFile;
pub use codex_config_types::HooksToml;
pub use codex_config_types::ManagedHooksRequirementsToml;
pub use codex_config_types::MatcherGroup;
use codex_protocol::protocol::HookEventName;

pub fn hook_events_into_matcher_groups(
    hook_events: HookEventsToml,
) -> [(HookEventName, Vec<MatcherGroup>); 8] {
    [
        (HookEventName::PreToolUse, hook_events.pre_tool_use),
        (
            HookEventName::PermissionRequest,
            hook_events.permission_request,
        ),
        (HookEventName::PostToolUse, hook_events.post_tool_use),
        (HookEventName::PreCompact, hook_events.pre_compact),
        (HookEventName::PostCompact, hook_events.post_compact),
        (HookEventName::SessionStart, hook_events.session_start),
        (
            HookEventName::UserPromptSubmit,
            hook_events.user_prompt_submit,
        ),
        (HookEventName::Stop, hook_events.stop),
    ]
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
