use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use codex_approval_service_api::ApplyPatchApprovalDispatch;
use codex_approval_service_api::ApplyPatchApprovalKey;
use codex_approval_service_api::ApplyPatchApprovalRequest;
use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::ApprovalServiceFuture;
use codex_approval_service_api::ExecCommandApprovalRequirement;
use codex_approval_service_api::ExecCommandApprovalDispatch;
use codex_approval_service_api::ExecCommandApprovalOutcome;
use codex_approval_service_api::GuardianReviewDispatch;
use codex_approval_service_api::GuardianReviewResult;
use codex_guardian::GuardianApprovalRequest;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::ReviewDecision;
use thread_service_api::PermissionRequestPayload;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;

#[derive(Default)]
pub struct ApprovalService;

fn should_use_guardian(
    turn: &dyn ThreadTurnCapability,
    strict_auto_review_enabled: bool,
) -> bool {
    (matches!(
        turn.approval_policy(),
        codex_protocol::protocol::AskForApproval::OnRequest
            | codex_protocol::protocol::AskForApproval::Granular(_)
    ) && turn.approvals_reviewer() == ApprovalsReviewer::AutoReview)
        || strict_auto_review_enabled
}

async fn request_cached_approval<T>(
    session: &dyn ThreadSessionCapability,
    tool_name: &str,
    keys: Vec<T>,
    fetch: impl std::future::Future<Output = ReviewDecision>,
) -> ReviewDecision
where
    T: serde::Serialize,
{
    if keys.is_empty() {
        let decision = fetch.await;
        session
            .record_approval_request_telemetry(tool_name, &decision)
            .await;
        return decision;
    }

    let serialized_keys = keys
        .into_iter()
        .map(|key| serde_json::to_string(&key))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string());

    let Ok(serialized_keys) = serialized_keys else {
        let decision = fetch.await;
        session
            .record_approval_request_telemetry(tool_name, &decision)
            .await;
        return decision;
    };

    let mut already_approved = true;
    for key in &serialized_keys {
        if session.cached_approval_decision(key.clone()).await
            != Some(ReviewDecision::ApprovedForSession)
        {
            already_approved = false;
            break;
        }
    }
    if already_approved {
        return ReviewDecision::ApprovedForSession;
    }

    let decision = fetch.await;
    session
        .record_approval_request_telemetry(tool_name, &decision)
        .await;
    session
        .cache_approval_decision(serialized_keys, decision.clone())
        .await;
    decision
}

impl ApprovalServiceApi for ApprovalService {
    fn request_apply_patch_approval(
        &self,
        request: ApplyPatchApprovalDispatch,
    ) -> ApprovalServiceFuture<'_, Result<(), String>> {
        Box::pin(request_apply_patch_approval(
            request.session,
            request.turn,
            request.call_id,
            request.approval_keys,
            request.approval_request,
            request.changes,
            request.permissions_preapproved,
            request.retry_reason,
        ))
    }

    fn request_exec_command_approval(
        &self,
        request: ExecCommandApprovalDispatch,
    ) -> ApprovalServiceFuture<'_, Result<ExecCommandApprovalOutcome, String>> {
        let session_api = Arc::clone(&request.session);
        let turn = Arc::clone(&request.turn);
        Box::pin(request_exec_command_approval(
            session_api,
            turn,
            request,
        ))
    }

    fn review_guardian_request(
        &self,
        request: GuardianReviewDispatch,
    ) -> ApprovalServiceFuture<'_, GuardianReviewResult> {
        Box::pin(async move {
            let review_id = request.review_id;
            let decision = crate::guardian::review_approval_request(
                request.session.as_ref(),
                request.turn.as_ref(),
                review_id.clone(),
                request.request,
                request.retry_reason,
            )
            .await;
            let decline_message = match decision {
                ReviewDecision::Denied => Some(
                    crate::guardian::guardian_rejection_message(
                        request.session.as_ref(),
                        &review_id,
                    )
                    .await,
                ),
                ReviewDecision::TimedOut => Some(crate::guardian::guardian_timeout_message()),
                ReviewDecision::Approved
                | ReviewDecision::ApprovedForSession
                | ReviewDecision::ApprovedExecpolicyAmendment { .. }
                | ReviewDecision::NetworkPolicyAmendment { .. }
                | ReviewDecision::Abort => None,
            };
            GuardianReviewResult {
                decision,
                decline_message,
            }
        })
    }
}

