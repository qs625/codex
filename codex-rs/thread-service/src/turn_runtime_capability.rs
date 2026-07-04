use std::sync::Arc;

use crate::session::turn_context::TurnContext;
use codex_approval_service_api::routes_approval_to_guardian;

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
        tool_service_api::FunctionCallError,
    > {
        self.resolve_apply_patch_environment(environment_id)
    }

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
        cwd: &codex_utils_absolute_path::AbsolutePathBuf,
    ) -> codex_file_system::FileSystemSandboxContext {
        TurnContext::file_system_sandbox_context(self, additional_permissions, cwd)
    }

    fn single_local_environment_cwd(
        &self,
    ) -> Result<codex_utils_absolute_path::AbsolutePathBuf, tool_service_api::FunctionCallError>
    {
        TurnContext::single_local_environment_cwd(self)
    }

    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64> {
        TurnContext::default_agent_job_max_runtime_seconds(self)
    }

    fn routes_approval_to_guardian(&self) -> bool {
        routes_approval_to_guardian(
            &self.approval_policy.value(),
            self.config.approvals_reviewer,
        )
    }

    fn current_exec_policy(&self) -> std::sync::Arc<permissions_service_api::Policy> {
        self.session_arc().services.exec_policy.current()
    }

    fn shell_environment_policy(&self) -> protocol::config_types::ShellEnvironmentPolicy {
        self.shell_environment_policy.clone()
    }

    fn runtime_shell(&self) -> thread_service_api::RuntimeShell {
        self.session_arc().runtime_shell()
    }

    fn tool_user_shell_type(&self) -> tool_config::ToolUserShellType {
        self.session_arc().tool_user_shell_type()
    }

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        command: &'a str,
        workdir: &'a codex_utils_absolute_path::AbsolutePathBuf,
    ) -> thread_service_api::SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            crate::skills::maybe_emit_implicit_skill_invocation(
                &self.session_arc(),
                self,
                command,
                workdir,
            )
            .await;
        })
    }

    fn exec_permission_approvals_enabled(&self) -> bool {
        self.session_arc()
            .enabled(codex_features::Feature::ExecPermissionApprovals)
    }

    fn request_permissions_tool_enabled(&self) -> bool {
        self.session_arc()
            .enabled(codex_features::Feature::RequestPermissionsTool)
    }

    fn resolve_model_shell(&self, shell: &std::path::Path) -> thread_service_api::RuntimeShell {
        let mut shell =
            crate::runtime_shell_model::get_shell_by_model_provided_path(&shell.to_path_buf());
        shell.shell_snapshot = crate::runtime_shell_model::empty_shell_snapshot_receiver();
        shell.to_runtime_shell()
    }

    fn resolve_exec_command(
        &self,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&thread_service_api::RuntimeShell>,
    ) -> Result<thread_service_api::ResolvedExecCommand, String> {
        let session_shell = self.session_arc().user_shell().as_ref().to_runtime_shell();
        thread_service_api::resolve_exec_command_for_parts(
            command,
            login,
            &session_shell,
            model_shell,
            &self.unified_exec_shell_mode(),
            self.allow_login_shell(),
        )
    }

    fn shell_env_overrides(&self) -> std::collections::HashMap<String, String> {
        self.explicit_shell_env_overrides()
    }

    fn resolve_shell_workdir(
        &self,
        workdir: Option<String>,
    ) -> codex_utils_absolute_path::AbsolutePathBuf {
        TurnContext::resolve_shell_workdir(self, workdir)
    }

    fn resolve_turn_path(
        &self,
        path: Option<String>,
    ) -> codex_utils_absolute_path::AbsolutePathBuf {
        #[allow(deprecated)]
        TurnContext::resolve_path(self, path)
    }

    fn begin_tool_network_approval<'a>(
        &'a self,
        spec: Option<
            thread_service_api::NetworkApprovalSpec<
                thread_service_api::ToolRuntimeNetworkApprovalTrigger,
            >,
        >,
    ) -> thread_service_api::SessionCapabilityFuture<
        'a,
        Option<Arc<dyn thread_service_api::ToolRuntimeNetworkApprovalHandle>>,
    > {
        Box::pin(async move {
            let spec = spec.map(|spec| thread_service_api::NetworkApprovalSpec {
                network: spec.network,
                mode: spec.mode,
                trigger: crate::session_capability::map_network_trigger(spec.trigger),
                command: spec.command,
            });
            let active = Arc::clone(&self.session_arc().services.network_approval)
                .begin_network_approval(
                    self.runtime_turn_id().as_str(),
                    self.active_network().is_some(),
                    spec,
                )
                .await;
            let Some(active) = active else {
                return None;
            };
            let mode = active.mode();
            let cancellation_token = active.cancellation_token();
            let state = match mode {
                thread_service_api::NetworkApprovalMode::Deferred => {
                    let Some(deferred) = active.into_deferred() else {
                        panic!("deferred network approval should convert to deferred state");
                    };
                    crate::session_capability::SessionToolNetworkApprovalState::Deferred(deferred)
                }
                thread_service_api::NetworkApprovalMode::Immediate => {
                    crate::session_capability::SessionToolNetworkApprovalState::Immediate(
                        std::sync::Mutex::new(active.registration_id().map(ToString::to_string)),
                    )
                }
            };
            Some(Arc::new(
                crate::session_capability::SessionToolNetworkApprovalHandle {
                    service: Arc::clone(&self.session_arc().services.network_approval),
                    mode,
                    cancellation_token,
                    state,
                },
            )
                as Arc<
                    dyn thread_service_api::ToolRuntimeNetworkApprovalHandle,
                >)
        })
    }

    fn unified_exec_shell_mode(&self) -> tool_config::UnifiedExecShellMode {
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
        tool_service_api::FunctionCallError,
    > {
        self.resolve_exec_command_environment(environment_id, workdir)
            .map(|resolved| {
                resolved.map(
                    |resolved| codex_sandboxing_api::ResolvedExecCommandEnvironment {
                        cwd: resolved.cwd,
                        sandbox_cwd: resolved.sandbox_cwd,
                        environment: resolved.environment,
                        apply_patch_environment: Arc::new(CommandApplyPatchEnvironmentBridge {
                            inner: resolved.apply_patch_environment,
                        }),
                    },
                )
            })
    }
}
