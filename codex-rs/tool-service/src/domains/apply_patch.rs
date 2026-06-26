use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::ApplyPatchApprovalDispatch;
use codex_apply_patch::AppliedPatchDelta;
use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_apply_patch::Hunk;
use codex_apply_patch::MaybeApplyPatchVerified;
use codex_apply_patch::StreamingPatchParser;
use codex_command_runtime::is_likely_sandbox_denied;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::exec_output::StreamOutput;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::FileSystemPermissions;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::PatchApplyUpdatedEvent;
use codex_sandboxing_api::SandboxType;
use codex_sandboxing_api::SandboxablePreference;
use codex_sandboxing_api::policy_transforms::effective_file_system_sandbox_policy;
use codex_sandboxing_api::policy_transforms::effective_permission_profile;
use codex_sandboxing_api::policy_transforms::merge_permission_profiles;
use codex_sandboxing_api::policy_transforms::normalize_additional_permissions;
use codex_thread_api::SessionToolEventHost;
use codex_thread_api::SharedToolTurnDiffTracker;
use codex_thread_api::ToolRuntimeSessionCapability;
use codex_thread_api::ToolRuntimeTurnCapability;
use codex_thread_runtime::ThreadRuntimeSession;
use codex_thread_runtime::ThreadTurnContext;
use codex_tool_runtime::ApplyPatchToolOutput;
use codex_tool_runtime::FunctionToolOutput;
use codex_tool_runtime::ToolEmitter;
use codex_tool_runtime::ToolEventCtx;
use codex_tool_runtime::convert_apply_patch_to_protocol;
use codex_tool_runtime::plan_apply_patch;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_runtime_api::ApplyPatchDiffContext;
use codex_tool_runtime_api::ApplyPatchRequest;
use codex_tool_runtime_api::PostToolUsePayload;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ToolError;
use codex_tool_runtime_api::sandbox_override_for_first_attempt;
use codex_tool_runtime_api::should_bypass_approval;
use codex_tool_runtime_api::wants_no_sandbox_approval;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_planning::ToolEnvironmentMode;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_apply_patch_freeform_tool;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;

use crate::context::TypedToolSpecRequest;

const APPLY_PATCH_TOOL_NAME: &str = "apply_patch";
pub(crate) const APPLY_PATCH_ARGUMENT_DIFF_BUFFER_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    vec![create_apply_patch_freeform_tool(matches!(
        request.config.environment_mode,
        ToolEnvironmentMode::Multiple
    ))]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name == APPLY_PATCH_TOOL_NAME
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    Some(Box::<ApplyPatchArgumentDiffConsumer>::default())
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) async fn dispatch(
    approval_api: Arc<dyn ApprovalServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    tracker: SharedToolTurnDiffTracker,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let output =
        dispatch_apply_patch(approval_api, session.clone(), turn.clone(), tracker, &call).await?;
    let post_tool_use_payload = Some(PostToolUsePayload {
        tool_name: codex_tool_runtime::HookToolName::apply_patch(),
        tool_use_id: call.call_id.clone(),
        tool_input: serde_json::json!({
            "command": apply_patch_payload_command(&call.payload)?,
        }),
        tool_response: output
            .post_tool_use_response(&call.call_id, &call.payload)
            .ok_or_else(|| {
                FunctionCallError::Fatal("apply_patch post_tool_use payload missing".to_string())
            })?,
    });

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(output),
        post_tool_use_payload,
    })
}

