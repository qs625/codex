use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::ApprovalServiceFuture;
use codex_approval_service_api::ApplyPatchApprovalDispatch;
use codex_approval_service_api::ExecCommandApprovalDispatch;
use codex_approval_service_api::ExecCommandApprovalOutcome;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::ReviewDecision;
use codex_thread_api::ToolServiceSessionRef;
use codex_thread_api::ToolServiceTurnRef;
use codex_thread_api::ToolRuntimeSessionCapability;
use codex_thread_api::ToolRuntimeTurnCapability;
use codex_tool_runtime_api::ApplyPatchApprovalKey;
use codex_tool_runtime_api::ApplyPatchApprovalRequest;
use codex_tool_runtime_api::PermissionRequestPayload;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tool_approval_support::with_cached_approval;

#[derive(Default)]
pub struct ThreadApprovalService;

impl ThreadApprovalService {
    fn session(session: &Arc<dyn ToolServiceSessionRef>) -> Result<Arc<Session>, String> {
        session
            .clone()
            .into_any_arc()
            .downcast::<Session>()
            .map_err(|_| "approval service received unsupported session context".to_string())
    }

    fn turn(turn: &Arc<dyn ToolServiceTurnRef>) -> Result<Arc<TurnContext>, String> {
        turn.clone()
            .into_any_arc()
            .downcast::<TurnContext>()
            .map_err(|_| "approval service received unsupported turn context".to_string())
    }
}

impl ApprovalServiceApi for ThreadApprovalService {
    fn request_apply_patch_approval(
        &self,
        request: ApplyPatchApprovalDispatch,
    ) -> ApprovalServiceFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let session = Self::session(&request.session)?;
            let turn = Self::turn(&request.turn)?;
            request_apply_patch_approval(
                session,
                turn,
                request.call_id,
                request.approval_keys,
                request.approval_request,
                request.changes,
                request.permissions_preapproved,
                request.retry_reason,
            )
            .await
        })
    }

    fn request_exec_command_approval(
        &self,
        request: ExecCommandApprovalDispatch,
    ) -> ApprovalServiceFuture<'_, Result<ExecCommandApprovalOutcome, String>> {
        Box::pin(async move {
            let session = Self::session(&request.session)?;
            let turn = Self::turn(&request.turn)?;
            request_exec_command_approval(session, turn, request).await
        })
    }
}

async fn request_apply_patch_approval(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    approval_keys: Vec<ApplyPatchApprovalKey>,
    approval_request: ApplyPatchApprovalRequest,
    changes: HashMap<PathBuf, FileChange>,
    permissions_preapproved: bool,
    retry_reason: Option<String>,
) -> Result<(), String> {
    let review_with_guardian =
        turn.routes_approval_to_guardian() || session.strict_auto_review_enabled_for_turn().await;
    let decision = if review_with_guardian {
        crate::guardian::review_approval_request(
            &turn.session_arc(),
            &turn.self_arc(),
            uuid::Uuid::new_v4().to_string(),
            crate::guardian::GuardianApprovalRequest::ApplyPatch {
                id: call_id,
                cwd: approval_request.cwd,
                files: approval_request.files,
                patch: approval_request.patch,
            },
            retry_reason,
        )
        .await
    } else if permissions_preapproved && retry_reason.is_none() {
        ReviewDecision::Approved
    } else if let Some(reason) = retry_reason {
        let rx_approve = session
            .request_patch_approval(
                &turn,
                call_id,
                changes,
                Some(reason),
                /*grant_root*/ None,
        )
        .await;
        rx_approve.await.unwrap_or_default()
    } else {
        let session_for_prompt = Arc::clone(&session);
        let turn_for_prompt = Arc::clone(&turn);
        with_cached_approval(&session.services, "apply_patch", approval_keys, || async move {
            let rx_approve = session_for_prompt
                .request_patch_approval(
                    &turn_for_prompt,
                    call_id,
                    changes,
                    /*reason*/ None,
                    /*grant_root*/ None,
                )
                .await;
            rx_approve.await.unwrap_or_default()
        })
        .await
    };

    match decision {
        ReviewDecision::Approved
        | ReviewDecision::ApprovedExecpolicyAmendment { .. }
        | ReviewDecision::ApprovedForSession => Ok(()),
        ReviewDecision::Denied | ReviewDecision::Abort => Err("patch rejected by user".to_string()),
        ReviewDecision::TimedOut => Err(crate::guardian::guardian_timeout_message()),
        ReviewDecision::NetworkPolicyAmendment {
            network_policy_amendment,
        } => match network_policy_amendment.action {
            codex_protocol::protocol::NetworkPolicyRuleAction::Allow => Ok(()),
            codex_protocol::protocol::NetworkPolicyRuleAction::Deny => {
                Err("patch rejected by user".to_string())
            }
        },
    }
}

