use std::collections::HashMap;
use std::sync::Arc;

use codex_command_service_api::CommandServiceSessionCapability;
use codex_command_service_api::CommandServiceTurnCapability;
use codex_thread_api::ThreadRuntimeCapability;
use codex_thread_api::ToolEventSessionCapability;
use codex_thread_api::ToolEventTurnCapability;
use codex_thread_api::ToolRuntimeNetworkApprovalHandle;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_thread_api::ToolRuntimeSessionCapability;
use codex_thread_api::ToolRuntimeTurnCapability;
use codex_hooks_api::PermissionRequestDecision;
use codex_tool_runtime_api::NetworkApprovalSpec;
use codex_tool_runtime_api::PermissionRequestPayload;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ResolvedExecCommandEnvironment;
use codex_tool_runtime_api::ToolPermissionGrants;

pub(crate) struct SessionCapabilityAdapter {
    pub(crate) inner: Arc<dyn CommandServiceSessionCapability>,
}

impl SessionCapabilityAdapter {
    pub(crate) fn new(inner: Arc<dyn CommandServiceSessionCapability>) -> Self {
        Self { inner }
    }
}

pub(crate) struct TurnCapabilityAdapter {
    pub(crate) inner: Arc<dyn CommandServiceTurnCapability>,
}

impl TurnCapabilityAdapter {
    pub(crate) fn new(inner: Arc<dyn CommandServiceTurnCapability>) -> Self {
        Self { inner }
    }

    pub(crate) fn active_network(&self) -> Option<codex_network_proxy_api::SharedNetworkProxyRuntime> {
        self.inner.active_network()
    }
}

impl codex_thread_api::ToolTurnCapability for TurnCapabilityAdapter {
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }

    fn tool_dispatch_telemetry(&self) -> codex_session_telemetry_api::SharedSessionTelemetry {
        self.inner.tool_sandbox_context().telemetry
    }

    fn base_tool_result_tags(&self) -> codex_tool_runtime_api::ToolTelemetryTags {
        codex_tool_runtime_api::ToolTelemetryTags::default()
    }

    fn rollout_turn_id(&self) -> String {
        self.inner.runtime_turn_id()
    }
}

impl codex_thread_api::ThreadCapability for TurnCapabilityAdapter {
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

impl ThreadRuntimeCapability for TurnCapabilityAdapter {
    fn runtime_turn_id(&self) -> String {
        self.inner.runtime_turn_id()
    }

    fn can_request_original_image_detail(&self) -> bool {
        self.inner.can_request_original_image_detail()
    }

    fn resolve_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, codex_tool_types::FunctionCallError> {
        self.inner.resolve_environment(environment_id)
    }

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cwd: &codex_utils_absolute_path::AbsolutePathBuf,
    ) -> codex_file_system::FileSystemSandboxContext {
        self.inner
            .file_system_sandbox_context(additional_permissions, cwd)
    }

    fn single_local_environment_cwd(
        &self,
    ) -> Result<codex_utils_absolute_path::AbsolutePathBuf, codex_tool_types::FunctionCallError> {
        self.inner.single_local_environment_cwd()
    }

    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64> {
        self.inner.default_agent_job_max_runtime_seconds()
    }
}

impl ToolEventTurnCapability for TurnCapabilityAdapter {
    fn runtime_turn_id_str(&self) -> &str {
        self.inner.runtime_turn_id_str()
    }

    fn truncation_policy(&self) -> codex_utils_output_truncation::TruncationPolicy {
        codex_utils_output_truncation::TruncationPolicy::Tokens(12_000)
    }
}

impl ToolEventSessionCapability for SessionCapabilityAdapter {
    fn tool_send_exec_command_begin<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        event: codex_protocol::protocol::ExecCommandBeginEvent,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        self.inner.send_exec_command_begin(turn.inner.as_ref(), event)
    }

    fn tool_send_exec_command_end<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        event: codex_protocol::protocol::ExecCommandEndEvent,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        self.inner.send_exec_command_end(turn.inner.as_ref(), event)
    }

    fn tool_emit_file_change_started<'a>(
        &'a self,
        _turn: &'a dyn ToolEventTurnCapability,
        _item: codex_protocol::items::FileChangeItem,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        async {}
    }

    fn tool_emit_file_change_completed<'a>(
        &'a self,
        _turn: &'a dyn ToolEventTurnCapability,
        _item: codex_protocol::items::FileChangeItem,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        async {}
    }

    fn tool_record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        items: Vec<codex_protocol::models::ResponseItem>,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        async move {
            self.inner
                .record_model_items_and_emit_display_events(turn.inner.as_ref(), &items)
                .await;
        }
    }

    fn tool_emit_turn_diff<'a>(
        &'a self,
        _turn: &'a dyn ToolEventTurnCapability,
        _event: codex_protocol::protocol::TurnDiffEvent,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        async {}
    }
}

impl ToolRuntimeTurnCapability for TurnCapabilityAdapter {
    fn runtime_turn_id_str(&self) -> &str {
        ToolEventTurnCapability::runtime_turn_id_str(self)
    }

    fn routes_approval_to_guardian(&self) -> bool {
        self.inner.routes_approval_to_guardian()
    }

    fn tool_sandbox_context(&self) -> codex_tool_runtime_api::ToolSandboxContext {
        self.inner.tool_sandbox_context()
    }

    fn approval_policy(&self) -> codex_protocol::protocol::AskForApproval {
        self.inner.approval_policy()
    }

    fn permission_profile(&self) -> codex_protocol::models::PermissionProfile {
        self.inner.tool_sandbox_context().permission_profile
    }

