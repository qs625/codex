use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use std::collections::HashMap;
use std::sync::Arc;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

struct CommandApplyPatchEnvironmentBridge {
    inner: Arc<dyn codex_sandboxing_api::ApplyPatchEnvironment>,
}

impl codex_sandboxing_api::ApplyPatchEnvironment for CommandApplyPatchEnvironmentBridge {
    fn environment_id(&self) -> &str {
        self.inner.environment_id()
    }

    fn filesystem(&self) -> Arc<dyn codex_file_system::ExecutorFileSystem> {
        self.inner.filesystem()
    }
}

impl thread_service_api::ThreadRuntimeCapability for TurnContext {
    fn runtime_turn_id(&self) -> String {
        self.turn_id()
    }

    fn can_request_original_image_detail(&self) -> bool {
        self.can_request_original_image_detail()
    }

    fn resolve_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<
        Option<codex_sandboxing_api::ResolvedApplyPatchEnvironment>,
        codex_tool_types::FunctionCallError,
    > {
        self.resolve_apply_patch_environment(environment_id)
    }

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cwd: &codex_utils_absolute_path::AbsolutePathBuf,
    ) -> codex_file_system::FileSystemSandboxContext {
        TurnContext::file_system_sandbox_context(self, additional_permissions, cwd)
    }

    fn single_local_environment_cwd(
        &self,
    ) -> Result<codex_utils_absolute_path::AbsolutePathBuf, codex_tool_types::FunctionCallError>
    {
        TurnContext::single_local_environment_cwd(self)
    }

    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64> {
        TurnContext::default_agent_job_max_runtime_seconds(self)
    }

    fn routes_approval_to_guardian(&self) -> bool {
        approval_service::guardian::routes_approval_to_guardian(
            &self.approval_policy.value(),
            self.config.approvals_reviewer,
        )
    }

    fn shell_environment_policy(&self) -> codex_protocol::config_types::ShellEnvironmentPolicy {
        self.shell_environment_policy.clone()
    }

    fn unified_exec_shell_mode(&self) -> codex_tool_config::UnifiedExecShellMode {
        self.unified_exec_shell_mode()
    }

    fn allow_login_shell(&self) -> bool {
        self.allow_login_shell()
    }

    fn active_network(&self) -> Option<codex_network_proxy_api::SharedNetworkProxyRuntime> {
        self.managed_network()
    }

    fn emit_unified_exec_tty_metric(&self, tty: bool) {
        self.emit_unified_exec_tty_metric(tty);
    }

    fn resolve_exec_command_environment(
        &self,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<
        Option<codex_sandboxing_api::ResolvedExecCommandEnvironment>,
        codex_tool_types::FunctionCallError,
    > {
        self.resolve_exec_command_environment(environment_id, workdir)
            .map(|resolved| {
                resolved.map(|resolved| codex_sandboxing_api::ResolvedExecCommandEnvironment {
                    cwd: resolved.cwd,
                    sandbox_cwd: resolved.sandbox_cwd,
                    environment: resolved.environment,
                    apply_patch_environment: Arc::new(CommandApplyPatchEnvironmentBridge {
                        inner: resolved.apply_patch_environment,
                    }),
                })
            })
    }

}

impl thread_service_api::SessionCodeModeCaller for Session {
    async fn code_mode_stored_values(&self) -> HashMap<String, serde_json::Value> {
        Session::code_mode_stored_values(self).await
    }

    async fn code_mode_replace_stored_values(&self, values: HashMap<String, serde_json::Value>) {
        Session::code_mode_replace_stored_values(self, values).await;
    }

    fn code_mode_allocate_cell_id(&self) -> String {
        Session::code_mode_allocate_cell_id(self)
    }

    async fn code_mode_execute(&self, request: ExecuteRequest) -> Result<RuntimeResponse, String> {
        Session::code_mode_execute(self, request).await
    }

    async fn code_mode_wait(&self, request: WaitRequest) -> Result<WaitOutcome, String> {
        Session::code_mode_wait(self, request).await
    }

    fn record_code_mode_cell_started(
        &self,
        turn: &dyn thread_service_api::ThreadRuntimeCapability,
        runtime_cell_id: &str,
        model_visible_call_id: &str,
        source_js: &str,
    ) {
        Session::record_code_mode_cell_started(
            self,
            turn.runtime_turn_id().as_str(),
            runtime_cell_id,
            model_visible_call_id,
            source_js,
        );
    }

    fn record_code_mode_cell_initial_response(
        &self,
        turn: &dyn thread_service_api::ThreadRuntimeCapability,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        Session::record_code_mode_cell_initial_response(
            self,
            turn.runtime_turn_id().as_str(),
            runtime_cell_id,
            response,
        );
    }

    fn record_code_mode_cell_ended(
        &self,
        turn: &dyn thread_service_api::ThreadRuntimeCapability,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        Session::record_code_mode_cell_ended(
            self,
            turn.runtime_turn_id().as_str(),
            runtime_cell_id,
            response,
        );
    }
}
