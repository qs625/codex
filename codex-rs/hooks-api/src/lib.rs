//! Lightweight API helpers for Codex hook metadata.
//!
//! This crate owns hook declaration projection and persisted hook-key helpers
//! that do not need the hook command execution runtime.

mod config_layers;
mod declarations;
mod event_projection;
pub mod events;
mod list;
mod runtime;
mod types;

use codex_protocol::protocol::HookEventName;

pub use config_layers::HookConfigLayerEntry;
pub use config_layers::HookConfigLayerStack;
pub use config_layers::HookConfigLayerStackOrdering;
pub use config_layers::HookManagedHooksRequirement;
pub use declarations::PluginHookDeclaration;
pub use declarations::plugin_hook_declarations;
pub use declarations::plugin_hook_key_source;
pub use event_projection::hook_events_into_matcher_groups;
pub use events::compact::PostCompactRequest;
pub use events::compact::PreCompactOutcome;
pub use events::compact::PreCompactRequest;
pub use events::compact::StatelessHookOutcome;
pub use events::permission_request::PermissionRequestDecision;
pub use events::permission_request::PermissionRequestOutcome;
pub use events::permission_request::PermissionRequestRequest;
pub use events::post_tool_use::PostToolUseOutcome;
pub use events::post_tool_use::PostToolUseRequest;
pub use events::pre_tool_use::PreToolUseOutcome;
pub use events::pre_tool_use::PreToolUseRequest;
pub use events::session_start::SessionStartOutcome;
pub use events::session_start::SessionStartRequest;
pub use events::session_start::SessionStartSource;
pub use events::stop::StopOutcome;
pub use events::stop::StopRequest;
pub use events::user_prompt_submit::UserPromptSubmitOutcome;
pub use events::user_prompt_submit::UserPromptSubmitRequest;
pub use list::HookListEntry;
pub use list::HookListOutcome;
pub use runtime::DisabledHookRuntime;
pub use runtime::DisabledHookRuntimeFactory;
pub use runtime::HookFuture;
pub use runtime::HookRuntime;
pub use runtime::HookRuntimeFactory;
pub use runtime::HooksConfig;
pub use runtime::SharedHookRuntime;
pub use runtime::SharedHookRuntimeFactory;
pub use types::Hook;
pub use types::HookEvent;
pub use types::HookEventAfterAgent;
pub use types::HookFn;
pub use types::HookPayload;
pub use types::HookResponse;
pub use types::HookResult;

/// Returns the hook event label used in persisted hook-state keys.
pub fn hook_event_key_label(event_name: HookEventName) -> &'static str {
    match event_name {
        HookEventName::PreToolUse => "pre_tool_use",
        HookEventName::PermissionRequest => "permission_request",
        HookEventName::PostToolUse => "post_tool_use",
        HookEventName::PreCompact => "pre_compact",
        HookEventName::PostCompact => "post_compact",
        HookEventName::SessionStart => "session_start",
        HookEventName::UserPromptSubmit => "user_prompt_submit",
        HookEventName::Stop => "stop",
    }
}

/// Builds the persisted config-state key for one discovered hook handler.
pub fn hook_key(
    key_source: &str,
    event_name: HookEventName,
    group_index: usize,
    handler_index: usize,
) -> String {
    format!(
        "{key_source}:{}:{group_index}:{handler_index}",
        hook_event_key_label(event_name)
    )
}
