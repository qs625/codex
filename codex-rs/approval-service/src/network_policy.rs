use permissions_service_api::Decision as ExecPolicyDecision;
use permissions_service_api::NetworkRuleProtocol as ExecPolicyNetworkRuleProtocol;
use protocol::approvals::NetworkApprovalContext;
use protocol::approvals::NetworkApprovalProtocol;
use protocol::approvals::NetworkPolicyAmendment;
use protocol::approvals::NetworkPolicyRuleAction;
use protocol::network_policy::NetworkPolicyDecisionPayload;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecPolicyNetworkRuleAmendment {
    pub protocol: ExecPolicyNetworkRuleProtocol,
    pub decision: ExecPolicyDecision,
    pub justification: String,
}

pub fn network_approval_context_from_payload(
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

pub fn execpolicy_network_rule_amendment(
    amendment: &NetworkPolicyAmendment,
    network_approval_context: &NetworkApprovalContext,
    host: &str,
) -> ExecPolicyNetworkRuleAmendment {
    let protocol = match network_approval_context.protocol {
        NetworkApprovalProtocol::Http => ExecPolicyNetworkRuleProtocol::Http,
        NetworkApprovalProtocol::Https => ExecPolicyNetworkRuleProtocol::Https,
        NetworkApprovalProtocol::Socks5Tcp => ExecPolicyNetworkRuleProtocol::Socks5Tcp,
        NetworkApprovalProtocol::Socks5Udp => ExecPolicyNetworkRuleProtocol::Socks5Udp,
    };
    let (decision, action_verb) = match amendment.action {
        NetworkPolicyRuleAction::Allow => (ExecPolicyDecision::Allow, "Allow"),
        NetworkPolicyRuleAction::Deny => (ExecPolicyDecision::Forbidden, "Deny"),
    };
    let protocol_label = match network_approval_context.protocol {
        NetworkApprovalProtocol::Http => "http",
        NetworkApprovalProtocol::Https => "https_connect",
        NetworkApprovalProtocol::Socks5Tcp => "socks5_tcp",
        NetworkApprovalProtocol::Socks5Udp => "socks5_udp",
    };
    let justification = format!("{action_verb} {protocol_label} access to {host}");

    ExecPolicyNetworkRuleAmendment {
        protocol,
        decision,
        justification,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_network_proxy_api::NetworkDecisionSource;
    use codex_network_proxy_api::NetworkPolicyDecision;
    use pretty_assertions::assert_eq;

    #[test]
    fn network_approval_context_requires_ask_from_decider() {
        let payload = NetworkPolicyDecisionPayload {
            decision: NetworkPolicyDecision::Deny,
            source: NetworkDecisionSource::Decider,
            protocol: Some(NetworkApprovalProtocol::Https),
            host: Some("example.com".to_string()),
            reason: Some("not_allowed".to_string()),
            port: Some(443),
        };

        assert_eq!(network_approval_context_from_payload(&payload), None);
    }

    #[test]
    fn network_approval_context_maps_protocols() {
        for protocol in [
            NetworkApprovalProtocol::Http,
            NetworkApprovalProtocol::Https,
            NetworkApprovalProtocol::Socks5Tcp,
            NetworkApprovalProtocol::Socks5Udp,
        ] {
            let payload = NetworkPolicyDecisionPayload {
                decision: NetworkPolicyDecision::Ask,
                source: NetworkDecisionSource::Decider,
                protocol: Some(protocol),
                host: Some("example.com".to_string()),
                reason: Some("not_allowed".to_string()),
                port: Some(443),
            };
            assert_eq!(
                network_approval_context_from_payload(&payload),
                Some(NetworkApprovalContext {
                    host: "example.com".to_string(),
                    protocol,
                })
            );
        }
    }

    #[test]
    fn network_policy_decision_payload_deserializes_proxy_protocol_aliases() {
        let payload: NetworkPolicyDecisionPayload = serde_json::from_str(
            r#"{"decision":"ask","source":"decider","protocol":"https_connect","host":"example.com","reason":"not_allowed","port":443}"#,
        )
        .expect("payload should deserialize");
        assert_eq!(payload.protocol, Some(NetworkApprovalProtocol::Https));

        let payload: NetworkPolicyDecisionPayload = serde_json::from_str(
            r#"{"decision":"ask","source":"decider","protocol":"http-connect","host":"example.com","reason":"not_allowed","port":443}"#,
        )
        .expect("payload should deserialize");
        assert_eq!(payload.protocol, Some(NetworkApprovalProtocol::Https));
    }

    #[test]
    fn execpolicy_network_rule_amendment_maps_protocol_action_and_justification() {
        let amendment = NetworkPolicyAmendment {
            action: NetworkPolicyRuleAction::Deny,
            host: "example.com".to_string(),
        };
        let context = NetworkApprovalContext {
            host: "example.com".to_string(),
            protocol: NetworkApprovalProtocol::Socks5Udp,
        };

        assert_eq!(
            execpolicy_network_rule_amendment(&amendment, &context, "example.com"),
            ExecPolicyNetworkRuleAmendment {
                protocol: ExecPolicyNetworkRuleProtocol::Socks5Udp,
                decision: ExecPolicyDecision::Forbidden,
                justification: "Deny socks5_udp access to example.com".to_string(),
            }
        );
    }
}
