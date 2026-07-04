use std::sync::Arc;

use codex_analytics_api::GuardianApprovalRequestSource;
use codex_analytics_api::GuardianReviewAnalyticsResult;
use codex_analytics_api::GuardianReviewDecision;
use codex_analytics_api::GuardianReviewFailureReason;
use codex_analytics_api::GuardianReviewTerminalStatus;
use codex_analytics_api::GuardianReviewTrackContext;
use codex_approval_service_api::ApprovalSessionCapability;
pub use codex_approval_service_api::GUARDIAN_REVIEWER_NAME;
use codex_approval_service_api::ReviewRejectionRecord;
use codex_approval_service_api::ReviewRuntimeError;
use codex_approval_service_api::ReviewRuntimeOutcome;
pub use codex_approval_service_api::is_guardian_reviewer_source;
pub use codex_approval_service_api::routes_approval_to_guardian;
use codex_guardian::GuardianApprovalRequest;
use codex_guardian::guardian_assessment_action;
use codex_guardian::guardian_request_target_item_id;
use codex_guardian::guardian_request_turn_id;
use codex_guardian::guardian_reviewed_action;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadTurnCapability;

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum GuardianApprovalRequestPayload {
    Shell {
        id: String,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        sandbox_permissions: protocol::models::SandboxPermissions,
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
        justification: Option<String>,
    },
    ExecCommand {
        id: String,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        sandbox_permissions: protocol::models::SandboxPermissions,
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
        justification: Option<String>,
        tty: bool,
    },
    #[cfg(unix)]
    Execve {
        id: String,
        source: protocol::approvals::GuardianCommandSource,
        program: String,
        argv: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
    },
    ApplyPatch {
        id: String,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        files: Vec<codex_utils_absolute_path::AbsolutePathBuf>,
        patch: String,
    },
    NetworkAccess {
        id: String,
        turn_id: String,
        target: String,
        host: String,
        protocol: protocol::approvals::NetworkApprovalProtocol,
        port: u16,
        trigger: Option<codex_guardian::GuardianNetworkAccessTrigger>,
    },
    McpToolCall {
        id: String,
        server: String,
        tool_name: String,
        arguments: Option<serde_json::Value>,
        connector_id: Option<String>,
        connector_name: Option<String>,
        connector_description: Option<String>,
        tool_title: Option<String>,
        tool_description: Option<String>,
        annotations: Option<codex_guardian::GuardianMcpAnnotations>,
    },
    RequestPermissions {
        id: String,
        turn_id: String,
        reason: Option<String>,
        permissions: protocol::request_permissions::RequestPermissionProfile,
    },
}

pub fn new_guardian_review_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub async fn guardian_rejection_message(
    session: &dyn ApprovalSessionCapability,
    review_id: &str,
) -> String {
    let rejection = session.take_review_rejection(review_id).await;
    codex_approval_service_api::guardian_rejection_message_from_rationale(
        rejection
            .as_ref()
            .map(|rejection| rejection.rationale.as_str()),
    )
}

pub fn guardian_rejection_message_from_rationale(rationale: Option<&str>) -> String {
    codex_approval_service_api::guardian_rejection_message_from_rationale(rationale)
}

pub fn guardian_timeout_message() -> String {
    codex_approval_service_api::guardian_timeout_message()
}

pub async fn review_approval_request(
    session: &dyn ApprovalSessionCapability,
    turn: &dyn ThreadTurnCapability,
    review_id: String,
    request: GuardianApprovalRequest,
    retry_reason: Option<String>,
) -> protocol::protocol::ReviewDecision {
    run_guardian_review(
        session,
        turn,
        review_id,
        request,
        retry_reason,
        GuardianApprovalRequestSource::MainTurn,
    )
    .await
}

pub async fn review_approval_request_with_source(
    session: &dyn ApprovalSessionCapability,
    turn: &dyn ThreadTurnCapability,
    review_id: String,
    request: GuardianApprovalRequest,
    retry_reason: Option<String>,
    approval_request_source: GuardianApprovalRequestSource,
) -> protocol::protocol::ReviewDecision {
    run_guardian_review(
        session,
        turn,
        review_id,
        request,
        retry_reason,
        approval_request_source,
    )
    .await
}