async fn dispatch_apply_patch(
    approval_api: Arc<dyn ApprovalServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    tracker: SharedToolTurnDiffTracker,
    call: &ToolCall,
) -> Result<ApplyPatchToolOutput, FunctionCallError> {
    let ToolPayload::Custom { input: patch_input } = &call.payload else {
        return Err(FunctionCallError::RespondToModel(
            "apply_patch handler received unsupported payload".to_string(),
        ));
    };

    let args = codex_apply_patch::parse_patch(patch_input).map_err(|parse_error| {
        FunctionCallError::RespondToModel(format!(
            "apply_patch verification failed: {parse_error}"
        ))
    })?;
    let selected_environment_id = require_environment_id(
        args.environment_id.as_deref(),
        true,
    )?;
    let Some(environment) = turn.resolve_apply_patch_environment(selected_environment_id.as_deref())?
    else {
        return Err(FunctionCallError::RespondToModel(
            "apply_patch is unavailable in this session".to_string(),
        ));
    };
    let sandbox = turn.file_system_sandbox_context(None, &environment.cwd);
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
                approval_api,
                session,
                turn,
                Some(&tracker),
                &call.call_id,
                action,
                environment,
            )
            .await?;
            Ok(ApplyPatchToolOutput::from_text(content))
        }
        MaybeApplyPatchVerified::CorrectnessError(parse_error) => Err(
            FunctionCallError::RespondToModel(format!(
                "apply_patch verification failed: {parse_error}"
            )),
        ),
        MaybeApplyPatchVerified::ShellParseError(_) => {
            Err(FunctionCallError::RespondToModel(
                "apply_patch handler received invalid patch input".to_string(),
            ))
        }
        MaybeApplyPatchVerified::NotApplyPatch => Err(FunctionCallError::RespondToModel(
            "apply_patch handler received non-apply_patch input".to_string(),
        )),
    }
}

#[derive(Default)]
struct ApplyPatchArgumentDiffConsumer {
    parser: StreamingPatchParser,
    last_sent_at: Option<Instant>,
    pending: Option<PatchApplyUpdatedEvent>,
}

impl ErasedToolArgumentDiffConsumer for ApplyPatchArgumentDiffConsumer {
    fn consume_diff(
        &mut self,
        turn: &dyn codex_thread_api::ToolServiceTurnRef,
        call_id: String,
        diff: &str,
    ) -> Option<EventMsg> {
        let turn = turn.as_any().downcast_ref::<Arc<ThreadTurnContext>>()?;
        if !turn.as_ref().apply_patch_streaming_events_enabled() {
            return None;
        }

        self.push_delta(call_id, diff).map(EventMsg::PatchApplyUpdated)
    }

    fn finish(&mut self) -> Result<Option<EventMsg>, FunctionCallError> {
        self.finish_update_on_complete()
            .map(|event| event.map(EventMsg::PatchApplyUpdated))
    }
}