async fn request_exec_command_approval(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    request: ExecCommandApprovalDispatch,
) -> Result<ExecCommandApprovalOutcome, String> {
    let strict_auto_review = session.strict_auto_review_enabled_for_turn().await;
    let review_with_guardian = turn.routes_approval_to_guardian() || strict_auto_review;

    match request.exec_approval_requirement {
        codex_permissions_runtime::ExecApprovalRequirement::Forbidden { reason } => Err(reason),
        codex_permissions_runtime::ExecApprovalRequirement::Skip { .. } => {
            if !strict_auto_review {
                return Ok(ExecCommandApprovalOutcome::ContinueInRuntime);
            }

            let decision = crate::guardian::review_approval_request(
                &session,
                &turn,
                uuid::Uuid::new_v4().to_string(),
                crate::guardian::GuardianApprovalRequest::ExecCommand {
                    id: request.call_id,
                    command: request.command,
                    cwd: request
                        .cwd
                        .clone()
                        .try_into()
                        .map_err(|_| "exec approval received invalid cwd".to_string())?,
                    sandbox_permissions: request.sandbox_permissions,
                    additional_permissions: request.additional_permissions,
                    justification: request.justification,
                    tty: request.tty,
                },
                request.reason,
            )
            .await;
            reject_unapproved_exec_decision(decision)?;
            Ok(ExecCommandApprovalOutcome::Preapproved)
        }
        codex_permissions_runtime::ExecApprovalRequirement::NeedsApproval {
            reason,
            proposed_execpolicy_amendment,
        } => {
            if !strict_auto_review
                && let Some(decision) = session
                    .run_permission_request_hooks(
                        turn.as_ref(),
                        &request.call_id,
                        PermissionRequestPayload::bash(
                            request.hook_command.clone(),
                            request.justification.clone(),
                        ),
                    )
                    .await
            {
                match decision {
                    codex_hooks_api::PermissionRequestDecision::Allow => {
                        return Ok(ExecCommandApprovalOutcome::Preapproved);
                    }
                    codex_hooks_api::PermissionRequestDecision::Deny { message } => {
                        return Err(message);
                    }
                }
            }

            let decision = if review_with_guardian {
                let retry_reason = request.reason.clone().or(reason.clone());
                crate::guardian::review_approval_request(
                    &session,
                    &turn,
                    uuid::Uuid::new_v4().to_string(),
                    crate::guardian::GuardianApprovalRequest::ExecCommand {
                        id: request.call_id,
                        command: request.command,
                        cwd: request
                            .cwd
                            .clone()
                            .try_into()
                            .map_err(|_| "exec approval received invalid cwd".to_string())?,
                        sandbox_permissions: request.sandbox_permissions,
                        additional_permissions: request.additional_permissions.clone(),
                        justification: request.justification.clone(),
                        tty: request.tty,
                    },
                    retry_reason,
                )
                .await
            } else {
                let session_for_prompt = Arc::clone(&session);
                let turn_for_prompt = Arc::clone(&turn);
                let call_id = request.call_id.clone();
                let command = request.command.clone();
                let cwd = request
                    .cwd
                    .clone()
                    .try_into()
                    .map_err(|_| "exec approval received invalid cwd".to_string())?;
                let prompt_reason = request.reason.clone().or(reason);
                let additional_permissions = request.additional_permissions.clone();
                let network_approval_context = request.network_approval_context.clone();
                with_cached_approval(&session.services, "unified_exec", request.approval_keys, || async move {
                    session_for_prompt
                        .request_command_approval(
                            turn_for_prompt.as_ref(),
                            call_id,
                            /*approval_id*/ None,
                            command,
                            cwd,
                            prompt_reason,
                            network_approval_context,
                            proposed_execpolicy_amendment,
                            additional_permissions,
                            /*available_decisions*/ None,
                        )
                        .await
                })
                .await
            };

            reject_unapproved_exec_decision(decision)?;
            Ok(ExecCommandApprovalOutcome::Preapproved)
        }
    }
}

fn reject_unapproved_exec_decision(decision: ReviewDecision) -> Result<(), String> {
    match decision {
        ReviewDecision::Approved
        | ReviewDecision::ApprovedExecpolicyAmendment { .. }
        | ReviewDecision::ApprovedForSession => Ok(()),
        ReviewDecision::Denied | ReviewDecision::Abort => Err("rejected by user".to_string()),
        ReviewDecision::TimedOut => Err(crate::guardian::guardian_timeout_message()),
        ReviewDecision::NetworkPolicyAmendment {
            network_policy_amendment,
        } => match network_policy_amendment.action {
            codex_protocol::protocol::NetworkPolicyRuleAction::Allow => Ok(()),
            codex_protocol::protocol::NetworkPolicyRuleAction::Deny => {
                Err("rejected by user".to_string())
            }
        },
    }
}
