use std::sync::Arc;

use codex_hooks_api::HookFuture;
use codex_hooks_api::HookListOutcome;
use codex_hooks_api::HookPayload;
use codex_hooks_api::HookResponse;
use codex_hooks_api::HookRuntime;
use codex_hooks_api::HookRuntimeFactory;
use codex_hooks_api::HooksConfig;
use codex_hooks_api::SharedHookRuntime;
use tokio::process::Command;

use crate::engine::ClaudeHooksEngine;
use crate::engine::CommandShell;
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
use codex_hooks_api::Hook;
use codex_hooks_api::HookEvent;

#[derive(Clone)]
pub struct Hooks {
    after_agent: Vec<Hook>,
    engine: ClaudeHooksEngine,
}

impl Default for Hooks {
    fn default() -> Self {
        Self::new(HooksConfig::default())
    }
}

impl Hooks {
    pub fn new(config: HooksConfig) -> Self {
        let after_agent = config
            .legacy_notify_argv
            .filter(|argv| !argv.is_empty() && !argv[0].is_empty())
            .map(crate::notify_hook)
            .into_iter()
            .collect();
        let engine = ClaudeHooksEngine::new(
            config.feature_enabled,
            config.bypass_hook_trust,
            config.config_layer_stack.as_ref(),
            config.plugin_hook_sources,
            config.plugin_hook_load_warnings,
            CommandShell {
                program: config.shell_program.unwrap_or_default(),
                args: config.shell_args,
            },
        );
        Self {
            after_agent,
            engine,
        }
    }

    pub fn startup_warnings(&self) -> &[String] {
        self.engine.warnings()
    }

    fn hooks_for_event(&self, hook_event: &HookEvent) -> &[Hook] {
        match hook_event {
            HookEvent::AfterAgent { .. } => &self.after_agent,
        }
    }

    pub async fn dispatch(&self, hook_payload: HookPayload) -> Vec<HookResponse> {
        let hooks = self.hooks_for_event(&hook_payload.hook_event);
        let mut outcomes = Vec::with_capacity(hooks.len());
        for hook in hooks {
            let outcome = hook.execute(&hook_payload).await;
            let should_abort_operation = outcome.result.should_abort_operation();
            outcomes.push(outcome);
            if should_abort_operation {
                break;
            }
        }

        outcomes
    }

    pub fn preview_session_start(
        &self,
        request: &SessionStartRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_session_start(request)
    }

    pub fn preview_pre_tool_use(
        &self,
        request: &PreToolUseRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_pre_tool_use(request)
    }

    pub fn preview_permission_request(
        &self,
        request: &PermissionRequestRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_permission_request(request)
    }

    pub fn preview_post_tool_use(
        &self,
        request: &PostToolUseRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_post_tool_use(request)
    }

    pub async fn run_session_start(
        &self,
        request: SessionStartRequest,
        turn_id: Option<String>,
    ) -> SessionStartOutcome {
        self.engine.run_session_start(request, turn_id).await
    }

    pub async fn run_pre_tool_use(&self, request: PreToolUseRequest) -> PreToolUseOutcome {
        self.engine.run_pre_tool_use(request).await
    }

    pub async fn run_permission_request(
        &self,
        request: PermissionRequestRequest,
    ) -> PermissionRequestOutcome {
        self.engine.run_permission_request(request).await
    }

    pub async fn run_post_tool_use(&self, request: PostToolUseRequest) -> PostToolUseOutcome {
        self.engine.run_post_tool_use(request).await
    }

    pub fn preview_pre_compact(
        &self,
        request: &PreCompactRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_pre_compact(request)
    }

    pub async fn run_pre_compact(&self, request: PreCompactRequest) -> PreCompactOutcome {
        self.engine.run_pre_compact(request).await
    }

    pub fn preview_post_compact(
        &self,
        request: &PostCompactRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_post_compact(request)
    }

    pub async fn run_post_compact(&self, request: PostCompactRequest) -> StatelessHookOutcome {
        self.engine.run_post_compact(request).await
    }

    pub fn preview_user_prompt_submit(
        &self,
        request: &UserPromptSubmitRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_user_prompt_submit(request)
    }

    pub async fn run_user_prompt_submit(
        &self,
        request: UserPromptSubmitRequest,
    ) -> UserPromptSubmitOutcome {
        self.engine.run_user_prompt_submit(request).await
    }

