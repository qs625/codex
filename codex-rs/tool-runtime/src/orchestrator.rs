use crate::flat_tool_name;
use codex_hooks_api::PermissionRequestDecision;
use codex_metrics_api::ToolDecisionSource;
use codex_protocol::approvals::NetworkApprovalContext;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::network_policy::NetworkPolicyDecisionPayload;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::NetworkPolicyRuleAction;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxType;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_tool_runtime_api::ApprovalCtx;
use codex_tool_runtime_api::ExecApprovalRequirement;
use codex_tool_runtime_api::NetworkApprovalMode;
use codex_tool_runtime_api::SandboxAttempt;
use codex_tool_runtime_api::SandboxOverride;
use codex_tool_runtime_api::ToolCtx;
use codex_tool_runtime_api::ToolError;
use codex_tool_runtime_api::ToolRuntime;
use codex_tool_runtime_api::default_exec_approval_requirement;
use codex_tool_runtime_api::sandbox_override_for_first_attempt;

pub use codex_tool_runtime_api::OrchestratorRunResult;
pub use codex_tool_runtime_api::ToolOrchestratorHost;
pub use codex_tool_runtime_api::ToolSandboxContext;

pub struct ToolOrchestrator<Host> {
    sandbox_runtime: SharedSandboxRuntime,
    host: Host,
}

impl<Host> ToolOrchestrator<Host> {
    pub fn new(host: Host, sandbox_runtime: SharedSandboxRuntime) -> Self {
        Self {
            sandbox_runtime,
            host,
        }
    }

    async fn run_attempt<Rq, Out, T, Session, Turn, Trigger>(
        &self,
        tool: &mut T,
        req: &Rq,
        tool_ctx: &ToolCtx<Session, Turn>,
        turn_id: &str,
        attempt: &SandboxAttempt<'_>,
        managed_network_active: bool,
    ) -> (
        Result<Out, ToolError>,
        Option<Host::DeferredNetworkApproval>,
    )
    where
        T: ToolRuntime<Rq, Out, Session = Session, Turn = Turn, NetworkApprovalTrigger = Trigger>,
        Session: Clone,
        Turn: Clone,
        Host: ToolOrchestratorHost<Session, Turn, Trigger>,
    {
        let network_approval = self
            .host
            .begin_network_approval(
                &tool_ctx.session,
                turn_id,
                managed_network_active,
                tool.network_approval_spec(req, tool_ctx),
            )
            .await;

        let attempt_tool_ctx = ToolCtx {
            session: tool_ctx.session.clone(),
            turn: tool_ctx.turn.clone(),
            call_id: tool_ctx.call_id.clone(),
            tool_name: tool_ctx.tool_name.clone(),
        };
        let attempt_with_network_approval = SandboxAttempt {
            sandbox: attempt.sandbox,
            permissions: attempt.permissions,
            enforce_managed_network: attempt.enforce_managed_network,
            sandbox_runtime: attempt.sandbox_runtime,
            sandbox_cwd: attempt.sandbox_cwd,
            codex_linux_sandbox_exe: attempt.codex_linux_sandbox_exe,
            use_legacy_landlock: attempt.use_legacy_landlock,
            windows_sandbox_level: attempt.windows_sandbox_level,
            windows_sandbox_private_desktop: attempt.windows_sandbox_private_desktop,
            network_denial_cancellation_token: network_approval
                .as_ref()
                .map(|active| self.host.active_network_approval_cancellation_token(active)),
        };
        let run_result = tool
            .run(req, &attempt_with_network_approval, &attempt_tool_ctx)
            .await;

        let Some(network_approval) = network_approval else {
            return (run_result, None);
        };

        match self.host.active_network_approval_mode(&network_approval) {
            NetworkApprovalMode::Immediate => {
                let finalize_result = self
                    .host
                    .finish_immediate_network_approval(&tool_ctx.session, network_approval)
                    .await;
                if let Err(err) = finalize_result {
                    return (Err(err), None);
                }
                (run_result, None)
            }
            NetworkApprovalMode::Deferred => {
                let deferred = self.host.into_deferred_network_approval(network_approval);
                if run_result.is_err() {
                    let finalize_result = self
                        .host
                        .finish_deferred_network_approval(&tool_ctx.session, deferred)
                        .await;
                    if let Err(err) = finalize_result {
                        return (Err(err), None);
                    }
                    return (run_result, None);
                }
                (run_result, deferred)
            }
        }
    }