pub fn spawn_approval_request_review(
    session: Arc<dyn ApprovalSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    review_id: String,
    request: GuardianApprovalRequest,
    retry_reason: Option<String>,
    approval_request_source: GuardianApprovalRequestSource,
    cancel_token: tokio_util::sync::CancellationToken,
) -> tokio::sync::oneshot::Receiver<protocol::protocol::ReviewDecision> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            let _ = tx.send(protocol::protocol::ReviewDecision::Denied);
            return;
        };
        let decision = runtime.block_on(async move {
            tokio::select! {
                _ = cancel_token.cancelled() => protocol::protocol::ReviewDecision::Abort,
                decision = run_guardian_review(
                    session.as_ref(),
                    turn.as_ref(),
                    review_id,
                    request,
                    retry_reason,
                    approval_request_source,
                ) => decision,
            }
        });
        let _ = tx.send(decision);
    });
    rx
}

async fn run_guardian_review(
    session: &dyn ApprovalSessionCapability,
    turn: &dyn ThreadTurnCapability,
    review_id: String,
    request: GuardianApprovalRequest,
    retry_reason: Option<String>,
    approval_request_source: GuardianApprovalRequestSource,
) -> protocol::protocol::ReviewDecision {
    let target_item_id = guardian_request_target_item_id(&request).map(str::to_string);
    let assessment_turn_id =
        guardian_request_turn_id(&request, turn.runtime_turn_id_str()).to_string();
    let action_summary = guardian_assessment_action(&request);
    let review_tracking = GuardianReviewTrackContext::new(
        ApprovalSessionCapability::conversation_id(session).to_string(),
        assessment_turn_id.clone(),
        review_id.clone(),
        target_item_id.clone(),
        approval_request_source,
        guardian_reviewed_action(&request),
        codex_guardian::GUARDIAN_REVIEW_TIMEOUT.as_millis() as u64,
    );
    let started_at_ms: i64 = review_tracking.started_at_ms.try_into().unwrap_or_default();

    turn.emit_event(protocol::protocol::EventMsg::GuardianAssessment(
        protocol::protocol::GuardianAssessmentEvent {
            id: review_id.clone(),
            target_item_id: target_item_id.clone(),
            turn_id: assessment_turn_id.clone(),
            started_at_ms,
            completed_at_ms: None,
            status: protocol::protocol::GuardianAssessmentStatus::InProgress,
            risk_level: None,
            user_authorization: None,
            rationale: None,
            decision_source: None,
            action: action_summary.clone(),
        },
    ))
    .await;

    let runtime_result = session
        .run_review_session(
            turn,
            match serialize_guardian_request(request) {
                Ok(request) => request,
                Err(message) => {
                    return handle_guardian_runtime_error(
                        session,
                        turn,
                        review_tracking,
                        review_id,
                        target_item_id,
                        assessment_turn_id,
                        started_at_ms,
                        action_summary,
                        now_unix_timestamp_ms().try_into().unwrap_or_default(),
                        now_unix_timestamp_ms(),
                        GuardianReviewAnalyticsResult::without_session(),
                        ReviewRuntimeError::PromptBuild { message },
                    )
                    .await;
                }
            },
            retry_reason,
        )
        .await;

    let completed_at_ms_u64 = now_unix_timestamp_ms();
    let completed_at_ms: i64 = completed_at_ms_u64.try_into().unwrap_or_default();
    match runtime_result.outcome {
        ReviewRuntimeOutcome::Completed(assessment) => {
            let approved = matches!(
                assessment.outcome,
                protocol::protocol::GuardianAssessmentOutcome::Allow
            );
            session
                .track_review_analytics(
                    review_tracking,
                    GuardianReviewAnalyticsResult {
                        decision: if approved {
                            GuardianReviewDecision::Approved
                        } else {
                            GuardianReviewDecision::Denied
                        },
                        terminal_status: if approved {
                            GuardianReviewTerminalStatus::Approved
                        } else {
                            GuardianReviewTerminalStatus::Denied
                        },
                        failure_reason: None,
                        risk_level: Some(assessment.risk_level),
                        user_authorization: Some(assessment.user_authorization),
                        outcome: Some(assessment.outcome),
                        ..runtime_result.analytics_result
                    },
                    completed_at_ms_u64,
                )
                .await;

            let verdict = if approved { "approved" } else { "denied" };
            let user_authorization = match assessment.user_authorization {
                protocol::protocol::GuardianUserAuthorization::Unknown => "unknown",
                protocol::protocol::GuardianUserAuthorization::Low => "low",
                protocol::protocol::GuardianUserAuthorization::Medium => "medium",
                protocol::protocol::GuardianUserAuthorization::High => "high",
            };
            let warning = format!(
                "Automatic approval review {verdict} (risk: {}, authorization: {user_authorization}): {}",
                guardian_risk_level_str(assessment.risk_level),
                assessment.rationale
            );
            turn.emit_event(protocol::protocol::EventMsg::GuardianWarning(
                protocol::protocol::WarningEvent { message: warning },
            ))
            .await;

            let status = if approved {
                protocol::protocol::GuardianAssessmentStatus::Approved
            } else {
                protocol::protocol::GuardianAssessmentStatus::Denied
            };
            session
                .set_review_rejection(
                    review_id.clone(),
                    (!approved).then_some(ReviewRejectionRecord {
                        rationale: assessment.rationale.clone(),
                        source: protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    }),
                )
                .await;
            turn.emit_event(protocol::protocol::EventMsg::GuardianAssessment(
                protocol::protocol::GuardianAssessmentEvent {
                    id: review_id,
                    target_item_id,
                    turn_id: assessment_turn_id.clone(),
                    started_at_ms,
                    completed_at_ms: Some(completed_at_ms),
                    status,
                    risk_level: Some(assessment.risk_level),
                    user_authorization: Some(assessment.user_authorization),
                    rationale: Some(assessment.rationale.clone()),
                    decision_source: Some(
                        protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    ),
                    action: action_summary,
                },
            ))
            .await;

            if approved {
                session
                    .record_review_non_rejection(&assessment_turn_id)
                    .await;
                protocol::protocol::ReviewDecision::Approved
            } else {
                session
                    .record_review_rejection(turn, &assessment_turn_id)
                    .await;
                protocol::protocol::ReviewDecision::Denied
            }
        }
        ReviewRuntimeOutcome::Error(error) => {
            let decision = handle_guardian_runtime_error(
                session,
                turn,
                review_tracking,
                review_id,
                target_item_id,
                assessment_turn_id,
                started_at_ms,
                action_summary,
                completed_at_ms,
                completed_at_ms_u64,
                runtime_result.analytics_result,
                error,
            )
            .await;
            decision
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_guardian_runtime_error(
    session: &dyn ApprovalSessionCapability,
    turn: &dyn ThreadTurnCapability,
    review_tracking: GuardianReviewTrackContext,
    review_id: String,
    target_item_id: Option<String>,
    assessment_turn_id: String,
    started_at_ms: i64,
    action_summary: protocol::approvals::GuardianAssessmentAction,
    completed_at_ms: i64,
    completed_at_ms_u64: u64,
    analytics_result: GuardianReviewAnalyticsResult,
    error: ReviewRuntimeError,
) -> protocol::protocol::ReviewDecision {
    match error {
        ReviewRuntimeError::Timeout => {
            let rationale =
                "Automatic approval review timed out while evaluating the requested approval."
                    .to_string();
            session
                .track_review_analytics(
                    review_tracking,
                    GuardianReviewAnalyticsResult {
                        decision: GuardianReviewDecision::Denied,
                        terminal_status: GuardianReviewTerminalStatus::TimedOut,
                        failure_reason: Some(GuardianReviewFailureReason::Timeout),
                        ..analytics_result
                    },
                    completed_at_ms_u64,
                )
                .await;
            turn.emit_event(protocol::protocol::EventMsg::GuardianWarning(
                protocol::protocol::WarningEvent {
                    message: rationale.clone(),
                },
            ))
            .await;
            turn.emit_event(protocol::protocol::EventMsg::GuardianAssessment(
                protocol::protocol::GuardianAssessmentEvent {
                    id: review_id,
                    target_item_id,
                    turn_id: assessment_turn_id.clone(),
                    started_at_ms,
                    completed_at_ms: Some(completed_at_ms),
                    status: protocol::protocol::GuardianAssessmentStatus::TimedOut,
                    risk_level: None,
                    user_authorization: None,
                    rationale: Some(rationale),
                    decision_source: Some(
                        protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    ),
                    action: action_summary,
                },
            ))
            .await;
            session
                .record_review_non_rejection(&assessment_turn_id)
                .await;
            protocol::protocol::ReviewDecision::TimedOut
        }
        ReviewRuntimeError::Cancelled => {
            session
                .track_review_analytics(
                    review_tracking,
                    GuardianReviewAnalyticsResult {
                        decision: GuardianReviewDecision::Aborted,
                        terminal_status: GuardianReviewTerminalStatus::Aborted,
                        failure_reason: Some(GuardianReviewFailureReason::Cancelled),
                        ..analytics_result
                    },
                    completed_at_ms_u64,
                )
                .await;
            turn.emit_event(protocol::protocol::EventMsg::GuardianAssessment(
                protocol::protocol::GuardianAssessmentEvent {
                    id: review_id,
                    target_item_id,
                    turn_id: assessment_turn_id.clone(),
                    started_at_ms,
                    completed_at_ms: Some(completed_at_ms),
                    status: protocol::protocol::GuardianAssessmentStatus::Aborted,
                    risk_level: None,
                    user_authorization: None,
                    rationale: None,
                    decision_source: Some(
                        protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    ),
                    action: action_summary,
                },
            ))
            .await;
            session
                .record_review_non_rejection(&assessment_turn_id)
                .await;
            protocol::protocol::ReviewDecision::Abort
        }
        ReviewRuntimeError::PromptBuild { message } => {
            let rationale = format!("Automatic approval review failed: {message}");
            session
                .track_review_analytics(
                    review_tracking,
                    GuardianReviewAnalyticsResult {
                        decision: GuardianReviewDecision::Denied,
                        terminal_status: GuardianReviewTerminalStatus::FailedClosed,
                        failure_reason: Some(GuardianReviewFailureReason::PromptBuildError),
                        ..analytics_result
                    },
                    completed_at_ms_u64,
                )
                .await;
            let warning = format!(
                "Automatic approval review denied (risk: high, authorization: unknown): {rationale}"
            );
            turn.emit_event(protocol::protocol::EventMsg::GuardianWarning(
                protocol::protocol::WarningEvent { message: warning },
            ))
            .await;
            session
                .set_review_rejection(
                    review_id.clone(),
                    Some(ReviewRejectionRecord {
                        rationale: rationale.clone(),
                        source: protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    }),
                )
                .await;
            turn.emit_event(protocol::protocol::EventMsg::GuardianAssessment(
                protocol::protocol::GuardianAssessmentEvent {
                    id: review_id,
                    target_item_id,
                    turn_id: assessment_turn_id.clone(),
                    started_at_ms,
                    completed_at_ms: Some(completed_at_ms),
                    status: protocol::protocol::GuardianAssessmentStatus::Denied,
                    risk_level: Some(protocol::protocol::GuardianRiskLevel::High),
                    user_authorization: Some(
                        protocol::protocol::GuardianUserAuthorization::Unknown,
                    ),
                    rationale: Some(rationale),
                    decision_source: Some(
                        protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    ),
                    action: action_summary,
                },
            ))
            .await;
            session
                .record_review_non_rejection(&assessment_turn_id)
                .await;
            protocol::protocol::ReviewDecision::Denied
        }
        ReviewRuntimeError::Session { message } => {
            let rationale = format!("Automatic approval review failed: {message}");
            session
                .track_review_analytics(
                    review_tracking,
                    GuardianReviewAnalyticsResult {
                        decision: GuardianReviewDecision::Denied,
                        terminal_status: GuardianReviewTerminalStatus::FailedClosed,
                        failure_reason: Some(GuardianReviewFailureReason::SessionError),
                        ..analytics_result
                    },
                    completed_at_ms_u64,
                )
                .await;
            let warning = format!(
                "Automatic approval review denied (risk: high, authorization: unknown): {rationale}"
            );
            turn.emit_event(protocol::protocol::EventMsg::GuardianWarning(
                protocol::protocol::WarningEvent { message: warning },
            ))
            .await;
            session
                .set_review_rejection(
                    review_id.clone(),
                    Some(ReviewRejectionRecord {
                        rationale: rationale.clone(),
                        source: protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    }),
                )
                .await;
            turn.emit_event(protocol::protocol::EventMsg::GuardianAssessment(
                protocol::protocol::GuardianAssessmentEvent {
                    id: review_id,
                    target_item_id,
                    turn_id: assessment_turn_id.clone(),
                    started_at_ms,
                    completed_at_ms: Some(completed_at_ms),
                    status: protocol::protocol::GuardianAssessmentStatus::Denied,
                    risk_level: Some(protocol::protocol::GuardianRiskLevel::High),
                    user_authorization: Some(
                        protocol::protocol::GuardianUserAuthorization::Unknown,
                    ),
                    rationale: Some(rationale),
                    decision_source: Some(
                        protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    ),
                    action: action_summary,
                },
            ))
            .await;
            session
                .record_review_non_rejection(&assessment_turn_id)
                .await;
            protocol::protocol::ReviewDecision::Denied
        }
        ReviewRuntimeError::Parse { message } => {
            let rationale = format!("Automatic approval review failed: {message}");
            session
                .track_review_analytics(
                    review_tracking,
                    GuardianReviewAnalyticsResult {
                        decision: GuardianReviewDecision::Denied,
                        terminal_status: GuardianReviewTerminalStatus::FailedClosed,
                        failure_reason: Some(GuardianReviewFailureReason::ParseError),
                        ..analytics_result
                    },
                    completed_at_ms_u64,
                )
                .await;
            let warning = format!(
                "Automatic approval review denied (risk: high, authorization: unknown): {rationale}"
            );
            turn.emit_event(protocol::protocol::EventMsg::GuardianWarning(
                protocol::protocol::WarningEvent { message: warning },
            ))
            .await;
            session
                .set_review_rejection(
                    review_id.clone(),
                    Some(ReviewRejectionRecord {
                        rationale: rationale.clone(),
                        source: protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    }),
                )
                .await;
            turn.emit_event(protocol::protocol::EventMsg::GuardianAssessment(
                protocol::protocol::GuardianAssessmentEvent {
                    id: review_id,
                    target_item_id,
                    turn_id: assessment_turn_id.clone(),
                    started_at_ms,
                    completed_at_ms: Some(completed_at_ms),
                    status: protocol::protocol::GuardianAssessmentStatus::Denied,
                    risk_level: Some(protocol::protocol::GuardianRiskLevel::High),
                    user_authorization: Some(
                        protocol::protocol::GuardianUserAuthorization::Unknown,
                    ),
                    rationale: Some(rationale),
                    decision_source: Some(
                        protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                    ),
                    action: action_summary,
                },
            ))
            .await;
            session
                .record_review_non_rejection(&assessment_turn_id)
                .await;
            protocol::protocol::ReviewDecision::Denied
        }
    }
}

fn guardian_risk_level_str(level: protocol::protocol::GuardianRiskLevel) -> &'static str {
    match level {
        protocol::protocol::GuardianRiskLevel::Low => "low",
        protocol::protocol::GuardianRiskLevel::Medium => "medium",
        protocol::protocol::GuardianRiskLevel::High => "high",
        protocol::protocol::GuardianRiskLevel::Critical => "critical",
    }
}

fn serialize_guardian_request(
    request: GuardianApprovalRequest,
) -> Result<serde_json::Value, String> {
    let payload = match request {
        GuardianApprovalRequest::Shell {
            id,
            command,
            cwd,
            sandbox_permissions,
            additional_permissions,
            justification,
        } => GuardianApprovalRequestPayload::Shell {
            id,
            command,
            cwd,
            sandbox_permissions,
            additional_permissions,
            justification,
        },
        GuardianApprovalRequest::ExecCommand {
            id,
            command,
            cwd,
            sandbox_permissions,
            additional_permissions,
            justification,
            tty,
        } => GuardianApprovalRequestPayload::ExecCommand {
            id,
            command,
            cwd,
            sandbox_permissions,
            additional_permissions,
            justification,
            tty,
        },
        #[cfg(unix)]
        GuardianApprovalRequest::Execve {
            id,
            source,
            program,
            argv,
            cwd,
            additional_permissions,
        } => GuardianApprovalRequestPayload::Execve {
            id,
            source,
            program,
            argv,
            cwd,
            additional_permissions,
        },
        GuardianApprovalRequest::ApplyPatch {
            id,
            cwd,
            files,
            patch,
        } => GuardianApprovalRequestPayload::ApplyPatch {
            id,
            cwd,
            files,
            patch,
        },
        GuardianApprovalRequest::NetworkAccess {
            id,
            turn_id,
            target,
            host,
            protocol,
            port,
            trigger,
        } => GuardianApprovalRequestPayload::NetworkAccess {
            id,
            turn_id,
            target,
            host,
            protocol,
            port,
            trigger,
        },
        GuardianApprovalRequest::McpToolCall {
            id,
            server,
            tool_name,
            arguments,
            connector_id,
            connector_name,
            connector_description,
            tool_title,
            tool_description,
            annotations,
        } => GuardianApprovalRequestPayload::McpToolCall {
            id,
            server,
            tool_name,
            arguments,
            connector_id,
            connector_name,
            connector_description,
            tool_title,
            tool_description,
            annotations,
        },
        GuardianApprovalRequest::RequestPermissions {
            id,
            turn_id,
            reason,
            permissions,
        } => GuardianApprovalRequestPayload::RequestPermissions {
            id,
            turn_id,
            reason,
            permissions,
        },
    };
    serde_json::to_value(payload).map_err(|err| err.to_string())
}

fn now_unix_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or_default()
}
