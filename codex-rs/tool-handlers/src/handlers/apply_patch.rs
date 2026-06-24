use crate::ApplyPatchRequest;
use crate::ApplyPatchRuntime;
use crate::ApplyPatchRuntimeHost;
use crate::ApplyPatchToolOutput;
use crate::FunctionToolOutput;
use crate::HookToolName;
use crate::PostToolUsePayload;
use crate::PreToolUsePayload;
use crate::convert_apply_patch_to_protocol;
use crate::plan_apply_patch;
use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_apply_patch::Hunk;
use codex_apply_patch::MaybeApplyPatchVerified;
use codex_apply_patch::StreamingPatchParser;
use codex_file_system::ExecutorFileSystem;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::FileSystemPermissions;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::PatchApplyUpdatedEvent;
use codex_sandboxing_api::policy_transforms::effective_file_system_sandbox_policy;
use codex_sandboxing_api::policy_transforms::merge_permission_profiles;
use codex_sandboxing_api::policy_transforms::normalize_additional_permissions;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_apply_patch_freeform_tool;
use codex_tool_runtime::ToolArgumentDiffConsumer;
use codex_tool_runtime::ToolCtx;
use codex_tool_runtime::ToolEmitter;
use codex_tool_runtime::ToolEventCtx;
use codex_tool_runtime::ToolHandler;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime::ToolOrchestrator;
use codex_tool_runtime_api::ApplyPatchDiffContext;
use codex_tool_runtime_api::ApplyPatchEnvironment;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ToolOrchestratorHost;
use codex_tool_runtime_api::ToolPermissionGrants;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

pub const APPLY_PATCH_ARGUMENT_DIFF_BUFFER_INTERVAL: Duration = Duration::from_millis(500);

pub type ApplyPatchNetworkTrigger<Host> =
    <<Host as ApplyPatchHandlerHost>::RuntimeHost as ApplyPatchRuntimeHost>::NetworkApprovalTrigger;

pub type ApplyPatchActiveNetworkApproval<Host> =
    <<Host as ApplyPatchHandlerHost>::OrchestratorHost as ToolOrchestratorHost<
        <Host as ApplyPatchHandlerHost>::Session,
        <Host as ApplyPatchHandlerHost>::Turn,
        ApplyPatchNetworkTrigger<Host>,
    >>::ActiveNetworkApproval;

pub type ApplyPatchDeferredNetworkApproval<Host> =
    <<Host as ApplyPatchHandlerHost>::OrchestratorHost as ToolOrchestratorHost<
        <Host as ApplyPatchHandlerHost>::Session,
        <Host as ApplyPatchHandlerHost>::Turn,
        ApplyPatchNetworkTrigger<Host>,
    >>::DeferredNetworkApproval;

#[derive(Default)]
pub struct ApplyPatchHandler<Host> {
    multi_environment: bool,
    host: Host,
}

impl<Host> ApplyPatchHandler<Host>
where
    Host: Default,
{
    pub fn new(multi_environment: bool) -> Self {
        Self {
            multi_environment,
            host: Host::default(),
        }
    }
}

impl<Host> ApplyPatchHandler<Host> {
    pub fn with_host(multi_environment: bool, host: Host) -> Self {
        Self {
            multi_environment,
            host,
        }
    }
}

#[derive(Default)]
pub struct ApplyPatchArgumentDiffConsumer {
    parser: StreamingPatchParser,
    pub last_sent_at: Option<Instant>,
    pending: Option<PatchApplyUpdatedEvent>,
}

impl<DiffContext> ToolArgumentDiffConsumer<DiffContext> for ApplyPatchArgumentDiffConsumer
where
    DiffContext: ApplyPatchDiffContext,
{
    fn consume_diff(
        &mut self,
        turn: &DiffContext,
        call_id: String,
        diff: &str,
    ) -> Option<EventMsg> {
        if !turn.apply_patch_streaming_events_enabled() {
            return None;
        }

        self.push_delta(call_id, diff)
            .map(EventMsg::PatchApplyUpdated)
    }

    fn finish(&mut self) -> Result<Option<EventMsg>, FunctionCallError> {
        self.finish_update_on_complete()
            .map(|event| event.map(EventMsg::PatchApplyUpdated))
    }
}

