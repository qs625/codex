use std::sync::Arc;

use codex_approval_service_api::ActiveNetworkApproval;
use codex_approval_service_api::ApprovalSessionCapability;
use codex_approval_service_api::ApprovalServiceFuture;
use codex_approval_service_api::PermissionRequestPayload;
use codex_approval_service_api::SessionNetworkApprovalApi;
use codex_guardian::GuardianNetworkAccessTrigger;
use codex_network_proxy_api::BlockedRequest;
use codex_network_proxy_api::BlockedRequestObserver;
use codex_network_proxy_api::NetworkDecision;
use codex_network_proxy_api::NetworkPolicyDecider;
use codex_network_proxy_api::NetworkPolicyRequest;
use codex_network_proxy_api::NetworkProtocol;
use permissions_service::ActiveNetworkApprovalCall;
use permissions_service::HostApprovalKey;
use permissions_service::NetworkApprovalOutcome;
use permissions_service::NetworkApprovalRuntime;
use permissions_service::PendingApprovalDecision;
use permissions_service::PendingHostApproval;
use permissions_service::allows_network_approval_flow;
use permissions_service::permission_profile_allows_network_approval_flow;
use hooks_api::PermissionRequestDecision;
use protocol::approvals::NetworkApprovalContext;
use protocol::approvals::NetworkApprovalProtocol;
use protocol::approvals::NetworkPolicyRuleAction;
use protocol::protocol::EventMsg;
use protocol::protocol::ReviewDecision;
use protocol::protocol::WarningEvent;
use thread_service_api::NetworkApprovalSpec;
use thread_service_api::ThreadTurnCapability;
use thread_service_api::ToolRuntimeNetworkApprovalError;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

fn network_approval_outcome_to_result(
    outcome: Option<NetworkApprovalOutcome>,
) -> Result<(), ToolRuntimeNetworkApprovalError> {
    match outcome {
        Some(NetworkApprovalOutcome::DeniedByUser) => Err(
            ToolRuntimeNetworkApprovalError::Rejected("rejected by user".to_string()),
        ),
        Some(NetworkApprovalOutcome::DeniedByPolicy(message)) => {
            Err(ToolRuntimeNetworkApprovalError::Rejected(message))
        }
        None => Ok(()),
    }
}

#[derive(Default)]
pub struct NetworkApprovalService {
    runtime: NetworkApprovalRuntime<GuardianNetworkAccessTrigger>,
}

impl NetworkApprovalService {
    pub async fn sync_session_approved_hosts_to(&self, other: &Self) {
        self.runtime
            .sync_session_approved_hosts_to(&other.runtime)
            .await;
    }

    pub async fn register_call(
        &self,
        registration_id: String,
        turn_id: String,
        trigger: GuardianNetworkAccessTrigger,
        command: String,
        cancellation_token: CancellationToken,
    ) {
        self.runtime
            .register_call(
                registration_id,
                turn_id,
                trigger,
                command,
                cancellation_token,
            )
            .await;
    }

    pub async fn unregister_call(&self, registration_id: &str) {
        self.runtime.unregister_call(registration_id).await;
    }

    pub async fn resolve_single_active_call(
        &self,
    ) -> Option<Arc<ActiveNetworkApprovalCall<GuardianNetworkAccessTrigger>>> {
        self.runtime.resolve_single_active_call().await
    }

    async fn get_or_create_pending_approval(
        &self,
        key: HostApprovalKey,
    ) -> (Arc<PendingHostApproval>, bool) {
        self.runtime.get_or_create_pending_approval(key).await
    }

    async fn record_outcome_for_single_active_call(&self, outcome: NetworkApprovalOutcome) {
        self.runtime
            .record_outcome_for_single_active_call(outcome)
            .await;
    }

    pub async fn record_call_outcome(
        &self,
        registration_id: &str,
        outcome: NetworkApprovalOutcome,
    ) {
        self.runtime
            .record_call_outcome(registration_id, outcome)
            .await;
    }

    async fn finish_call_outcome(&self, registration_id: &str) -> Option<NetworkApprovalOutcome> {
        self.runtime.finish_call_outcome(registration_id).await
    }