impl ApplyPatchArgumentDiffConsumer {
    fn push_delta(&mut self, call_id: String, delta: &str) -> Option<PatchApplyUpdatedEvent> {
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

    fn finish_update_on_complete(
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

pub(crate) async fn intercept_apply_patch(
    approval_api: Arc<dyn ApprovalServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    tracker: Option<&SharedToolTurnDiffTracker>,
    command: &[String],
    cwd: &AbsolutePathBuf,
    environment: Arc<dyn codex_tool_runtime_api::ApplyPatchEnvironment>,
    call_id: &str,
    _tool_name: &str,
) -> Result<Option<FunctionToolOutput>, FunctionCallError> {
    let sandbox = turn.file_system_sandbox_context(None, cwd);
    match codex_apply_patch::maybe_parse_apply_patch_verified(
        command,
        cwd,
        environment.filesystem().as_ref(),
        Some(&sandbox),
    )
    .await
    {
        MaybeApplyPatchVerified::Body(action) => {
            let environment = ResolvedApplyPatchEnvironment {
                cwd: cwd.clone(),
                environment,
            };
            let content = execute_verified_apply_patch(
                approval_api,
                session,
                turn,
                tracker,
                call_id,
                action,
                environment,
            )
            .await?;
            Ok(Some(FunctionToolOutput::from_text(content, Some(true))))
        }
        MaybeApplyPatchVerified::CorrectnessError(parse_error) => Err(
            FunctionCallError::RespondToModel(format!(
                "apply_patch verification failed: {parse_error}"
            )),
        ),
        MaybeApplyPatchVerified::ShellParseError(_) => {
            Ok(None)
        }
        MaybeApplyPatchVerified::NotApplyPatch => Ok(None),
    }
}

async fn execute_verified_apply_patch(
    approval_api: Arc<dyn ApprovalServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    tracker: Option<&SharedToolTurnDiffTracker>,
    call_id: &str,
    action: ApplyPatchAction,
    environment: ResolvedApplyPatchEnvironment,
) -> Result<String, FunctionCallError> {
    let cwd = environment.cwd.clone();
    let (file_paths, effective_additional_permissions, file_system_sandbox_policy) =
        effective_patch_permissions(session.as_ref(), turn.as_ref(), &action, &cwd).await;
    let apply = match plan_apply_patch(
        action,
        turn.approval_policy(),
        &turn.permission_profile(),
        &file_system_sandbox_policy,
        &cwd,
        turn.windows_sandbox_level(),
    ) {
        codex_tool_runtime::ApplyPatchPlan::DelegateToRuntime(invocation) => invocation,
        codex_tool_runtime::ApplyPatchPlan::Reject { reason } => {
            return Err(FunctionCallError::RespondToModel(format!(
                "patch rejected: {reason}"
            )));
        }
    };

    let changes = convert_apply_patch_to_protocol(&apply.action);
    let emitter = ToolEmitter::apply_patch(changes.clone(), apply.auto_approved);
    let event_host = SessionToolEventHost::new(session.as_ref(), &turn, tracker);
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
    let mut committed_delta = AppliedPatchDelta::default();
    let out = run_apply_patch_request(
        approval_api,
        Arc::clone(&session),
        Arc::clone(&turn),
        call_id,
        &req,
        &mut committed_delta,
    )
    .await;
    let delta = Some(committed_delta);
    let event_host = SessionToolEventHost::new(session.as_ref(), &turn, tracker);
    emitter
        .finish(ToolEventCtx::new(event_host, call_id), out, delta.as_ref())
        .await
}

async fn run_apply_patch_request(
    approval_api: Arc<dyn ApprovalServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    call_id: &str,
    req: &ApplyPatchRequest,
    committed_delta: &mut AppliedPatchDelta,
) -> Result<ExecToolCallOutput, ToolError> {
    let approval_policy = turn.approval_policy();
    let file_system_sandbox_policy = turn.file_system_sandbox_policy();
    let strict_auto_review = session.strict_auto_review_enabled_for_turn().await;
    let mut already_approved = false;

    match &req.exec_approval_requirement {
        codex_tool_runtime_api::ExecApprovalRequirement::Skip { .. } => {
            if strict_auto_review {
                request_apply_patch_approval(
                    Arc::clone(&approval_api),
                    Arc::clone(&session),
                    Arc::clone(&turn),
                    call_id,
                    req,
                    /*retry_reason*/ None,
                )
                .await?;
                already_approved = true;
            }
        }
        codex_tool_runtime_api::ExecApprovalRequirement::Forbidden { reason } => {
            return Err(ToolError::Rejected(reason.clone()));
        }
        codex_tool_runtime_api::ExecApprovalRequirement::NeedsApproval { reason, .. } => {
            if !strict_auto_review
                && let Some(decision) = session
                    .run_permission_request_hooks(
                        turn.as_ref(),
                        call_id,
                        codex_tool_runtime_api::PermissionRequestPayload {
                            tool_name: codex_tool_runtime::HookToolName::apply_patch(),
                            tool_input: serde_json::json!({ "command": req.action.patch }),
                        },
                    )
                    .await
            {
                match decision {
                    codex_hooks_api::PermissionRequestDecision::Allow => {
                        already_approved = true;
                    }
                    codex_hooks_api::PermissionRequestDecision::Deny { message } => {
                        return Err(ToolError::Rejected(message));
                    }
                }
            }

            if !already_approved {
                request_apply_patch_approval(
                    Arc::clone(&approval_api),
                    Arc::clone(&session),
                    Arc::clone(&turn),
                    call_id,
                    req,
                    reason.clone(),
                )
                .await?;
                already_approved = true;
            }
        }
    }

    let tool_sandbox_context = turn.tool_sandbox_context();
    let initial_sandbox = match sandbox_override_for_first_attempt(
        SandboxPermissions::UseDefault,
        &req.exec_approval_requirement,
        &file_system_sandbox_policy,
    ) {
        codex_tool_runtime_api::SandboxOverride::BypassSandboxFirstAttempt => SandboxType::None,
        codex_tool_runtime_api::SandboxOverride::NoOverride => session.sandbox_runtime().select_initial(
            &file_system_sandbox_policy,
            tool_sandbox_context.network_sandbox_policy,
            SandboxablePreference::Auto,
            tool_sandbox_context.windows_sandbox_level,
            tool_sandbox_context.managed_network_active,
        ),
    };

    let first_result = run_apply_patch_attempt(req, initial_sandbox, &tool_sandbox_context, committed_delta).await;
    match first_result {
        Ok(output) => Ok(output),
        Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied { output, .. }))) => {
            if !wants_no_sandbox_approval(approval_policy) {
                return Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                    output,
                    network_policy_decision: None,
                })));
            }

            if !should_bypass_approval(approval_policy, already_approved) {
                request_apply_patch_approval(
                    approval_api,
                    session,
                    turn,
                    call_id,
                    req,
                    Some("patch failed; retry without sandbox?".to_string()),
                )
                .await?;
            }

            run_apply_patch_attempt(req, SandboxType::None, &tool_sandbox_context, committed_delta).await
        }
        Err(err) => Err(err),
    }
}