    pub fn preview_stop(
        &self,
        request: &StopRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_stop(request)
    }

    pub async fn run_stop(&self, request: StopRequest) -> StopOutcome {
        self.engine.run_stop(request).await
    }
}

impl HookRuntime for Hooks {
    fn startup_warnings(&self) -> &[String] {
        Hooks::startup_warnings(self)
    }

    fn dispatch(&self, hook_payload: HookPayload) -> HookFuture<'_, Vec<HookResponse>> {
        Box::pin(Hooks::dispatch(self, hook_payload))
    }

    fn preview_session_start(
        &self,
        request: &SessionStartRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        Hooks::preview_session_start(self, request)
    }

    fn preview_pre_tool_use(
        &self,
        request: &PreToolUseRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        Hooks::preview_pre_tool_use(self, request)
    }

    fn preview_permission_request(
        &self,
        request: &PermissionRequestRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        Hooks::preview_permission_request(self, request)
    }

    fn preview_post_tool_use(
        &self,
        request: &PostToolUseRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        Hooks::preview_post_tool_use(self, request)
    }

    fn preview_pre_compact(
        &self,
        request: &PreCompactRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        Hooks::preview_pre_compact(self, request)
    }

    fn preview_post_compact(
        &self,
        request: &PostCompactRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        Hooks::preview_post_compact(self, request)
    }

    fn preview_user_prompt_submit(
        &self,
        request: &UserPromptSubmitRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        Hooks::preview_user_prompt_submit(self, request)
    }

    fn preview_stop(&self, request: &StopRequest) -> Vec<codex_protocol::protocol::HookRunSummary> {
        Hooks::preview_stop(self, request)
    }

    fn run_session_start(
        &self,
        request: SessionStartRequest,
        turn_id: Option<String>,
    ) -> HookFuture<'_, SessionStartOutcome> {
        Box::pin(Hooks::run_session_start(self, request, turn_id))
    }

    fn run_pre_tool_use(&self, request: PreToolUseRequest) -> HookFuture<'_, PreToolUseOutcome> {
        Box::pin(Hooks::run_pre_tool_use(self, request))
    }

    fn run_permission_request(
        &self,
        request: PermissionRequestRequest,
    ) -> HookFuture<'_, PermissionRequestOutcome> {
        Box::pin(Hooks::run_permission_request(self, request))
    }

    fn run_post_tool_use(&self, request: PostToolUseRequest) -> HookFuture<'_, PostToolUseOutcome> {
        Box::pin(Hooks::run_post_tool_use(self, request))
    }

    fn run_pre_compact(&self, request: PreCompactRequest) -> HookFuture<'_, PreCompactOutcome> {
        Box::pin(Hooks::run_pre_compact(self, request))
    }

    fn run_post_compact(
        &self,
        request: PostCompactRequest,
    ) -> HookFuture<'_, StatelessHookOutcome> {
        Box::pin(Hooks::run_post_compact(self, request))
    }

    fn run_user_prompt_submit(
        &self,
        request: UserPromptSubmitRequest,
    ) -> HookFuture<'_, UserPromptSubmitOutcome> {
        Box::pin(Hooks::run_user_prompt_submit(self, request))
    }

    fn run_stop(&self, request: StopRequest) -> HookFuture<'_, StopOutcome> {
        Box::pin(Hooks::run_stop(self, request))
    }
}

pub struct HooksRuntimeFactory;

impl HookRuntimeFactory for HooksRuntimeFactory {
    fn create(&self, config: HooksConfig) -> SharedHookRuntime {
        Arc::new(Hooks::new(config))
    }
}

pub fn list_hooks(config: HooksConfig) -> HookListOutcome {
    if !config.feature_enabled {
        return HookListOutcome::default();
    }

    let discovered = crate::engine::discovery::discover_handlers(
        config.config_layer_stack.as_ref(),
        config.plugin_hook_sources,
        config.plugin_hook_load_warnings,
        config.bypass_hook_trust,
    );
    HookListOutcome {
        hooks: discovered.hook_entries,
        warnings: discovered.warnings,
    }
}

pub fn command_from_argv(argv: &[String]) -> Option<Command> {
    let (program, args) = argv.split_first()?;
    if program.is_empty() {
        return None;
    }
    let mut command = Command::new(program);
    command.args(args);
    Some(command)
}