    fn file_system_sandbox_policy(&self) -> codex_protocol::permissions::FileSystemSandboxPolicy {
        self.inner.tool_sandbox_context().file_system_sandbox_policy
    }

    fn windows_sandbox_level(&self) -> codex_protocol::config_types::WindowsSandboxLevel {
        self.inner.tool_sandbox_context().windows_sandbox_level
    }

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cwd: &codex_utils_absolute_path::AbsolutePathBuf,
    ) -> codex_file_system::FileSystemSandboxContext {
        self.inner
            .file_system_sandbox_context(additional_permissions, cwd)
    }

    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, codex_tool_types::FunctionCallError> {
        self.inner.resolve_environment(environment_id)
    }

    fn primary_apply_patch_environment(&self) -> Option<ResolvedApplyPatchEnvironment> {
        None
    }

    fn explicit_shell_env_overrides(&self) -> HashMap<String, String> {
        self.inner.shell_environment_policy().r#set
    }

    fn resolve_shell_workdir(
        &self,
        workdir: Option<String>,
    ) -> codex_utils_absolute_path::AbsolutePathBuf {
        match workdir {
            Some(workdir) => {
                codex_utils_absolute_path::AbsolutePathBuf::try_from(std::path::PathBuf::from(
                    workdir,
                ))
                .unwrap_or_else(|_| self.inner.tool_sandbox_context().cwd)
            }
            None => self.inner.tool_sandbox_context().cwd,
        }
    }

    fn legacy_cwd(&self) -> codex_utils_absolute_path::AbsolutePathBuf {
        self.inner.tool_sandbox_context().cwd
    }

    fn resolve_exec_command_environment(
        &self,
        _environment_id: Option<&str>,
        _workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, codex_tool_types::FunctionCallError> {
        Ok(None)
    }

    fn truncation_policy(&self) -> codex_utils_output_truncation::TruncationPolicy {
        ToolEventTurnCapability::truncation_policy(self)
    }

    fn allow_login_shell(&self) -> bool {
        self.inner.allow_login_shell()
    }

    fn emit_unified_exec_tty_metric(&self, tty: bool) {
        self.inner.emit_unified_exec_tty_metric(tty);
    }
}

impl ToolRuntimeSessionCapability for SessionCapabilityAdapter {
    fn sandbox_runtime(&self) -> codex_sandboxing_api::SharedSandboxRuntime {
        self.inner.sandbox_runtime()
    }

    fn tool_send_exec_command_begin<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: codex_protocol::protocol::ExecCommandBeginEvent,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        ToolEventSessionCapability::tool_send_exec_command_begin(self, turn, event)
    }

    fn tool_send_exec_command_end<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: codex_protocol::protocol::ExecCommandEndEvent,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        ToolEventSessionCapability::tool_send_exec_command_end(self, turn, event)
    }

    fn tool_emit_file_change_started<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        item: codex_protocol::items::FileChangeItem,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        ToolEventSessionCapability::tool_emit_file_change_started(self, turn, item)
    }

    fn tool_emit_file_change_completed<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        item: codex_protocol::items::FileChangeItem,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        ToolEventSessionCapability::tool_emit_file_change_completed(self, turn, item)
    }

    fn tool_record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        items: Vec<codex_protocol::models::ResponseItem>,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        ToolEventSessionCapability::tool_record_model_items_and_emit_display_events(
            self, turn, items,
        )
    }

    fn tool_emit_turn_diff<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: codex_protocol::protocol::TurnDiffEvent,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        ToolEventSessionCapability::tool_emit_turn_diff(self, turn, event)
    }

    fn tool_permission_grants(
        &self,
    ) -> impl std::future::Future<Output = ToolPermissionGrants> + Send + '_ {
        async { ToolPermissionGrants::default() }
    }

    fn dependency_env(
        &self,
    ) -> impl std::future::Future<Output = HashMap<String, String>> + Send + '_ {
        async { HashMap::new() }
    }

    fn exec_permission_approvals_enabled(&self) -> bool {
        true
    }

    fn request_permissions_tool_enabled(&self) -> bool {
        true
    }

    fn create_exec_approval_requirement<'a>(
        &'a self,
        request: codex_permissions_runtime::ExecPolicyApprovalRequest<'a>,
    ) -> impl std::future::Future<Output = codex_tool_runtime_api::ExecApprovalRequirement> + Send + 'a {
        self.inner.create_exec_approval_requirement(request)
    }

    fn strict_auto_review_enabled_for_turn(
        &self,
    ) -> impl std::future::Future<Output = bool> + Send + '_ {
        self.inner.strict_auto_review_enabled_for_turn()
    }

    fn guardian_rejection_message<'a>(
        &'a self,
        review_id: &'a str,
    ) -> impl std::future::Future<Output = String> + Send + 'a {
        self.inner.guardian_rejection_message(review_id)
    }

    fn guardian_timeout_message(&self) -> String {
        self.inner.guardian_timeout_message()
    }

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl std::future::Future<Output = Option<PermissionRequestDecision>> + Send + 'a {
        let turn = codex_thread_api::ToolTurnCapability::as_any(turn)
            .downcast_ref::<TurnCapabilityAdapter>()
            .expect("turn adapter");
        self.inner.run_permission_request_hooks(
            turn.inner.as_ref(),
            permission_request_run_id,
            permission_request,
        )
    }

    fn begin_tool_network_approval<'a>(
        &'a self,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> impl std::future::Future<Output = Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>> + Send + 'a {
        self.inner
            .begin_tool_network_approval(turn_id, managed_network_active, spec)
    }
}