async fn request_apply_patch_approval(
    approval_api: Arc<dyn ApprovalServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    call_id: &str,
    req: &ApplyPatchRequest,
    retry_reason: Option<String>,
) -> Result<(), ToolError> {
    approval_api
        .request_apply_patch_approval(ApplyPatchApprovalDispatch {
            session,
            turn,
            call_id: call_id.to_string(),
            approval_keys: apply_patch_approval_keys(req.environment.environment_id(), &req.file_paths),
            approval_request: codex_tool_runtime_api::ApplyPatchApprovalRequest::from_request(req),
            changes: req.changes.clone(),
            permissions_preapproved: req.permissions_preapproved,
            retry_reason,
        })
        .await
        .map_err(ToolError::Rejected)
}

async fn run_apply_patch_attempt(
    req: &ApplyPatchRequest,
    sandbox: SandboxType,
    tool_sandbox_context: &codex_tool_runtime_api::ToolSandboxContext,
    committed_delta: &mut AppliedPatchDelta,
) -> Result<ExecToolCallOutput, ToolError> {
    let started_at = Instant::now();
    let filesystem = req.environment.filesystem();
    let sandbox_context = file_system_sandbox_context_for_attempt(req, sandbox, tool_sandbox_context);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = codex_apply_patch::apply_patch(
        &req.action.patch,
        &req.action.cwd,
        &mut stdout,
        &mut stderr,
        filesystem.as_ref(),
        sandbox_context.as_ref(),
    )
    .await;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    let failed = result.is_err();
    let exit_code = if failed { 1 } else { 0 };
    let delta = match result {
        Ok(delta) => delta,
        Err(failure) => failure.into_parts().1,
    };
    committed_delta.append(delta);
    let output = ExecToolCallOutput {
        exit_code,
        stdout: StreamOutput::new(stdout.clone()),
        stderr: StreamOutput::new(stderr.clone()),
        aggregated_output: StreamOutput::new(format!("{stdout}{stderr}")),
        duration: started_at.elapsed(),
        timed_out: false,
    };
    if failed && is_likely_sandbox_denied(sandbox, &output) {
        return Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
            output: Box::new(output),
            network_policy_decision: None,
        })));
    }
    Ok(output)
}