async fn request_apply_patch_approval(
    session_api: Arc<dyn ThreadSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    call_id: String,
    approval_keys: Vec<ApplyPatchApprovalKey>,
    approval_request: ApplyPatchApprovalRequest,
    changes: HashMap<PathBuf, FileChange>,
    permissions_preapproved: bool,
    retry_reason: Option<String>,
) -> Result<(), String> {
    let strict_auto_review = session_api.strict_auto_review_enabled_for_turn().await;
    let review_with_guardian = should_use_guardian(turn.as_ref(), strict_auto_review);
    let decision = if review_with_guardian {
        crate::guardian::review_approval_request(
            session_api.as_ref(),
            turn.as_ref(),
            uuid::Uuid::new_v4().to_string(),
            GuardianApprovalRequest::ApplyPatch {
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
        session_api
            .request_patch_approval(
                turn.as_ref(),
                call_id,
                changes,
                Some(reason),
                /*grant_root*/ None,
            )
            .await
    } else {
        request_cached_approval(
            session_api.as_ref(),
            "apply_patch",
            approval_keys,
            session_api.request_patch_approval(
                turn.as_ref(),
                call_id,
                changes,
                /*reason*/ None,
                /*grant_root*/ None,
            ),
        )
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
    session_api: Arc<dyn ThreadSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    request: ExecCommandApprovalDispatch,
) -> Result<ExecCommandApprovalOutcome, String> {
    let strict_auto_review = session_api.strict_auto_review_enabled_for_turn().await;
    let review_with_guardian = should_use_guardian(turn.as_ref(), strict_auto_review);

    match request.exec_approval_requirement {
        ExecCommandApprovalRequirement::Forbidden { reason } => Err(reason),
        ExecCommandApprovalRequirement::Skip { .. } => {
            if !strict_auto_review {
                return Ok(ExecCommandApprovalOutcome::ContinueInRuntime);
            }

            let decision = crate::guardian::review_approval_request(
                    session_api.as_ref(),
                    turn.as_ref(),
                    uuid::Uuid::new_v4().to_string(),
                    GuardianApprovalRequest::ExecCommand {
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
            reject_unapproved_exec_decision(decision, session_api.as_ref())?;
            Ok(ExecCommandApprovalOutcome::Preapproved)
        }
        ExecCommandApprovalRequirement::NeedsApproval {
            reason,
            proposed_execpolicy_amendment,
        } => {
            if !strict_auto_review
                && let Some(decision) = session_api
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
                        session_api.as_ref(),
                        turn.as_ref(),
                        uuid::Uuid::new_v4().to_string(),
                        GuardianApprovalRequest::ExecCommand {
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
                request_cached_approval(
                    session_api.as_ref(),
                    "unified_exec",
                    request.approval_keys,
                    session_api.request_command_approval(
                        turn.as_ref(),
                        call_id,
                        /*approval_id*/ None,
                        command,
                        cwd,
                        prompt_reason,
                        network_approval_context,
                        proposed_execpolicy_amendment,
                        additional_permissions,
                        /*available_decisions*/ None,
                    ),
                )
                .await
            };

            reject_unapproved_exec_decision(decision, session_api.as_ref())?;
            Ok(ExecCommandApprovalOutcome::Preapproved)
        }
    }
}

fn reject_unapproved_exec_decision(
    decision: ReviewDecision,
    _session: &dyn ThreadSessionCapability,
) -> Result<(), String> {
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
