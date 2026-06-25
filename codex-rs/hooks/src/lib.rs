mod config_rules;
mod engine;
pub(crate) mod events;
mod legacy_notify;
mod output_spill;
mod registry;
mod runtime;
mod schema;

pub use codex_hooks_api::DisabledHookRuntime;
pub use codex_hooks_api::DisabledHookRuntimeFactory;
pub use codex_hooks_api::Hook;
pub use codex_hooks_api::HookConfigLayerEntry;
pub use codex_hooks_api::HookConfigLayerStack;
pub use codex_hooks_api::HookEvent;
pub use codex_hooks_api::HookEventAfterAgent;
pub use codex_hooks_api::HookFn;
pub use codex_hooks_api::HookFuture;
pub use codex_hooks_api::HookListEntry;
pub use codex_hooks_api::HookListOutcome;
pub use codex_hooks_api::HookManagedHooksRequirement;
pub use codex_hooks_api::HookPayload;
pub use codex_hooks_api::HookResponse;
pub use codex_hooks_api::HookResult;
pub use codex_hooks_api::HookRuntime;
pub use codex_hooks_api::HookRuntimeFactory;
pub use codex_hooks_api::HooksConfig;
pub use codex_hooks_api::PluginHookDeclaration;
pub use codex_hooks_api::SharedHookRuntime;
pub use codex_hooks_api::SharedHookRuntimeFactory;
pub use codex_hooks_api::hook_event_key_label;
pub use codex_hooks_api::hook_key;
pub use codex_hooks_api::plugin_hook_declarations;
pub use codex_hooks_api::plugin_hook_key_source;
pub use config_rules::hook_states_from_stack;
/// Hook event names as they appear in hooks JSON and config files.
pub const HOOK_EVENT_NAMES: [&str; 8] = [
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SessionStart",
    "UserPromptSubmit",
    "Stop",
];

/// Hook event names whose matcher fields are meaningful during dispatch.
///
/// Other events can appear in hooks JSON, but Codex ignores their matcher
/// fields because those events do not dispatch against a tool, compaction
/// trigger, or session-start source.
pub const HOOK_EVENT_NAMES_WITH_MATCHERS: [&str; 6] = [
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SessionStart",
];

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
pub use legacy_notify::legacy_notify_json;
pub use legacy_notify::notify_hook;
pub use registry::Hooks;
pub use registry::HooksRuntimeFactory;
pub use registry::command_from_argv;
pub use registry::list_hooks;
pub use runtime::HookRuntimeContext;
pub use runtime::HookRuntimeHost;
pub use runtime::HookRuntimeOutcome;
pub use runtime::HookRuntimeTurn;
pub use runtime::PendingInputHookDisposition;
pub use runtime::PendingInputRecord;
pub use runtime::PermissionRequestHookPayload;
pub use runtime::PostCompactHookOutcome;
pub use runtime::PreCompactHookOutcome;
pub use runtime::PreToolUseHookResult;
pub use runtime::emit_hook_completed_events;
pub use runtime::inspect_pending_input;
pub use runtime::record_additional_contexts;
pub use runtime::record_pending_input;
pub use runtime::run_pending_session_start_hooks;
pub use runtime::run_permission_request_hooks;
pub use runtime::run_post_compact_hooks;
pub use runtime::run_post_tool_use_hooks;
pub use runtime::run_pre_compact_hooks;
pub use runtime::run_pre_tool_use_hooks;
pub use runtime::run_user_prompt_submit_hooks;
pub use schema::write_schema_fixtures;
