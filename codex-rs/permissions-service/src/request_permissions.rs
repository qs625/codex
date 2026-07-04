use codex_network_proxy_api::normalize_host;
use protocol::approvals::NetworkApprovalContext;
use protocol::approvals::NetworkPolicyAmendment;

/// Validates that a persisted network policy amendment targets the same host
/// that was approved by the request path, after applying canonical host
/// normalization to both sides.
pub fn validate_network_policy_amendment_host(
    amendment: &NetworkPolicyAmendment,
    network_approval_context: &NetworkApprovalContext,
) -> Result<String, String> {
    let approved_host = normalize_host(&network_approval_context.host);
    let amendment_host = normalize_host(&amendment.host);
    if amendment_host != approved_host {
        return Err(format!(
            "network policy amendment host '{}' does not match approved host '{}'",
            amendment.host, network_approval_context.host
        ));
    }
    Ok(approved_host)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use protocol::approvals::NetworkApprovalContext;
    use protocol::approvals::NetworkApprovalProtocol;
    use protocol::approvals::NetworkPolicyAmendment;
    use protocol::approvals::NetworkPolicyRuleAction;

    use super::validate_network_policy_amendment_host;

    #[test]
    fn validate_network_policy_amendment_host_allows_normalized_match() {
        let amendment = NetworkPolicyAmendment {
            host: "Example.com:443".to_string(),
            action: NetworkPolicyRuleAction::Allow,
        };
        let context = NetworkApprovalContext {
            host: "example.com".to_string(),
            protocol: NetworkApprovalProtocol::Https,
        };

        let host = validate_network_policy_amendment_host(&amendment, &context)
            .expect("normalized host should match");

        assert_eq!(host, "example.com");
    }

    #[test]
    fn validate_network_policy_amendment_host_rejects_mismatch() {
        let amendment = NetworkPolicyAmendment {
            host: "other.example".to_string(),
            action: NetworkPolicyRuleAction::Allow,
        };
        let context = NetworkApprovalContext {
            host: "example.com".to_string(),
            protocol: NetworkApprovalProtocol::Https,
        };

        let err = validate_network_policy_amendment_host(&amendment, &context)
            .expect_err("mismatched host should be rejected");

        assert_eq!(
            err,
            "network policy amendment host 'other.example' does not match approved host 'example.com'"
        );
    }
}