fn file_system_sandbox_context_for_attempt(
    req: &ApplyPatchRequest,
    sandbox: SandboxType,
    tool_sandbox_context: &codex_tool_runtime_api::ToolSandboxContext,
) -> Option<codex_file_system::FileSystemSandboxContext> {
    if sandbox == SandboxType::None {
        return None;
    }

    let permissions =
        effective_permission_profile(&tool_sandbox_context.permission_profile, req.additional_permissions.as_ref());
    Some(codex_file_system::FileSystemSandboxContext {
        permissions,
        cwd: Some(req.action.cwd.clone()),
        windows_sandbox_level: tool_sandbox_context.windows_sandbox_level,
        windows_sandbox_private_desktop: tool_sandbox_context.windows_sandbox_private_desktop,
        use_legacy_landlock: tool_sandbox_context.use_legacy_landlock,
    })
}

fn apply_patch_approval_keys(
    environment_id: &str,
    file_paths: &[AbsolutePathBuf],
) -> Vec<codex_tool_runtime_api::ApplyPatchApprovalKey> {
    file_paths
        .iter()
        .cloned()
        .map(|path| codex_tool_runtime_api::ApplyPatchApprovalKey {
            environment_id: environment_id.to_string(),
            path,
        })
        .collect()
}

async fn effective_patch_permissions(
    session: &ThreadRuntimeSession,
    turn: &ThreadTurnContext,
    action: &ApplyPatchAction,
    cwd: &AbsolutePathBuf,
) -> (
    Vec<AbsolutePathBuf>,
    EffectiveAdditionalPermissions,
    FileSystemSandboxPolicy,
) {
    let file_paths = file_paths_for_action(action);
    let grants = session.tool_permission_grants().await;
    let granted_permissions = merge_permission_profiles(grants.session.as_ref(), grants.turn.as_ref());
    let base_file_system_sandbox_policy = turn.file_system_sandbox_policy();
    let file_system_sandbox_policy = effective_file_system_sandbox_policy(
        &base_file_system_sandbox_policy,
        granted_permissions.as_ref(),
    );
    let effective_additional_permissions = apply_granted_permissions_from_grants(
        codex_tool_runtime_api::ToolPermissionGrants {
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

pub(crate) fn apply_patch_payload_command(
    payload: &ToolPayload,
) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Custom { input } => Ok(input.clone()),
        _ => Err(FunctionCallError::RespondToModel(
            "apply_patch handler received unsupported payload".to_string(),
        )),
    }
}

pub(crate) fn require_environment_id(
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

pub(crate) fn file_paths_for_action(action: &ApplyPatchAction) -> Vec<AbsolutePathBuf> {
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

pub(crate) fn write_permissions_for_paths(
    file_paths: &[AbsolutePathBuf],
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
) -> Option<AdditionalPermissionProfile> {
    let write_paths = file_paths
        .iter()
        .map(|path| path.parent().unwrap_or_else(|| path.clone()).into_path_buf())
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

pub(crate) struct EffectiveAdditionalPermissions {
    pub(crate) sandbox_permissions: SandboxPermissions,
    pub(crate) additional_permissions: Option<AdditionalPermissionProfile>,
    pub(crate) permissions_preapproved: bool,
}

pub(crate) fn implicit_granted_permissions(
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<&AdditionalPermissionProfile>,
    effective_additional_permissions: &EffectiveAdditionalPermissions,
) -> Option<AdditionalPermissionProfile> {
    if !sandbox_permissions.uses_additional_permissions()
        && !matches!(sandbox_permissions, SandboxPermissions::RequireEscalated)
        && additional_permissions.is_none()
    {
        effective_additional_permissions.additional_permissions.clone()
    } else {
        None
    }
}

pub(crate) fn apply_granted_permissions_from_grants(
    grants: codex_tool_runtime_api::ToolPermissionGrants,
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

pub(crate) fn normalize_and_validate_additional_permissions(
    additional_permissions_allowed: bool,
    approval_policy: AskForApproval,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<AdditionalPermissionProfile>,
    permissions_preapproved: bool,
    _cwd: &AbsolutePathBuf,
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