    pub async fn finish_call(
        &self,
        registration_id: &str,
    ) -> Result<(), ToolRuntimeNetworkApprovalError> {
        network_approval_outcome_to_result(self.finish_call_outcome(registration_id).await)
    }

    pub async fn record_blocked_request(&self, blocked: BlockedRequest) {
        self.runtime.record_blocked_request(blocked).await;
    }

    fn format_network_target(protocol: &str, host: &str, port: u16) -> String {
        format!("{protocol}://{host}:{port}")
    }

    pub async fn handle_inline_policy_request(
        &self,
        session: Arc<dyn ApprovalSessionCapability>,
        request: NetworkPolicyRequest,
    ) -> NetworkDecision {
        const REASON_NOT_ALLOWED: &str = "not_allowed";

        let protocol = match request.protocol {
            NetworkProtocol::Http => NetworkApprovalProtocol::Http,
            NetworkProtocol::HttpsConnect => NetworkApprovalProtocol::Https,
            NetworkProtocol::Socks5Tcp => NetworkApprovalProtocol::Socks5Tcp,
            NetworkProtocol::Socks5Udp => NetworkApprovalProtocol::Socks5Udp,
        };
        let key = HostApprovalKey::from_request(&request, protocol);

        if self.runtime.is_session_denied(&key).await {
            return NetworkDecision::deny(REASON_NOT_ALLOWED);
        }
        if self.runtime.is_session_approved(&key).await {
            return NetworkDecision::Allow;
        }

        let (pending, is_owner) = self.get_or_create_pending_approval(key.clone()).await;
        if !is_owner {
            return pending.wait_for_decision().await.to_network_decision();
        }

        let target = Self::format_network_target(key.protocol(), request.host.as_str(), key.port());
        let policy_denial_message =
            format!("Network access to \"{target}\" was blocked by policy.");
        let prompt_reason = format!("{} is not in the allowed_domains", request.host);

        let Some(turn) = session.active_turn_runtime().await else {
            pending.set_decision(PendingApprovalDecision::Deny).await;
            self.runtime.remove_pending_approval(&key).await;
            self.record_outcome_for_single_active_call(NetworkApprovalOutcome::DeniedByPolicy(
                policy_denial_message,
            ))
            .await;
            return NetworkDecision::deny(REASON_NOT_ALLOWED);
        };

        if !permission_profile_allows_network_approval_flow(&turn.permission_profile()) {
            pending.set_decision(PendingApprovalDecision::Deny).await;
            self.runtime.remove_pending_approval(&key).await;
            self.record_outcome_for_single_active_call(NetworkApprovalOutcome::DeniedByPolicy(
                policy_denial_message,
            ))
            .await;
            return NetworkDecision::deny(REASON_NOT_ALLOWED);
        }
        if !allows_network_approval_flow(turn.approval_policy()) {
            pending.set_decision(PendingApprovalDecision::Deny).await;
            self.runtime.remove_pending_approval(&key).await;
            self.record_outcome_for_single_active_call(NetworkApprovalOutcome::DeniedByPolicy(
                policy_denial_message,
            ))
            .await;
            return NetworkDecision::deny(REASON_NOT_ALLOWED);
        }

        let owner_call = self.resolve_single_active_call().await;
        let network_approval_context = NetworkApprovalContext {
            host: request.host.clone(),
            protocol,
        };
        let guardian_approval_id = key.approval_id();
        let prompt_command = vec!["network-access".to_string(), target.clone()];
        let command = owner_call
            .as_ref()
            .map_or_else(|| prompt_command.join(" "), |call| call.command.clone());

        if let Some(permission_request_decision) = session
            .run_permission_request_hooks(
                turn.as_ref(),
                &guardian_approval_id,
                PermissionRequestPayload::bash(command, Some(format!("network-access {target}"))),
            )
            .await
        {
            match permission_request_decision {
                PermissionRequestDecision::Allow => {
                    pending
                        .set_decision(PendingApprovalDecision::AllowOnce)
                        .await;
                    self.runtime.remove_pending_approval(&key).await;
                    return NetworkDecision::Allow;
                }
                PermissionRequestDecision::Deny { message } => {
                    if let Some(owner_call) = owner_call.as_ref() {
                        self.record_call_outcome(
                            &owner_call.registration_id,
                            NetworkApprovalOutcome::DeniedByPolicy(message),
                        )
                        .await;
                    }
                    pending.set_decision(PendingApprovalDecision::Deny).await;
                    self.runtime.remove_pending_approval(&key).await;
                    return NetworkDecision::deny(REASON_NOT_ALLOWED);
                }
            }
        }

        let use_guardian = crate::guardian::routes_approval_to_guardian(
            &turn.approval_policy(),
            turn.approvals_reviewer(),
        );
        let guardian_review_id = use_guardian.then(crate::guardian::new_guardian_review_id);
        let approval_decision = if let Some(review_id) = guardian_review_id.clone() {
            crate::guardian::review_approval_request(
                session.as_ref(),
                turn.as_ref(),
                review_id,
                codex_guardian::GuardianApprovalRequest::NetworkAccess {
                    id: guardian_approval_id.clone(),
                    turn_id: owner_call.as_ref().map_or_else(
                        || turn.runtime_turn_id_str().to_string(),
                        |call| call.turn_id.clone(),
                    ),
                    target,
                    host: request.host,
                    protocol,
                    port: key.port(),
                    trigger: owner_call.as_ref().map(|call| call.trigger.clone()),
                },
                Some(policy_denial_message.clone()),
            )
            .await
        } else {
            session
                .request_command_approval(
                    turn.as_ref(),
                    guardian_approval_id,
                    /*approval_id*/ None,
                    prompt_command,
                    turn.legacy_cwd(),
                    Some(prompt_reason),
                    Some(network_approval_context.clone()),
                    /*proposed_execpolicy_amendment*/ None,
                    /*additional_permissions*/ None,
                    /*available_decisions*/ None,
                )
                .await
        };

        let mut cache_session_deny = false;
        let resolved = match approval_decision {
            ReviewDecision::Approved | ReviewDecision::ApprovedExecpolicyAmendment { .. } => {
                PendingApprovalDecision::AllowOnce
            }
            ReviewDecision::ApprovedForSession => PendingApprovalDecision::AllowForSession,
            ReviewDecision::NetworkPolicyAmendment {
                network_policy_amendment,
            } => match network_policy_amendment.action {
                NetworkPolicyRuleAction::Allow => {
                    record_network_policy_amendment(
                        session.as_ref(),
                        turn.as_ref(),
                        &network_policy_amendment,
                        &network_approval_context,
                    )
                    .await;
                    PendingApprovalDecision::AllowForSession
                }
                NetworkPolicyRuleAction::Deny => {
                    record_network_policy_amendment(
                        session.as_ref(),
                        turn.as_ref(),
                        &network_policy_amendment,
                        &network_approval_context,
                    )
                    .await;
                    if let Some(owner_call) = owner_call.as_ref() {
                        self.record_call_outcome(
                            &owner_call.registration_id,
                            NetworkApprovalOutcome::DeniedByUser,
                        )
                        .await;
                    }
                    cache_session_deny = true;
                    PendingApprovalDecision::Deny
                }
            },
            ReviewDecision::Denied | ReviewDecision::Abort => {
                if let Some(review_id) = guardian_review_id.as_deref() {
                    if let Some(owner_call) = owner_call.as_ref() {
                        let message = crate::guardian::guardian_rejection_message(
                            session.as_ref(),
                            review_id,
                        )
                        .await;
                        self.record_call_outcome(
                            &owner_call.registration_id,
                            NetworkApprovalOutcome::DeniedByPolicy(message),
                        )
                        .await;
                    }
                } else if let Some(owner_call) = owner_call.as_ref() {
                    self.record_call_outcome(
                        &owner_call.registration_id,
                        NetworkApprovalOutcome::DeniedByUser,
                    )
                    .await;
                }
                PendingApprovalDecision::Deny
            }
            ReviewDecision::TimedOut => {
                if let Some(owner_call) = owner_call.as_ref() {
                    self.record_call_outcome(
                        &owner_call.registration_id,
                        NetworkApprovalOutcome::DeniedByPolicy(
                            crate::guardian::guardian_timeout_message(),
                        ),
                    )
                    .await;
                }
                PendingApprovalDecision::Deny
            }
        };

        if matches!(resolved, PendingApprovalDecision::AllowForSession) {
            self.runtime.cache_allow_for_session(key.clone()).await;
        }
        if cache_session_deny {
            self.runtime.cache_deny_for_session(key.clone()).await;
        }

        pending.set_decision(resolved).await;
        self.runtime.remove_pending_approval(&key).await;
        resolved.to_network_decision()
    }
}