impl ApplyPatchArgumentDiffConsumer {
    pub fn push_delta(&mut self, call_id: String, delta: &str) -> Option<PatchApplyUpdatedEvent> {
        let hunks = self.parser.push_delta(delta).ok()?;
        if hunks.is_empty() {
            return None;
        }
        let changes = convert_apply_patch_hunks_to_protocol(&hunks);
        let event = PatchApplyUpdatedEvent { call_id, changes };
        let now = Instant::now();
        match self.last_sent_at {
            Some(last_sent_at)
                if now.duration_since(last_sent_at) < APPLY_PATCH_ARGUMENT_DIFF_BUFFER_INTERVAL =>
            {
                self.pending = Some(event);
                None
            }
            Some(_) | None => {
                self.pending = None;
                self.last_sent_at = Some(now);
                Some(event)
            }
        }
    }

    pub fn finish_update_on_complete(
        &mut self,
    ) -> Result<Option<PatchApplyUpdatedEvent>, FunctionCallError> {
        self.parser.finish().map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to parse apply_patch: {err}"))
        })?;

        let event = self.pending.take();
        if event.is_some() {
            self.last_sent_at = Some(Instant::now());
        }
        Ok(event)
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for ApplyPatchHandler<Host>
where
    Host: ApplyPatchHandlerHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
{
    type Output = ApplyPatchToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("apply_patch")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_apply_patch_freeform_tool(self.multi_environment))
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
        ToolInvocation<Host::Session, Host::Turn, Host::Tracker>: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                tracker,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let tool_name = metadata.tool_name;

            let ToolPayload::Custom { input: patch_input } = metadata.payload else {
                return Err(FunctionCallError::RespondToModel(
                    "apply_patch handler received unsupported payload".to_string(),
                ));
            };
            let args = codex_apply_patch::parse_patch(&patch_input).map_err(|parse_error| {
                FunctionCallError::RespondToModel(format!(
                    "apply_patch verification failed: {parse_error}"
                ))
            })?;
            let selected_environment_id =
                require_environment_id(args.environment_id.as_deref(), self.multi_environment)?;
            let Some(environment) = self
                .host
                .resolve_environment(&turn, selected_environment_id.as_deref())?
            else {
                return Err(FunctionCallError::RespondToModel(
                    "apply_patch is unavailable in this session".to_string(),
                ));
            };
            let sandbox = self.host.file_system_sandbox_context(
                &turn,
                /*additional_permissions*/ None,
                &environment.cwd,
            );
            match codex_apply_patch::verify_apply_patch_args(
                args,
                &environment.cwd,
                environment.environment.filesystem().as_ref(),
                Some(&sandbox),
            )
            .await
            {
                MaybeApplyPatchVerified::Body(action) => {
                    let content = execute_verified_apply_patch(
                        &self.host,
                        session,
                        turn,
                        Some(&tracker),
                        &call_id,
                        tool_name,
                        action,
                        environment,
                    )
                    .await?;
                    Ok(ApplyPatchToolOutput::from_text(content))
                }
                MaybeApplyPatchVerified::CorrectnessError(parse_error) => {
                    Err(FunctionCallError::RespondToModel(format!(
                        "apply_patch verification failed: {parse_error}"
                    )))
                }
                MaybeApplyPatchVerified::ShellParseError(error) => {
                    tracing::trace!("Failed to parse apply_patch input, {error:?}");
                    Err(FunctionCallError::RespondToModel(
                        "apply_patch handler received invalid patch input".to_string(),
                    ))
                }
                MaybeApplyPatchVerified::NotApplyPatch => Err(FunctionCallError::RespondToModel(
                    "apply_patch handler received non-apply_patch input".to_string(),
                )),
            }
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for ApplyPatchHandler<Host>
where
    Host: ApplyPatchHandlerHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Custom { .. })
    }

    fn create_diff_consumer(&self) -> Option<Box<dyn ToolArgumentDiffConsumer<Host::DiffContext>>> {
        Some(Box::<ApplyPatchArgumentDiffConsumer>::default())
    }

    fn pre_tool_use_payload(
        &self,
        invocation: &ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> Option<PreToolUsePayload> {
        apply_patch_payload_command(&invocation.payload).map(|command| PreToolUsePayload {
            tool_name: HookToolName::apply_patch(),
            tool_input: serde_json::json!({ "command": command }),
        })
    }

    fn with_updated_hook_input(
        &self,
        mut invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        updated_input: serde_json::Value,
    ) -> Result<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, FunctionCallError> {
        let patch = updated_hook_command(&updated_input)?;
        if matches!(invocation.metadata.payload, ToolPayload::Custom { .. }) {
            invocation.metadata.payload = ToolPayload::Custom {
                input: patch.to_string(),
            };
        }
        Ok(invocation)
    }

    fn post_tool_use_payload(
        &self,
        invocation: &ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        result: &Self::Output,
    ) -> Option<PostToolUsePayload> {
        let tool_response =
            result.post_tool_use_response(&invocation.call_id, &invocation.payload)?;
        Some(PostToolUsePayload {
            tool_name: HookToolName::apply_patch(),
            tool_use_id: invocation.call_id.clone(),
            tool_input: serde_json::json!({
                "command": apply_patch_payload_command(&invocation.payload)?,
            }),
            tool_response,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn intercept_apply_patch<Host>(
    host: &Host,
    command: &[String],
    cwd: &AbsolutePathBuf,
    fs: &dyn ExecutorFileSystem,
    environment: Arc<dyn ApplyPatchEnvironment>,
    session: Host::Session,
    turn: Host::Turn,
    tracker: Option<&Host::Tracker>,
    call_id: &str,
    tool_name: &str,
) -> Result<Option<FunctionToolOutput>, FunctionCallError>
where
    Host: ApplyPatchHandlerHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
{
    let sandbox =
        host.file_system_sandbox_context(&turn, /*additional_permissions*/ None, cwd);
    match codex_apply_patch::maybe_parse_apply_patch_verified(command, cwd, fs, Some(&sandbox))
        .await
    {
        MaybeApplyPatchVerified::Body(action) => {
            let environment = ResolvedApplyPatchEnvironment {
                cwd: cwd.clone(),
                environment,
            };
            let content = execute_verified_apply_patch(
                host,
                session,
                turn,
                tracker,
                call_id,
                ToolName::plain(tool_name),
                action,
                environment,
            )
            .await?;
            Ok(Some(FunctionToolOutput::from_text(content, Some(true))))
        }
        MaybeApplyPatchVerified::CorrectnessError(parse_error) => {
            Err(FunctionCallError::RespondToModel(format!(
                "apply_patch verification failed: {parse_error}"
            )))
        }
        MaybeApplyPatchVerified::ShellParseError(error) => {
            tracing::trace!("Failed to parse apply_patch input, {error:?}");
            Ok(None)
        }
        MaybeApplyPatchVerified::NotApplyPatch => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_verified_apply_patch<Host>(
    host: &Host,
    session: Host::Session,
    turn: Host::Turn,
    tracker: Option<&Host::Tracker>,
    call_id: &str,
    tool_name: ToolName,
    action: ApplyPatchAction,
    environment: ResolvedApplyPatchEnvironment,
) -> Result<String, FunctionCallError>
where
    Host: ApplyPatchHandlerHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
{
    let cwd = environment.cwd.clone();
    let (file_paths, effective_additional_permissions, file_system_sandbox_policy) =
        effective_patch_permissions(host, &session, &turn, &action, &cwd).await;
    let apply = match plan_apply_patch(
        action,
        host.approval_policy(&turn),
        &host.permission_profile(&turn),
        &file_system_sandbox_policy,
        &cwd,
        host.windows_sandbox_level(&turn),
    ) {
        crate::ApplyPatchPlan::DelegateToRuntime(invocation) => invocation,
        crate::ApplyPatchPlan::Reject { reason } => {
            return Err(FunctionCallError::RespondToModel(format!(
                "patch rejected: {reason}"
            )));
        }
    };

    let changes = convert_apply_patch_to_protocol(&apply.action);
    let emitter = ToolEmitter::apply_patch(changes.clone(), apply.auto_approved);
    let event_host = host.event_host(&session, &turn, tracker);
    emitter.begin(ToolEventCtx::new(event_host, call_id)).await;

    let req = ApplyPatchRequest {
        environment: environment.environment,
        action: apply.action,
        file_paths,
        changes,
        exec_approval_requirement: apply.exec_approval_requirement,
        additional_permissions: effective_additional_permissions.additional_permissions,
        permissions_preapproved: effective_additional_permissions.permissions_preapproved,
    };

    let mut orchestrator =
        ToolOrchestrator::new(host.orchestrator_host(), host.sandbox_runtime(&session));
    let mut runtime = ApplyPatchRuntime::new(host.runtime_host());
    let tool_ctx = ToolCtx {
        session: session.clone(),
        turn: turn.clone(),
        call_id: call_id.to_string(),
        tool_name,
    };
    let out = orchestrator
        .run(
            &mut runtime,
            &req,
            &tool_ctx,
            &host.tool_sandbox_context(&turn),
            host.approval_policy(&turn),
        )
        .await
        .map(|result| result.output);
    let (out, delta) = match out {
        Ok(output) => (Ok(output.exec_output), Some(output.delta)),
        Err(error) => (Err(error), Some(runtime.committed_delta().clone())),
    };
    let event_host = host.event_host(&session, &turn, tracker);
    emitter
        .finish(ToolEventCtx::new(event_host, call_id), out, delta.as_ref())
        .await
}

async fn effective_patch_permissions<Host>(
    host: &Host,
    session: &Host::Session,
    turn: &Host::Turn,
    action: &ApplyPatchAction,
    cwd: &AbsolutePathBuf,
) -> (
    Vec<AbsolutePathBuf>,
    EffectiveAdditionalPermissions,
    FileSystemSandboxPolicy,
)
where
    Host: ApplyPatchHandlerHost,
{
    let file_paths = file_paths_for_action(action);
    let grants = host.permission_grants(session).await;
    let granted_permissions =
        merge_permission_profiles(grants.session.as_ref(), grants.turn.as_ref());
    let base_file_system_sandbox_policy = host.file_system_sandbox_policy(turn);
    let file_system_sandbox_policy = effective_file_system_sandbox_policy(
        &base_file_system_sandbox_policy,
        granted_permissions.as_ref(),
    );
    let effective_additional_permissions = apply_granted_permissions_from_grants(
        ToolPermissionGrants {
            session: None,
            turn: granted_permissions,
        },
        cwd.as_path(),
        SandboxPermissions::UseDefault,
        write_permissions_for_paths(&file_paths, &file_system_sandbox_policy, cwd),
    );

    (
        file_paths,
        effective_additional_permissions,
        file_system_sandbox_policy,
    )
}

/// Extracts the raw patch text used as the command-shaped hook input for apply_patch.
pub fn apply_patch_payload_command(payload: &ToolPayload) -> Option<String> {
    match payload {
        ToolPayload::Custom { input } => Some(input.clone()),
        _ => None,
    }
}

fn updated_hook_command(updated_input: &serde_json::Value) -> Result<&str, FunctionCallError> {
    updated_input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "hook returned updatedInput without string field `command`".to_string(),
            )
        })
}

pub fn require_environment_id(
    parsed_environment_id: Option<&str>,
    allow_environment_id: bool,
) -> Result<Option<String>, FunctionCallError> {
    match parsed_environment_id {
        Some(_) if !allow_environment_id => Err(FunctionCallError::RespondToModel(
            "apply_patch environment selection is unavailable for this turn".to_string(),
        )),
        Some(environment_id) => Ok(Some(environment_id.to_string())),
        None => Ok(None),
    }
}

pub fn file_paths_for_action(action: &ApplyPatchAction) -> Vec<AbsolutePathBuf> {
    let mut keys = Vec::new();
    let cwd = &action.cwd;

    for (path, change) in action.changes() {
        keys.push(AbsolutePathBuf::resolve_path_against_base(path, cwd));

        if let ApplyPatchFileChange::Update { move_path, .. } = change
            && let Some(dest) = move_path
        {
            keys.push(AbsolutePathBuf::resolve_path_against_base(dest, cwd));
        }
    }

    keys
}

pub fn write_permissions_for_paths(
    file_paths: &[AbsolutePathBuf],
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
) -> Option<AdditionalPermissionProfile> {
    let write_paths = file_paths
        .iter()
        .map(|path| {
            path.parent()
                .unwrap_or_else(|| path.clone())
                .into_path_buf()
        })
        .filter(|path| {
            !file_system_sandbox_policy.can_write_path_with_cwd(path.as_path(), cwd.as_path())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(AbsolutePathBuf::from_absolute_path)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;

    let permissions = (!write_paths.is_empty()).then_some(AdditionalPermissionProfile {
        file_system: Some(FileSystemPermissions::from_read_write_roots(
            Some(vec![]),
            Some(write_paths),
        )),
        ..Default::default()
    })?;

    normalize_additional_permissions(permissions).ok()
}

pub struct EffectiveAdditionalPermissions {
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub permissions_preapproved: bool,
}

pub fn implicit_granted_permissions(
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<&AdditionalPermissionProfile>,
    effective_additional_permissions: &EffectiveAdditionalPermissions,
) -> Option<AdditionalPermissionProfile> {
    if !sandbox_permissions.uses_additional_permissions()
        && !matches!(sandbox_permissions, SandboxPermissions::RequireEscalated)
        && additional_permissions.is_none()
    {
        effective_additional_permissions
            .additional_permissions
            .clone()
    } else {
        None
    }
}

pub fn apply_granted_permissions_from_grants(
    grants: ToolPermissionGrants,
    cwd: &Path,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<AdditionalPermissionProfile>,
) -> EffectiveAdditionalPermissions {
    if matches!(sandbox_permissions, SandboxPermissions::RequireEscalated) {
        return EffectiveAdditionalPermissions {
            sandbox_permissions,
            additional_permissions,
            permissions_preapproved: false,
        };
    }

    let granted_permissions =
        merge_permission_profiles(grants.session.as_ref(), grants.turn.as_ref());
    let effective_permissions = merge_permission_profiles(
        additional_permissions.as_ref(),
        granted_permissions.as_ref(),
    );
    let permissions_preapproved = match (effective_permissions.as_ref(), granted_permissions) {
        (Some(effective_permissions), Some(granted_permissions)) => {
            permissions_are_preapproved(effective_permissions, granted_permissions, cwd)
        }
        _ => false,
    };

    let sandbox_permissions =
        if effective_permissions.is_some() && !sandbox_permissions.uses_additional_permissions() {
            SandboxPermissions::WithAdditionalPermissions
        } else {
            sandbox_permissions
        };

    EffectiveAdditionalPermissions {
        sandbox_permissions,
        additional_permissions: effective_permissions,
        permissions_preapproved,
    }
}

pub fn normalize_and_validate_additional_permissions(
    additional_permissions_allowed: bool,
    approval_policy: AskForApproval,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<AdditionalPermissionProfile>,
    permissions_preapproved: bool,
    _cwd: &Path,
) -> Result<Option<AdditionalPermissionProfile>, String> {
    let uses_additional_permissions = matches!(
        sandbox_permissions,
        SandboxPermissions::WithAdditionalPermissions
    );

    if !permissions_preapproved
        && !additional_permissions_allowed
        && (uses_additional_permissions || additional_permissions.is_some())
    {
        return Err(
            "additional permissions are disabled; enable `features.exec_permission_approvals` before using `with_additional_permissions`"
                .to_string(),
        );
    }

    if uses_additional_permissions {
        if !permissions_preapproved && !matches!(approval_policy, AskForApproval::OnRequest) {
            return Err(format!(
                "approval policy is {approval_policy:?}; reject command — you cannot request additional permissions unless the approval policy is OnRequest"
            ));
        }
        let Some(additional_permissions) = additional_permissions else {
            return Err(
                "missing `additional_permissions`; provide at least one of `network` or `file_system` when using `with_additional_permissions`"
                    .to_string(),
            );
        };
        let normalized = normalize_additional_permissions(additional_permissions)?;
        if normalized.is_empty() {
            return Err(
                "`additional_permissions` must include at least one requested permission in `network` or `file_system`"
                    .to_string(),
            );
        }
        return Ok(Some(normalized));
    }

    if additional_permissions.is_some() {
        Err(
            "`additional_permissions` requires `sandbox_permissions` set to `with_additional_permissions`"
                .to_string(),
        )
    } else {
        Ok(None)
    }
}

fn permissions_are_preapproved(
    effective_permissions: &AdditionalPermissionProfile,
    granted_permissions: AdditionalPermissionProfile,
    cwd: &Path,
) -> bool {
    let materialized_effective_permissions =
        codex_sandboxing_api::policy_transforms::intersect_permission_profiles(
            effective_permissions.clone(),
            effective_permissions.clone(),
            cwd,
        );
    codex_sandboxing_api::policy_transforms::intersect_permission_profiles(
        effective_permissions.clone(),
        granted_permissions,
        cwd,
    ) == materialized_effective_permissions
}

fn convert_apply_patch_hunks_to_protocol(hunks: &[Hunk]) -> HashMap<PathBuf, FileChange> {
    hunks
        .iter()
        .map(|hunk| {
            let path = hunk_source_path(hunk).to_path_buf();
            let change = match hunk {
                Hunk::AddFile { contents, .. } => FileChange::Add {
                    content: contents.clone(),
                },
                Hunk::DeleteFile { .. } => FileChange::Delete {
                    content: String::new(),
                },
                Hunk::UpdateFile {
                    chunks, move_path, ..
                } => FileChange::Update {
                    unified_diff: format_update_chunks_for_progress(chunks),
                    move_path: move_path.clone(),
                },
            };
            (path, change)
        })
        .collect()
}

fn hunk_source_path(hunk: &Hunk) -> &Path {
    match hunk {
        Hunk::AddFile { path, .. } | Hunk::DeleteFile { path } | Hunk::UpdateFile { path, .. } => {
            path
        }
    }
}

fn format_update_chunks_for_progress(chunks: &[codex_apply_patch::UpdateFileChunk]) -> String {
    let mut unified_diff = String::new();
    for chunk in chunks {
        match &chunk.change_context {
            Some(context) => {
                unified_diff.push_str("@@ ");
                unified_diff.push_str(context);
                unified_diff.push('\n');
            }
            None => {
                unified_diff.push_str("@@");
                unified_diff.push('\n');
            }
        }
        for line in &chunk.old_lines {
            unified_diff.push('-');
            unified_diff.push_str(line);
            unified_diff.push('\n');
        }
        for line in &chunk.new_lines {
            unified_diff.push('+');
            unified_diff.push_str(line);
            unified_diff.push('\n');
        }
        if chunk.is_end_of_file {
            unified_diff.push_str("*** End of File");
            unified_diff.push('\n');
        }
    }
    unified_diff
}

#[cfg(test)]
#[path = "apply_patch_tests.rs"]
mod tests;
