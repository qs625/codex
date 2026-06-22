use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use codex_plugin_types::PluginHookSource;
use codex_protocol::protocol::HookRunSummary;

use crate::HookConfigLayerStack;
use crate::HookPayload;
use crate::HookResponse;
use crate::events::compact::PostCompactRequest;
use crate::events::compact::PreCompactOutcome;
use crate::events::compact::PreCompactRequest;
use crate::events::compact::StatelessHookOutcome;
use crate::events::permission_request::PermissionRequestOutcome;
use crate::events::permission_request::PermissionRequestRequest;
use crate::events::post_tool_use::PostToolUseOutcome;
use crate::events::post_tool_use::PostToolUseRequest;
use crate::events::pre_tool_use::PreToolUseOutcome;
use crate::events::pre_tool_use::PreToolUseRequest;
use crate::events::session_start::SessionStartOutcome;
use crate::events::session_start::SessionStartRequest;
use crate::events::stop::StopOutcome;
use crate::events::stop::StopRequest;
use crate::events::user_prompt_submit::UserPromptSubmitOutcome;
use crate::events::user_prompt_submit::UserPromptSubmitRequest;

pub type HookFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Default, Clone)]
pub struct HooksConfig {
    pub legacy_notify_argv: Option<Vec<String>>,
    pub feature_enabled: bool,
    pub bypass_hook_trust: bool,
    pub config_layer_stack: Option<HookConfigLayerStack>,
    pub plugin_hook_sources: Vec<PluginHookSource>,
    pub plugin_hook_load_warnings: Vec<String>,
    pub shell_program: Option<String>,
    pub shell_args: Vec<String>,
}

/// Runtime capability for previewing and executing configured hooks.
///
/// Implementations own command discovery and process execution. Consumers such
/// as `codex-core` should depend on this trait so they can run hooks without
/// directly depending on a concrete hook engine.
pub trait HookRuntime: Send + Sync {
    fn startup_warnings(&self) -> &[String];

    fn dispatch(&self, hook_payload: HookPayload) -> HookFuture<'_, Vec<HookResponse>>;

    fn preview_session_start(&self, request: &SessionStartRequest) -> Vec<HookRunSummary>;

    fn preview_pre_tool_use(&self, request: &PreToolUseRequest) -> Vec<HookRunSummary>;

    fn preview_permission_request(&self, request: &PermissionRequestRequest)
    -> Vec<HookRunSummary>;

    fn preview_post_tool_use(&self, request: &PostToolUseRequest) -> Vec<HookRunSummary>;

    fn preview_pre_compact(&self, request: &PreCompactRequest) -> Vec<HookRunSummary>;

    fn preview_post_compact(&self, request: &PostCompactRequest) -> Vec<HookRunSummary>;

    fn preview_user_prompt_submit(&self, request: &UserPromptSubmitRequest) -> Vec<HookRunSummary>;

    fn preview_stop(&self, request: &StopRequest) -> Vec<HookRunSummary>;

    fn run_session_start(
        &self,
        request: SessionStartRequest,
        turn_id: Option<String>,
    ) -> HookFuture<'_, SessionStartOutcome>;

    fn run_pre_tool_use(&self, request: PreToolUseRequest) -> HookFuture<'_, PreToolUseOutcome>;

    fn run_permission_request(
        &self,
        request: PermissionRequestRequest,
    ) -> HookFuture<'_, PermissionRequestOutcome>;

    fn run_post_tool_use(&self, request: PostToolUseRequest) -> HookFuture<'_, PostToolUseOutcome>;

    fn run_pre_compact(&self, request: PreCompactRequest) -> HookFuture<'_, PreCompactOutcome>;

    fn run_post_compact(&self, request: PostCompactRequest)
    -> HookFuture<'_, StatelessHookOutcome>;

    fn run_user_prompt_submit(
        &self,
        request: UserPromptSubmitRequest,
    ) -> HookFuture<'_, UserPromptSubmitOutcome>;

    fn run_stop(&self, request: StopRequest) -> HookFuture<'_, StopOutcome>;
}

pub type SharedHookRuntime = Arc<dyn HookRuntime>;

/// Factory for constructing a session-scoped hook runtime from one config
/// snapshot.
///
/// The factory boundary keeps concrete hook command execution out of session
/// orchestration crates while still allowing hooks to refresh when config
/// changes.
pub trait HookRuntimeFactory: Send + Sync {
    fn create(&self, config: HooksConfig) -> SharedHookRuntime;
}