impl SessionNetworkApprovalApi for NetworkApprovalService {
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }

    fn sync_session_approved_hosts_to(
        &self,
        other: Arc<dyn SessionNetworkApprovalApi>,
    ) -> ApprovalServiceFuture<'_, ()> {
        Box::pin(async move {
            let Some(other) = other.as_any().downcast_ref::<NetworkApprovalService>() else {
                panic!("network approval runtime type mismatch");
            };
            self.runtime
                .sync_session_approved_hosts_to(&other.runtime)
                .await;
        })
    }

    fn build_blocked_request_observer(self: Arc<Self>) -> Arc<dyn BlockedRequestObserver> {
        build_blocked_request_observer(self)
    }

    fn build_network_policy_decider(
        self: Arc<Self>,
        session: Arc<RwLock<Option<std::sync::Weak<dyn ApprovalSessionCapability>>>>,
    ) -> Arc<dyn NetworkPolicyDecider> {
        build_network_policy_decider(self, session)
    }

    fn begin_network_approval(
        self: Arc<Self>,
        turn_id: &str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<GuardianNetworkAccessTrigger>>,
    ) -> ApprovalServiceFuture<'_, Option<ActiveNetworkApproval>> {
        Box::pin(async move {
            begin_network_approval(self, turn_id, managed_network_active, spec).await
        })
    }

    fn unregister_call(&self, registration_id: String) -> ApprovalServiceFuture<'_, ()> {
        Box::pin(async move {
            self.unregister_call(&registration_id).await;
        })
    }

    fn finish_call(
        &self,
        registration_id: String,
    ) -> ApprovalServiceFuture<'_, Result<(), ToolRuntimeNetworkApprovalError>> {
        Box::pin(async move { self.finish_call(&registration_id).await })
    }
}

