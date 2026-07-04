use codex_network_proxy_api::BlockedRequest;
use codex_network_proxy_api::NetworkDecision;
use codex_network_proxy_api::NetworkPolicyDecision;
use codex_network_proxy_api::NetworkPolicyRequest;
use protocol::approvals::NetworkApprovalProtocol;
use protocol::models::PermissionProfile;
use protocol::protocol::AskForApproval;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Scope key for a network approval decision that can be cached within a session.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HostApprovalKey {
    host: String,
    protocol: &'static str,
    port: u16,
}

impl HostApprovalKey {
    /// Builds a session approval key from a proxy request and approval protocol.
    pub fn from_request(request: &NetworkPolicyRequest, protocol: NetworkApprovalProtocol) -> Self {
        Self {
            host: request.host.to_ascii_lowercase(),
            protocol: protocol_key_label(protocol),
            port: request.port,
        }
    }

    #[cfg(test)]
    fn new(host: impl Into<String>, protocol: &'static str, port: u16) -> Self {
        Self {
            host: host.into(),
            protocol,
            port,
        }
    }

    pub fn protocol(&self) -> &'static str {
        self.protocol
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn approval_id(&self) -> String {
        format!("network#{}#{}#{}", self.protocol, self.host, self.port)
    }
}

fn protocol_key_label(protocol: NetworkApprovalProtocol) -> &'static str {
    match protocol {
        NetworkApprovalProtocol::Http => "http",
        NetworkApprovalProtocol::Https => "https",
        NetworkApprovalProtocol::Socks5Tcp => "socks5-tcp",
        NetworkApprovalProtocol::Socks5Udp => "socks5-udp",
    }
}

/// Decision shared by concurrent waiters for the same host/protocol/port approval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingApprovalDecision {
    AllowOnce,
    AllowForSession,
    Deny,
}

impl PendingApprovalDecision {
    pub fn to_network_decision(self) -> NetworkDecision {
        match self {
            Self::AllowOnce | Self::AllowForSession => NetworkDecision::Allow,
            Self::Deny => NetworkDecision::deny("not_allowed"),
        }
    }
}

/// Terminal outcome recorded for a tool call that opened the approval scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkApprovalOutcome {
    DeniedByUser,
    DeniedByPolicy(String),
}

/// Shared pending approval slot for duplicate requests to the same network target.
pub struct PendingHostApproval {
    decision: Mutex<Option<PendingApprovalDecision>>,
    notify: Notify,
}