    pub async fn run<Rq, Out, T, Session, Turn, Trigger>(
        &mut self,
        tool: &mut T,
        req: &Rq,
        tool_ctx: &ToolCtx<Session, Turn>,
        sandbox_context: &ToolSandboxContext,
        approval_policy: AskForApproval,
    ) -> Result<OrchestratorRunResult<Out, Host::DeferredNetworkApproval>, ToolError>
    where
        T: ToolRuntime<Rq, Out, Session = Session, Turn = Turn, NetworkApprovalTrigger = Trigger>,
        Session: Clone,
        Turn: Clone,
        Host: ToolOrchestratorHost<Session, Turn, Trigger>,
    {
        let telemetry = sandbox_context.telemetry.clone();
        let otel_tn = flat_tool_name(&tool_ctx.tool_name).into_owned();
        let otel_ci = &tool_ctx.call_id;
        let strict_auto_review = self
            .host
            .strict_auto_review_enabled_for_turn(&tool_ctx.session)
            .await;
        let use_guardian =
            self.host.routes_approval_to_guardian(&tool_ctx.turn) || strict_auto_review;

        let mut already_approved = false;

        let file_system_sandbox_policy = sandbox_context.file_system_sandbox_policy.clone();
        let network_sandbox_policy = sandbox_context.network_sandbox_policy;
        let requirement = tool.exec_approval_requirement(req).unwrap_or_else(|| {
            default_exec_approval_requirement(approval_policy, &file_system_sandbox_policy)
        });
        match &requirement {
            ExecApprovalRequirement::Skip { .. } => {
                if strict_auto_review {
                    let guardian_review_id = Some(self.host.new_guardian_review_id());
                    let approval_ctx = ApprovalCtx {
                        session: &tool_ctx.session,
                        turn: &tool_ctx.turn,
                        call_id: &tool_ctx.call_id,
                        guardian_review_id: guardian_review_id.clone(),
                        retry_reason: None,
                        network_approval_context: None,
                    };
                    let decision = self
                        .request_approval(
                            tool,
                            req,
                            tool_ctx.call_id.as_str(),
                            approval_ctx,
                            tool_ctx,
                            /*evaluate_permission_request_hooks*/ false,
                            telemetry.as_ref(),
                        )
                        .await?;
                    self.reject_if_not_approved(tool_ctx, guardian_review_id.as_deref(), decision)
                        .await?;
                    already_approved = true;
                } else {
                    already_approved = tool.approval_preapproved(req);
                    telemetry.tool_decision(
                        &otel_tn,
                        otel_ci,
                        &ReviewDecision::Approved,
                        ToolDecisionSource::Config,
                    );
                }
            }
            ExecApprovalRequirement::Forbidden { reason } => {
                return Err(ToolError::Rejected(reason.clone()));
            }
            ExecApprovalRequirement::NeedsApproval { reason, .. } => {
                let guardian_review_id = use_guardian.then(|| self.host.new_guardian_review_id());
                let approval_ctx = ApprovalCtx {
                    session: &tool_ctx.session,
                    turn: &tool_ctx.turn,
                    call_id: &tool_ctx.call_id,
                    guardian_review_id: guardian_review_id.clone(),
                    retry_reason: reason.clone(),
                    network_approval_context: None,
                };
                let decision = self
                    .request_approval(
                        tool,
                        req,
                        tool_ctx.call_id.as_str(),
                        approval_ctx,
                        tool_ctx,
                        /*evaluate_permission_request_hooks*/ !strict_auto_review,
                        telemetry.as_ref(),
                    )
                    .await?;

                self.reject_if_not_approved(tool_ctx, guardian_review_id.as_deref(), decision)
                    .await?;
                already_approved = true;
            }
        }

        let sandbox_override = sandbox_override_for_first_attempt(
            tool.sandbox_permissions(req),
            &requirement,
            &file_system_sandbox_policy,
        );
        let managed_network_active = sandbox_context.managed_network_active;
        let initial_sandbox = match sandbox_override {
            SandboxOverride::BypassSandboxFirstAttempt => SandboxType::None,
            SandboxOverride::NoOverride => self.sandbox_runtime.select_initial(
                &file_system_sandbox_policy,
                network_sandbox_policy,
                tool.sandbox_preference(),
                sandbox_context.windows_sandbox_level,
                managed_network_active,
            ),
        };

        let sandbox_cwd = tool.sandbox_cwd(req).unwrap_or(&sandbox_context.cwd);
        let initial_attempt = SandboxAttempt {
            sandbox: initial_sandbox,
            permissions: &sandbox_context.permission_profile,
            enforce_managed_network: managed_network_active,
            sandbox_runtime: self.sandbox_runtime.as_ref(),
            sandbox_cwd,
            codex_linux_sandbox_exe: sandbox_context.codex_linux_sandbox_exe.as_ref(),
            use_legacy_landlock: sandbox_context.use_legacy_landlock,
            windows_sandbox_level: sandbox_context.windows_sandbox_level,
            windows_sandbox_private_desktop: sandbox_context.windows_sandbox_private_desktop,
            network_denial_cancellation_token: None,
        };

        let (first_result, first_deferred_network_approval) = self
            .run_attempt(
                tool,
                req,
                tool_ctx,
                &sandbox_context.turn_id,
                &initial_attempt,
                managed_network_active,
            )
            .await;
        match first_result {
            Ok(out) => Ok(OrchestratorRunResult {
                output: out,
                deferred_network_approval: first_deferred_network_approval,
            }),
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                output,
                network_policy_decision,
            }))) => {
                let network_approval_context = if managed_network_active {
                    network_policy_decision
                        .as_ref()
                        .and_then(network_approval_context_from_payload)
                } else {
                    None
                };
                if network_policy_decision.is_some() && network_approval_context.is_none() {
                    return Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                        output,
                        network_policy_decision,
                    })));
                }
                if !tool.escalate_on_failure() {
                    return Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                        output,
                        network_policy_decision,
                    })));
                }
                if !tool.wants_no_sandbox_approval(approval_policy) {
                    let allow_on_request_network_prompt =
                        matches!(approval_policy, AskForApproval::OnRequest)
                            && network_approval_context.is_some()
                            && matches!(
                                default_exec_approval_requirement(
                                    approval_policy,
                                    &file_system_sandbox_policy
                                ),
                                ExecApprovalRequirement::NeedsApproval { .. }
                            );
                    if !allow_on_request_network_prompt {
                        return Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                            output,
                            network_policy_decision,
                        })));
                    }
                }
                let retry_reason =
                    if let Some(network_approval_context) = network_approval_context.as_ref() {
                        format!(
                            "Network access to \"{}\" is blocked by policy.",
                            network_approval_context.host
                        )
                    } else {
                        build_denial_reason_from_output(output.as_ref())
                    };

                let bypass_retry_approval = !strict_auto_review
                    && tool.should_bypass_approval(approval_policy, already_approved)
                    && network_approval_context.is_none();
                if !bypass_retry_approval {
                    let guardian_review_id =
                        use_guardian.then(|| self.host.new_guardian_review_id());
                    let approval_ctx = ApprovalCtx {
                        session: &tool_ctx.session,
                        turn: &tool_ctx.turn,
                        call_id: &tool_ctx.call_id,
                        guardian_review_id: guardian_review_id.clone(),
                        retry_reason: Some(retry_reason),
                        network_approval_context: network_approval_context.clone(),
                    };

                    let permission_request_run_id = format!("{}:retry", tool_ctx.call_id);
                    let decision = self
                        .request_approval(
                            tool,
                            req,
                            &permission_request_run_id,
                            approval_ctx,
                            tool_ctx,
                            /*evaluate_permission_request_hooks*/ !strict_auto_review,
                            telemetry.as_ref(),
                        )
                        .await?;

                    self.reject_if_not_approved(tool_ctx, guardian_review_id.as_deref(), decision)
                        .await?;
                }

                let escalated_attempt = SandboxAttempt {
                    sandbox: SandboxType::None,
                    permissions: &sandbox_context.permission_profile,
                    enforce_managed_network: managed_network_active,
                    sandbox_runtime: self.sandbox_runtime.as_ref(),
                    sandbox_cwd,
                    codex_linux_sandbox_exe: None,
                    use_legacy_landlock: sandbox_context.use_legacy_landlock,
                    windows_sandbox_level: sandbox_context.windows_sandbox_level,
                    windows_sandbox_private_desktop: sandbox_context
                        .windows_sandbox_private_desktop,
                    network_denial_cancellation_token: None,
                };

                let (retry_result, retry_deferred_network_approval) = self
                    .run_attempt(
                        tool,
                        req,
                        tool_ctx,
                        &sandbox_context.turn_id,
                        &escalated_attempt,
                        managed_network_active,
                    )
                    .await;
                retry_result.map(|output| OrchestratorRunResult {
                    output,
                    deferred_network_approval: retry_deferred_network_approval,
                })
            }
            Err(err) => Err(err),
        }
    }

    async fn request_approval<Rq, Out, T, Session, Turn, Trigger>(
        &self,
        tool: &mut T,
        req: &Rq,
        permission_request_run_id: &str,
        approval_ctx: ApprovalCtx<'_, Session, Turn>,
        tool_ctx: &ToolCtx<Session, Turn>,
        evaluate_permission_request_hooks: bool,
        telemetry: &dyn codex_session_telemetry_api::SessionTelemetry,
    ) -> Result<ReviewDecision, ToolError>
    where
        T: ToolRuntime<Rq, Out, Session = Session, Turn = Turn, NetworkApprovalTrigger = Trigger>,
        Host: ToolOrchestratorHost<Session, Turn, Trigger>,
    {
        if evaluate_permission_request_hooks
            && let Some(permission_request) = tool.permission_request_payload(req)
        {
            let tool_name = flat_tool_name(&tool_ctx.tool_name);
            match self
                .host
                .run_permission_request_hooks(
                    approval_ctx.session,
                    approval_ctx.turn,
                    permission_request_run_id,
                    permission_request,
                )
                .await
            {
                Some(PermissionRequestDecision::Allow) => {
                    let decision = ReviewDecision::Approved;
                    telemetry.tool_decision(
                        tool_name.as_ref(),
                        &tool_ctx.call_id,
                        &decision,
                        ToolDecisionSource::Config,
                    );
                    return Ok(decision);
                }
                Some(PermissionRequestDecision::Deny { message }) => {
                    let decision = ReviewDecision::Denied;
                    telemetry.tool_decision(
                        tool_name.as_ref(),
                        &tool_ctx.call_id,
                        &decision,
                        ToolDecisionSource::Config,
                    );
                    return Err(ToolError::Rejected(message));
                }
                None => {}
            }
        }

        let otel_source = if approval_ctx.guardian_review_id.is_some() {
            ToolDecisionSource::AutomatedReviewer
        } else {
            ToolDecisionSource::User
        };
        let decision = tool.start_approval_async(req, approval_ctx).await;
        let tool_name = flat_tool_name(&tool_ctx.tool_name);
        telemetry.tool_decision(
            tool_name.as_ref(),
            &tool_ctx.call_id,
            &decision,
            otel_source,
        );
        Ok(decision)
    }

    async fn reject_if_not_approved<Session, Turn, Trigger>(
        &self,
        tool_ctx: &ToolCtx<Session, Turn>,
        guardian_review_id: Option<&str>,
        decision: ReviewDecision,
    ) -> Result<(), ToolError>
    where
        Host: ToolOrchestratorHost<Session, Turn, Trigger>,
    {
        match decision {
            ReviewDecision::Denied | ReviewDecision::Abort => {
                let reason = if let Some(review_id) = guardian_review_id {
                    self.host
                        .guardian_rejection_message(&tool_ctx.session, review_id)
                        .await
                } else {
                    "rejected by user".to_string()
                };
                Err(ToolError::Rejected(reason))
            }
            ReviewDecision::TimedOut => {
                Err(ToolError::Rejected(self.host.guardian_timeout_message()))
            }
            ReviewDecision::Approved
            | ReviewDecision::ApprovedExecpolicyAmendment { .. }
            | ReviewDecision::ApprovedForSession => Ok(()),
            ReviewDecision::NetworkPolicyAmendment {
                network_policy_amendment,
            } => match network_policy_amendment.action {
                NetworkPolicyRuleAction::Allow => Ok(()),
                NetworkPolicyRuleAction::Deny => {
                    Err(ToolError::Rejected("rejected by user".to_string()))
                }
            },
        }
    }
}

fn network_approval_context_from_payload(
    payload: &NetworkPolicyDecisionPayload,
) -> Option<NetworkApprovalContext> {
    if !payload.is_ask_from_decider() {
        return None;
    }

    let protocol = payload.protocol?;
    let host = payload.host.as_deref()?.trim();
    if host.is_empty() {
        return None;
    }

    Some(NetworkApprovalContext {
        host: host.to_string(),
        protocol,
    })
}

fn build_denial_reason_from_output(_output: &ExecToolCallOutput) -> String {
    "command failed; retry without sandbox?".to_string()
}