async fn record_network_policy_amendment(
    session: &dyn ApprovalSessionCapability,
    turn: &dyn ThreadTurnCapability,
    amendment: &protocol::approvals::NetworkPolicyAmendment,
    network_approval_context: &NetworkApprovalContext,
) {
    match session
        .persist_network_policy_amendment(amendment, network_approval_context)
        .await
    {
        Ok(()) => {
            session
                .record_network_policy_amendment_message(turn, amendment)
                .await;
        }
        Err(err) => {
            let message = format!("Failed to apply network policy amendment: {err}");
            warn!("{message}");
            turn.emit_event(EventMsg::Warning(WarningEvent { message }))
                .await;
        }
    }
}

pub fn build_blocked_request_observer(
    network_approval: Arc<NetworkApprovalService>,
) -> Arc<dyn BlockedRequestObserver> {
    Arc::new(move |blocked: BlockedRequest| {
        let network_approval = Arc::clone(&network_approval);
        async move {
            network_approval.record_blocked_request(blocked).await;
        }
    })
}

pub fn build_network_policy_decider(
    network_approval: Arc<NetworkApprovalService>,
    network_policy_decider_session: Arc<
        RwLock<Option<std::sync::Weak<dyn ApprovalSessionCapability>>>,
    >,
) -> Arc<dyn NetworkPolicyDecider> {
    Arc::new(move |request: NetworkPolicyRequest| {
        let network_approval = Arc::clone(&network_approval);
        let network_policy_decider_session = Arc::clone(&network_policy_decider_session);
        async move {
            let Some(session) = network_policy_decider_session
                .read()
                .await
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
            else {
                return NetworkDecision::ask("not_allowed");
            };
            network_approval
                .handle_inline_policy_request(session, request)
                .await
        }
    })
}

