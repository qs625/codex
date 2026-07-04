use std::sync::Arc;

use crate::AuthProvider;
use crate::SharedAuthProvider;
use codex_agent_identity_api::AgentIdentityKey;
use codex_agent_identity_api::AgentTaskAuthorizationTarget;
use codex_agent_identity_api::authorization_header_for_agent_task;
use codex_auth_types::RequestAuthSnapshot;
use http::HeaderMap;
use http::HeaderValue;

/// Bearer-token auth provider for OpenAI-compatible requests.
#[derive(Clone, Default)]
pub struct BearerAuthProvider {
    pub token: Option<String>,
    pub account_id: Option<String>,
    pub is_fedramp_account: bool,
}

impl BearerAuthProvider {
    pub fn new(token: String) -> Self {
        Self {
            token: Some(token),
            account_id: None,
            is_fedramp_account: false,
        }
    }

    pub fn for_test(token: Option<&str>, account_id: Option<&str>) -> Self {
        Self {
            token: token.map(str::to_string),
            account_id: account_id.map(str::to_string),
            is_fedramp_account: false,
        }
    }
}

impl AuthProvider for BearerAuthProvider {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        if let Some(token) = self.token.as_ref()
            && let Ok(header) = HeaderValue::from_str(&format!("Bearer {token}"))
        {
            let _ = headers.insert(http::header::AUTHORIZATION, header);
        }
        if let Some(account_id) = self.account_id.as_ref()
            && let Ok(header) = HeaderValue::from_str(account_id)
        {
            let _ = headers.insert("ChatGPT-Account-ID", header);
        }
        if self.is_fedramp_account {
            let _ = headers.insert("X-OpenAI-Fedramp", HeaderValue::from_static("true"));
        }
    }
}

#[derive(Clone, Debug)]
struct AgentIdentityAuthProvider {
    agent_runtime_id: String,
    private_key_pkcs8_base64: String,
    task_id: String,
    account_id: String,
    is_fedramp_account: bool,
}

impl AuthProvider for AgentIdentityAuthProvider {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        let header_value = authorization_header_for_agent_task(
            AgentIdentityKey {
                agent_runtime_id: &self.agent_runtime_id,
                private_key_pkcs8_base64: &self.private_key_pkcs8_base64,
            },
            AgentTaskAuthorizationTarget {
                agent_runtime_id: &self.agent_runtime_id,
                task_id: &self.task_id,
            },
        )
        .map_err(std::io::Error::other);

        if let Ok(header_value) = header_value
            && let Ok(header) = HeaderValue::from_str(&header_value)
        {
            let _ = headers.insert(http::header::AUTHORIZATION, header);
        }

        if let Ok(header) = HeaderValue::from_str(&self.account_id) {
            let _ = headers.insert("ChatGPT-Account-ID", header);
        }

        if self.is_fedramp_account {
            let _ = headers.insert("X-OpenAI-Fedramp", HeaderValue::from_static("true"));
        }
    }
}

#[derive(Clone, Debug)]
struct UnauthenticatedAuthProvider;

impl AuthProvider for UnauthenticatedAuthProvider {
    fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
}

pub fn unauthenticated_auth_provider() -> SharedAuthProvider {
    Arc::new(UnauthenticatedAuthProvider)
}

/// Builds request-header auth from a lightweight auth snapshot.
pub fn auth_provider_from_auth_snapshot(auth: &RequestAuthSnapshot) -> SharedAuthProvider {
    match auth {
        RequestAuthSnapshot::AgentIdentity(auth) => Arc::new(AgentIdentityAuthProvider {
            agent_runtime_id: auth.agent_runtime_id.clone(),
            private_key_pkcs8_base64: auth.private_key_pkcs8_base64.clone(),
            task_id: auth.task_id.clone(),
            account_id: auth.account_id.clone(),
            is_fedramp_account: auth.is_fedramp_account,
        }),
        RequestAuthSnapshot::Bearer(auth) => Arc::new(BearerAuthProvider {
            token: auth.token.clone(),
            account_id: auth.account_id.clone(),
            is_fedramp_account: auth.is_fedramp_account,
        }),
    }
}

#[cfg(test)]
mod tests {
    use crate::AuthHeaderTelemetry;
    use crate::auth_header_telemetry;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn bearer_auth_provider_reports_when_auth_header_will_attach() {
        let auth = BearerAuthProvider {
            token: Some("access-token".to_string()),
            account_id: None,
            is_fedramp_account: false,
        };

        assert_eq!(
            auth_header_telemetry(&auth),
            AuthHeaderTelemetry {
                attached: true,
                name: Some("authorization"),
            }
        );
    }

    #[test]
    fn bearer_auth_provider_adds_auth_headers() {
        let auth = BearerAuthProvider::for_test(Some("access-token"), Some("workspace-123"));
        let mut headers = HeaderMap::new();

        auth.add_auth_headers(&mut headers);

        assert_eq!(
            headers
                .get(http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer access-token")
        );
        assert_eq!(
            headers
                .get("ChatGPT-Account-ID")
                .and_then(|value| value.to_str().ok()),
            Some("workspace-123")
        );
    }

    #[test]
    fn bearer_auth_provider_adds_fedramp_routing_header_for_fedramp_accounts() {
        let auth = BearerAuthProvider {
            token: Some("access-token".to_string()),
            account_id: Some("workspace-123".to_string()),
            is_fedramp_account: true,
        };
        let mut headers = HeaderMap::new();

        auth.add_auth_headers(&mut headers);

        assert_eq!(
            headers
                .get("X-OpenAI-Fedramp")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
    }
}