pub type SharedHookRuntimeFactory = Arc<dyn HookRuntimeFactory>;

pub struct DisabledHookRuntime;

impl HookRuntime for DisabledHookRuntime {
    fn startup_warnings(&self) -> &[String] {
        &[]
    }

    fn dispatch(&self, _hook_payload: HookPayload) -> HookFuture<'_, Vec<HookResponse>> {
        Box::pin(async { Vec::new() })
    }

    fn preview_session_start(&self, _request: &SessionStartRequest) -> Vec<HookRunSummary> {
        Vec::new()
    }

    fn preview_pre_tool_use(&self, _request: &PreToolUseRequest) -> Vec<HookRunSummary> {
        Vec::new()
    }

    fn preview_permission_request(
        &self,
        _request: &PermissionRequestRequest,
    ) -> Vec<HookRunSummary> {
        Vec::new()
    }

    fn preview_post_tool_use(&self, _request: &PostToolUseRequest) -> Vec<HookRunSummary> {
        Vec::new()
    }

    fn preview_pre_compact(&self, _request: &PreCompactRequest) -> Vec<HookRunSummary> {
        Vec::new()
    }

    fn preview_post_compact(&self, _request: &PostCompactRequest) -> Vec<HookRunSummary> {
        Vec::new()
    }

    fn preview_user_prompt_submit(
        &self,
        _request: &UserPromptSubmitRequest,
    ) -> Vec<HookRunSummary> {
        Vec::new()
    }

    fn preview_stop(&self, _request: &StopRequest) -> Vec<HookRunSummary> {
        Vec::new()
    }

    fn run_session_start(
        &self,
        _request: SessionStartRequest,
        _turn_id: Option<String>,
    ) -> HookFuture<'_, SessionStartOutcome> {
        Box::pin(async {
            SessionStartOutcome {
                hook_events: Vec::new(),
                should_stop: false,
                stop_reason: None,
                additional_contexts: Vec::new(),
            }
        })
    }

    fn run_pre_tool_use(&self, _request: PreToolUseRequest) -> HookFuture<'_, PreToolUseOutcome> {
        Box::pin(async {
            PreToolUseOutcome {
                hook_events: Vec::new(),
                should_block: false,
                block_reason: None,
                additional_contexts: Vec::new(),
                updated_input: None,
            }
        })
    }

    fn run_permission_request(
        &self,
        _request: PermissionRequestRequest,
    ) -> HookFuture<'_, PermissionRequestOutcome> {
        Box::pin(async {
            PermissionRequestOutcome {
                hook_events: Vec::new(),
                decision: None,
            }
        })
    }

    fn run_post_tool_use(
        &self,
        _request: PostToolUseRequest,
    ) -> HookFuture<'_, PostToolUseOutcome> {
        Box::pin(async {
            PostToolUseOutcome {
                hook_events: Vec::new(),
                should_stop: false,
                stop_reason: None,
                additional_contexts: Vec::new(),
                feedback_message: None,
            }
        })
    }

    fn run_pre_compact(&self, _request: PreCompactRequest) -> HookFuture<'_, PreCompactOutcome> {
        Box::pin(async {
            PreCompactOutcome {
                hook_events: Vec::new(),
                should_stop: false,
                stop_reason: None,
            }
        })
    }

    fn run_post_compact(
        &self,
        _request: PostCompactRequest,
    ) -> HookFuture<'_, StatelessHookOutcome> {
        Box::pin(async {
            StatelessHookOutcome {
                hook_events: Vec::new(),
                should_stop: false,
                stop_reason: None,
            }
        })
    }

    fn run_user_prompt_submit(
        &self,
        _request: UserPromptSubmitRequest,
    ) -> HookFuture<'_, UserPromptSubmitOutcome> {
        Box::pin(async {
            UserPromptSubmitOutcome {
                hook_events: Vec::new(),
                should_stop: false,
                stop_reason: None,
                additional_contexts: Vec::new(),
            }
        })
    }

    fn run_stop(&self, _request: StopRequest) -> HookFuture<'_, StopOutcome> {
        Box::pin(async {
            StopOutcome {
                hook_events: Vec::new(),
                should_stop: false,
                stop_reason: None,
                should_block: false,
                block_reason: None,
                continuation_fragments: Vec::new(),
            }
        })
    }
}

pub struct DisabledHookRuntimeFactory;

impl HookRuntimeFactory for DisabledHookRuntimeFactory {
    fn create(&self, _config: HooksConfig) -> SharedHookRuntime {
        Arc::new(DisabledHookRuntime)
    }
}