pub async fn begin_network_approval(
    service: Arc<NetworkApprovalService>,
    turn_id: &str,
    managed_network_active: bool,
    spec: Option<NetworkApprovalSpec<GuardianNetworkAccessTrigger>>,
) -> Option<ActiveNetworkApproval> {
    let NetworkApprovalSpec {
        network,
        mode,
        trigger,
        command,
    } = spec?;
    if !managed_network_active || network.is_none() {
        return None;
    }

    let registration_id = Uuid::new_v4().to_string();
    let cancellation_token = CancellationToken::new();
    service
        .register_call(
            registration_id.clone(),
            turn_id.to_string(),
            trigger,
            command,
            cancellation_token.clone(),
        )
        .await;

    Some(ActiveNetworkApproval::new(
        Some(registration_id),
        mode,
        cancellation_token,
        service,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use permissions_service::NetworkApprovalOutcome;
    use core_test_support::PathBufExt;
    use core_test_support::test_path_buf;
    use pretty_assertions::assert_eq;
    use protocol::models::SandboxPermissions;

    fn default_shell_trigger() -> GuardianNetworkAccessTrigger {
        GuardianNetworkAccessTrigger {
            call_id: "call-1".to_string(),
            tool_name: "exec_command".to_string(),
            command: vec!["curl".to_string(), "https://example.com".to_string()],
            cwd: test_path_buf("/tmp").abs(),
            sandbox_permissions: SandboxPermissions::UseDefault,
            additional_permissions: None,
            justification: None,
            tty: None,
        }
    }

    async fn register_call_with_default_shell_trigger(
        service: &NetworkApprovalService,
        registration_id: &str,
    ) -> CancellationToken {
        let cancellation_token = CancellationToken::new();
        service
            .register_call(
                registration_id.to_string(),
                "turn-1".to_string(),
                default_shell_trigger(),
                "curl https://example.com".to_string(),
                cancellation_token.clone(),
            )
            .await;
        cancellation_token
    }

    #[tokio::test]
    async fn active_call_preserves_triggering_command_context() {
        let service = NetworkApprovalService::default();
        let expected = GuardianNetworkAccessTrigger {
            call_id: "call-1".to_string(),
            tool_name: "exec_command".to_string(),
            command: vec!["curl".to_string(), "https://example.com".to_string()],
            cwd: test_path_buf("/repo").abs(),
            sandbox_permissions: SandboxPermissions::UseDefault,
            additional_permissions: None,
            justification: Some("fetch release metadata".to_string()),
            tty: None,
        };

        service
            .register_call(
                "registration-1".to_string(),
                "turn-1".to_string(),
                expected.clone(),
                "curl https://example.com".to_string(),
                CancellationToken::new(),
            )
            .await;

        let call = service
            .resolve_single_active_call()
            .await
            .expect("single active call should resolve");

        assert_eq!(&call.trigger, &expected);
        assert_eq!(call.command, "curl https://example.com");
    }

    #[tokio::test]
    async fn finish_call_returns_denial_and_unregisters_active_call() {
        let service = NetworkApprovalService::default();
        register_call_with_default_shell_trigger(&service, "registration-1").await;

        service
            .record_call_outcome(
                "registration-1",
                NetworkApprovalOutcome::DeniedByPolicy("network denied".to_string()),
            )
            .await;

        let err = service
            .finish_call("registration-1")
            .await
            .expect_err("denial should be returned");

        assert!(
            matches!(err, ToolRuntimeNetworkApprovalError::Rejected(message) if message == "network denied")
        );
        assert!(service.resolve_single_active_call().await.is_none());
    }

    #[tokio::test]
    async fn deferred_finish_reuses_denial_result_after_first_consumer() {
        let service = NetworkApprovalService::default();
        let cancellation_token =
            register_call_with_default_shell_trigger(&service, "registration-1").await;
        let deferred = DeferredNetworkApproval {
            registration_id: "registration-1".to_string(),
            cancellation_token,
            finish_outcome: Arc::new(OnceCell::new()),
        };
        service
            .record_call_outcome(
                "registration-1",
                NetworkApprovalOutcome::DeniedByPolicy("network denied".to_string()),
            )
            .await;

        let first = deferred
            .finish(&service)
            .await
            .expect_err("first consumer should see denial");
        let second = deferred
            .finish(&service)
            .await
            .expect_err("second consumer should reuse denial");

        assert!(
            matches!(first, ToolRuntimeNetworkApprovalError::Rejected(message) if message == "network denied")
        );
        assert!(
            matches!(second, ToolRuntimeNetworkApprovalError::Rejected(message) if message == "network denied")
        );
    }
}