impl PendingHostApproval {
    fn new() -> Self {
        Self {
            decision: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    pub async fn wait_for_decision(&self) -> PendingApprovalDecision {
        loop {
            let notified = self.notify.notified();
            if let Some(decision) = *self.decision.lock().await {
                return decision;
            }
            notified.await;
        }
    }

    pub async fn set_decision(&self, decision: PendingApprovalDecision) {
        {
            let mut current = self.decision.lock().await;
            *current = Some(decision);
        }
        self.notify.notify_waiters();
    }
}

/// Active approval registration for a tool call that may be cancelled by a network denial.
pub struct ActiveNetworkApprovalCall<TTrigger> {
    pub registration_id: String,
    pub turn_id: String,
    pub trigger: TTrigger,
    pub command: String,
    cancellation_token: CancellationToken,
}

struct NetworkApprovalCallState<TTrigger> {
    active_calls: HashMap<String, Arc<ActiveNetworkApprovalCall<TTrigger>>>,
    call_outcomes: HashMap<String, NetworkApprovalOutcome>,
}

impl<TTrigger> Default for NetworkApprovalCallState<TTrigger> {
    fn default() -> Self {
        Self {
            active_calls: HashMap::new(),
            call_outcomes: HashMap::new(),
        }
    }
}

/// Runtime state for network approval de-duplication, session caching, and call cancellation.
pub struct NetworkApprovalRuntime<TTrigger> {
    calls: Mutex<NetworkApprovalCallState<TTrigger>>,
    pending_host_approvals: Mutex<HashMap<HostApprovalKey, Arc<PendingHostApproval>>>,
    session_approved_hosts: Mutex<HashSet<HostApprovalKey>>,
    session_denied_hosts: Mutex<HashSet<HostApprovalKey>>,
}

impl<TTrigger> Default for NetworkApprovalRuntime<TTrigger> {
    fn default() -> Self {
        Self {
            calls: Mutex::new(NetworkApprovalCallState::default()),
            pending_host_approvals: Mutex::new(HashMap::new()),
            session_approved_hosts: Mutex::new(HashSet::new()),
            session_denied_hosts: Mutex::new(HashSet::new()),
        }
    }
}

impl<TTrigger> NetworkApprovalRuntime<TTrigger> {
    /// Replace the target session's approval cache with the source session's
    /// currently approved hosts.
    pub async fn sync_session_approved_hosts_to(&self, other: &Self) {
        let approved_hosts = self.session_approved_hosts.lock().await.clone();
        let mut other_approved_hosts = other.session_approved_hosts.lock().await;
        other_approved_hosts.clear();
        other_approved_hosts.extend(approved_hosts.iter().cloned());
    }

    pub async fn register_call(
        &self,
        registration_id: String,
        turn_id: String,
        trigger: TTrigger,
        command: String,
        cancellation_token: CancellationToken,
    ) {
        let mut calls = self.calls.lock().await;
        let key = registration_id.clone();
        calls.active_calls.insert(
            key,
            Arc::new(ActiveNetworkApprovalCall {
                registration_id,
                turn_id,
                trigger,
                command,
                cancellation_token,
            }),
        );
    }

    pub async fn unregister_call(&self, registration_id: &str) {
        self.remove_call(registration_id).await;
    }

    pub async fn resolve_single_active_call(
        &self,
    ) -> Option<Arc<ActiveNetworkApprovalCall<TTrigger>>> {
        let calls = self.calls.lock().await;
        // Blocked proxy requests are not attributed to a specific tool call. Only pick an owner
        // when there is exactly one candidate; with concurrent calls, canceling one would be a guess.
        // TODO: Carry blocked-request attribution so concurrent active calls can be handled safely.
        if calls.active_calls.len() == 1 {
            return calls.active_calls.values().next().cloned();
        }

        None
    }

    pub async fn get_or_create_pending_approval(
        &self,
        key: HostApprovalKey,
    ) -> (Arc<PendingHostApproval>, bool) {
        let mut pending = self.pending_host_approvals.lock().await;
        if let Some(existing) = pending.get(&key).cloned() {
            return (existing, false);
        }

        let created = Arc::new(PendingHostApproval::new());
        pending.insert(key, Arc::clone(&created));
        (created, true)
    }

    pub async fn remove_pending_approval(&self, key: &HostApprovalKey) {
        self.pending_host_approvals.lock().await.remove(key);
    }

    pub async fn is_session_denied(&self, key: &HostApprovalKey) -> bool {
        self.session_denied_hosts.lock().await.contains(key)
    }

    pub async fn is_session_approved(&self, key: &HostApprovalKey) -> bool {
        self.session_approved_hosts.lock().await.contains(key)
    }

    pub async fn cache_allow_for_session(&self, key: HostApprovalKey) {
        {
            let mut denied_hosts = self.session_denied_hosts.lock().await;
            denied_hosts.remove(&key);
        }
        let mut approved_hosts = self.session_approved_hosts.lock().await;
        approved_hosts.insert(key);
    }

    pub async fn cache_deny_for_session(&self, key: HostApprovalKey) {
        {
            let mut approved_hosts = self.session_approved_hosts.lock().await;
            approved_hosts.remove(&key);
        }
        let mut denied_hosts = self.session_denied_hosts.lock().await;
        denied_hosts.insert(key);
    }

    pub async fn record_outcome_for_single_active_call(&self, outcome: NetworkApprovalOutcome) {
        let Some(owner_call) = self.resolve_single_active_call().await else {
            return;
        };
        self.record_call_outcome(&owner_call.registration_id, outcome)
            .await;
    }

    #[cfg(test)]
    async fn take_call_outcome(&self, registration_id: &str) -> Option<NetworkApprovalOutcome> {
        let mut calls = self.calls.lock().await;
        calls.call_outcomes.remove(registration_id)
    }

    pub async fn record_call_outcome(
        &self,
        registration_id: &str,
        outcome: NetworkApprovalOutcome,
    ) {
        let mut calls = self.calls.lock().await;
        let Some(call) = calls.active_calls.get(registration_id).cloned() else {
            return;
        };
        if matches!(
            calls.call_outcomes.get(registration_id),
            Some(NetworkApprovalOutcome::DeniedByUser)
        ) {
            return;
        }
        calls
            .call_outcomes
            .insert(registration_id.to_string(), outcome);

        drop(calls);
        call.cancellation_token.cancel();
    }

    async fn remove_call(&self, registration_id: &str) -> Option<NetworkApprovalOutcome> {
        let mut calls = self.calls.lock().await;
        calls.active_calls.remove(registration_id);
        calls.call_outcomes.remove(registration_id)
    }

    pub async fn finish_call_outcome(
        &self,
        registration_id: &str,
    ) -> Option<NetworkApprovalOutcome> {
        self.remove_call(registration_id).await
    }

    pub async fn record_blocked_request(&self, blocked: BlockedRequest) {
        let Some(message) = denied_network_policy_message(&blocked) else {
            return;
        };

        self.record_outcome_for_single_active_call(NetworkApprovalOutcome::DeniedByPolicy(message))
            .await;
    }
}

/// Whether an allowlist miss may be reviewed instead of hard-denied.
pub fn allows_network_approval_flow(policy: AskForApproval) -> bool {
    !matches!(policy, AskForApproval::Never)
}

pub fn permission_profile_allows_network_approval_flow(
    permission_profile: &PermissionProfile,
) -> bool {
    matches!(permission_profile, PermissionProfile::Managed { .. })
}

fn parse_network_policy_decision(value: &str) -> Option<NetworkPolicyDecision> {
    match value {
        "deny" => Some(NetworkPolicyDecision::Deny),
        "ask" => Some(NetworkPolicyDecision::Ask),
        _ => None,
    }
}

pub fn denied_network_policy_message(blocked: &BlockedRequest) -> Option<String> {
    let decision = blocked
        .decision
        .as_deref()
        .and_then(parse_network_policy_decision);
    if decision != Some(NetworkPolicyDecision::Deny) {
        return None;
    }

    let host = blocked.host.trim();
    if host.is_empty() {
        return Some("Network access was blocked by policy.".to_string());
    }

    let detail = match blocked.reason.as_str() {
        "denied" => "domain is explicitly denied by policy and cannot be approved from this prompt",
        "not_allowed" => "domain is not on the allowlist for the current sandbox mode",
        "not_allowed_local" => "local/private network addresses are blocked by the sandbox policy",
        "method_not_allowed" => "request method is blocked by the current network mode",
        "proxy_disabled" => "network proxy is disabled",
        _ => "request is blocked by network policy",
    };

    Some(format!(
        "Network access to \"{host}\" was blocked: {detail}."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_network_proxy_api::BlockedRequestArgs;
    use pretty_assertions::assert_eq;
    use protocol::permissions::NetworkSandboxPolicy;
    use protocol::protocol::SandboxPolicy;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestTrigger {
        command: String,
    }

    #[tokio::test]
    async fn pending_approvals_are_deduped_per_host_protocol_and_port() {
        let service = NetworkApprovalRuntime::<TestTrigger>::default();
        let key = HostApprovalKey::new("example.com", "http", 443);

        let (first, first_is_owner) = service.get_or_create_pending_approval(key.clone()).await;
        let (second, second_is_owner) = service.get_or_create_pending_approval(key).await;

        assert!(first_is_owner);
        assert!(!second_is_owner);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn pending_approvals_do_not_dedupe_across_ports() {
        let service = NetworkApprovalRuntime::<TestTrigger>::default();
        let first_key = HostApprovalKey::new("example.com", "https", 443);
        let second_key = HostApprovalKey::new("example.com", "https", 8443);

        let (first, first_is_owner) = service.get_or_create_pending_approval(first_key).await;
        let (second, second_is_owner) = service.get_or_create_pending_approval(second_key).await;

        assert!(first_is_owner);
        assert!(second_is_owner);
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn session_approved_hosts_preserve_protocol_and_port_scope() {
        let source = NetworkApprovalRuntime::<TestTrigger>::default();
        source
            .cache_allow_for_session(HostApprovalKey::new("example.com", "https", 443))
            .await;
        source
            .cache_allow_for_session(HostApprovalKey::new("example.com", "https", 8443))
            .await;
        source
            .cache_allow_for_session(HostApprovalKey::new("example.com", "http", 80))
            .await;

        let seeded = NetworkApprovalRuntime::default();
        source.sync_session_approved_hosts_to(&seeded).await;

        assert!(
            seeded
                .is_session_approved(&HostApprovalKey::new("example.com", "http", 80))
                .await
        );
        assert!(
            seeded
                .is_session_approved(&HostApprovalKey::new("example.com", "https", 443))
                .await
        );
        assert!(
            seeded
                .is_session_approved(&HostApprovalKey::new("example.com", "https", 8443))
                .await
        );
    }

    #[tokio::test]
    async fn sync_session_approved_hosts_to_replaces_existing_target_hosts() {
        let source = NetworkApprovalRuntime::<TestTrigger>::default();
        source
            .cache_allow_for_session(HostApprovalKey::new("source.example.com", "https", 443))
            .await;

        let target = NetworkApprovalRuntime::default();
        target
            .cache_allow_for_session(HostApprovalKey::new("stale.example.com", "https", 8443))
            .await;

        source.sync_session_approved_hosts_to(&target).await;

        assert!(
            target
                .is_session_approved(&HostApprovalKey::new("source.example.com", "https", 443))
                .await
        );
        assert!(
            !target
                .is_session_approved(&HostApprovalKey::new("stale.example.com", "https", 8443))
                .await
        );
    }

    #[tokio::test]
    async fn pending_waiters_receive_owner_decision() {
        let pending = Arc::new(PendingHostApproval::new());

        let waiter = {
            let pending = Arc::clone(&pending);
            tokio::spawn(async move { pending.wait_for_decision().await })
        };

        pending
            .set_decision(PendingApprovalDecision::AllowOnce)
            .await;

        let decision = waiter.await.expect("waiter should complete");
        assert_eq!(decision, PendingApprovalDecision::AllowOnce);
    }

    #[test]
    fn allow_once_and_allow_for_session_both_allow_network() {
        assert_eq!(
            PendingApprovalDecision::AllowOnce.to_network_decision(),
            NetworkDecision::Allow
        );
        assert_eq!(
            PendingApprovalDecision::AllowForSession.to_network_decision(),
            NetworkDecision::Allow
        );
    }

    #[test]
    fn only_never_policy_disables_network_approval_flow() {
        assert!(!allows_network_approval_flow(AskForApproval::Never));
        assert!(allows_network_approval_flow(AskForApproval::OnRequest));
        assert!(allows_network_approval_flow(AskForApproval::OnFailure));
        assert!(allows_network_approval_flow(AskForApproval::UnlessTrusted));
    }

    #[test]
    fn network_approval_flow_is_limited_to_restricted_sandbox_modes() {
        assert!(permission_profile_allows_network_approval_flow(
            &PermissionProfile::from_legacy_sandbox_policy(&SandboxPolicy::new_read_only_policy())
        ));
        assert!(permission_profile_allows_network_approval_flow(
            &PermissionProfile::from_legacy_sandbox_policy(
                &SandboxPolicy::new_workspace_write_policy()
            )
        ));
        assert!(!permission_profile_allows_network_approval_flow(
            &PermissionProfile::Disabled
        ));
        assert!(!permission_profile_allows_network_approval_flow(
            &PermissionProfile::External {
                network: NetworkSandboxPolicy::Restricted,
            }
        ));
    }

    fn denied_blocked_request(host: &str) -> BlockedRequest {
        BlockedRequest::new(BlockedRequestArgs {
            host: host.to_string(),
            reason: "not_allowed".to_string(),
            client: None,
            method: None,
            mode: None,
            protocol: "http".to_string(),
            decision: Some("deny".to_string()),
            source: Some("decider".to_string()),
            port: Some(80),
        })
    }

    #[test]
    fn denied_network_policy_message_requires_deny_decision() {
        let blocked = BlockedRequest {
            host: "example.com".to_string(),
            reason: "not_allowed".to_string(),
            client: None,
            method: Some("GET".to_string()),
            mode: None,
            protocol: "http".to_string(),
            decision: Some("ask".to_string()),
            source: Some("decider".to_string()),
            port: Some(80),
            timestamp: 0,
        };
        assert_eq!(denied_network_policy_message(&blocked), None);
    }

    #[test]
    fn denied_network_policy_message_for_denylist_block_is_explicit() {
        let blocked = BlockedRequest {
            host: "example.com".to_string(),
            reason: "denied".to_string(),
            client: None,
            method: Some("GET".to_string()),
            mode: None,
            protocol: "http".to_string(),
            decision: Some("deny".to_string()),
            source: Some("baseline_policy".to_string()),
            port: Some(80),
            timestamp: 0,
        };
        assert_eq!(
            denied_network_policy_message(&blocked),
            Some(
                "Network access to \"example.com\" was blocked: domain is explicitly denied by policy and cannot be approved from this prompt.".to_string()
            )
        );
    }

    async fn register_call_with_default_trigger(
        service: &NetworkApprovalRuntime<TestTrigger>,
        registration_id: &str,
    ) -> CancellationToken {
        let cancellation_token = CancellationToken::new();
        service
            .register_call(
                registration_id.to_string(),
                "turn-1".to_string(),
                TestTrigger {
                    command: "curl".to_string(),
                },
                "curl https://example.com".to_string(),
                cancellation_token.clone(),
            )
            .await;
        cancellation_token
    }

    #[tokio::test]
    async fn active_call_preserves_triggering_command_context() {
        let service = NetworkApprovalRuntime::<TestTrigger>::default();
        let expected = TestTrigger {
            command: "curl https://example.com".to_string(),
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
    async fn record_blocked_request_sets_policy_outcome_for_owner_call() {
        let service = NetworkApprovalRuntime::<TestTrigger>::default();
        let cancellation_token =
            register_call_with_default_trigger(&service, "registration-1").await;

        service
            .record_blocked_request(denied_blocked_request("example.com"))
            .await;

        assert!(cancellation_token.is_cancelled());
        assert_eq!(
            service.take_call_outcome("registration-1").await,
            Some(NetworkApprovalOutcome::DeniedByPolicy(
                "Network access to \"example.com\" was blocked: domain is not on the allowlist for the current sandbox mode.".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn blocked_request_policy_does_not_override_user_denial_outcome() {
        let service = NetworkApprovalRuntime::<TestTrigger>::default();
        register_call_with_default_trigger(&service, "registration-1").await;

        service
            .record_call_outcome("registration-1", NetworkApprovalOutcome::DeniedByUser)
            .await;
        service
            .record_blocked_request(denied_blocked_request("example.com"))
            .await;

        assert_eq!(
            service.take_call_outcome("registration-1").await,
            Some(NetworkApprovalOutcome::DeniedByUser)
        );
    }

    #[tokio::test]
    async fn finish_call_returns_denial_and_unregisters_active_call() {
        let service = NetworkApprovalRuntime::<TestTrigger>::default();
        register_call_with_default_trigger(&service, "registration-1").await;

        service
            .record_call_outcome(
                "registration-1",
                NetworkApprovalOutcome::DeniedByPolicy("network denied".to_string()),
            )
            .await;

        let outcome = service.finish_call_outcome("registration-1").await;

        assert_eq!(
            outcome,
            Some(NetworkApprovalOutcome::DeniedByPolicy(
                "network denied".to_string()
            ))
        );
        assert!(service.resolve_single_active_call().await.is_none());
        assert_eq!(service.take_call_outcome("registration-1").await, None);
    }

    #[tokio::test]
    async fn record_call_outcome_ignores_inactive_call() {
        let service = NetworkApprovalRuntime::<TestTrigger>::default();
        let cancellation_token =
            register_call_with_default_trigger(&service, "registration-1").await;
        service.unregister_call("registration-1").await;

        service
            .record_call_outcome(
                "registration-1",
                NetworkApprovalOutcome::DeniedByPolicy("network denied".to_string()),
            )
            .await;

        assert!(!cancellation_token.is_cancelled());
        assert_eq!(service.take_call_outcome("registration-1").await, None);
    }

    #[tokio::test]
    async fn record_blocked_request_ignores_ambiguous_unattributed_blocked_requests() {
        let service = NetworkApprovalRuntime::<TestTrigger>::default();
        register_call_with_default_trigger(&service, "registration-1").await;
        register_call_with_default_trigger(&service, "registration-2").await;

        service
            .record_blocked_request(denied_blocked_request("example.com"))
            .await;

        assert_eq!(service.take_call_outcome("registration-1").await, None);
        assert_eq!(service.take_call_outcome("registration-2").await, None);
    }
}
