use std::collections::HashMap;

use anyhow::Result;
use codex_config_types::McpServerConfig;
use codex_config_types::McpServerTransportConfig;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_mcp_types::EffectiveMcpServer;
use codex_mcp_types::McpAuthStatusEntry;
use codex_mcp_types::McpOAuthLoginConfig;
use codex_mcp_types::McpOAuthLoginSupport;
use codex_mcp_types::McpOAuthScopesSource;
use codex_mcp_types::ResolvedMcpOAuthScopes;
use codex_protocol::protocol::McpAuthStatus;
use codex_rmcp_client::OAuthProviderError;
use codex_rmcp_client::determine_streamable_http_auth_status;
use codex_rmcp_client::discover_streamable_http_oauth;
use futures::future::join_all;
use mcp_service_api::McpAuthFuture;
use mcp_service_api::McpAuthRuntime;
use mcp_service_api::McpOAuthLoginRequest;
use tracing::warn;

use super::CODEX_APPS_MCP_SERVER_NAME;

#[derive(Debug, Default)]
pub struct DefaultMcpAuthRuntime;

impl McpAuthRuntime for DefaultMcpAuthRuntime {
    fn oauth_login_support<'a>(
        &'a self,
        transport: &'a McpServerTransportConfig,
    ) -> McpAuthFuture<'a, McpOAuthLoginSupport> {
        Box::pin(oauth_login_support(transport))
    }

    fn perform_oauth_login<'a>(
        &'a self,
        request: McpOAuthLoginRequest,
    ) -> McpAuthFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            codex_rmcp_client::perform_oauth_login(
                &request.server_name,
                &request.server_url,
                request.store_mode,
                request.http_headers,
                request.env_http_headers,
                &request.scopes,
                request.oauth_client_id.as_deref(),
                request.oauth_resource.as_deref(),
                request.callback_port,
                request.callback_url.as_deref(),
            )
            .await
        })
    }

    fn should_retry_without_scopes(
        &self,
        scopes: &ResolvedMcpOAuthScopes,
        error: &anyhow::Error,
    ) -> bool {
        self::should_retry_without_scopes(scopes, error)
    }

    fn compute_auth_statuses<'a>(
        &'a self,
        servers: Vec<(String, EffectiveMcpServer)>,
        store_mode: OAuthCredentialsStoreMode,
        host_owned_codex_apps_enabled: bool,
    ) -> McpAuthFuture<'a, HashMap<String, McpAuthStatusEntry>> {
        Box::pin(async move {
            compute_auth_statuses(
                servers.iter().map(|(name, server)| (name, server)),
                store_mode,
                host_owned_codex_apps_enabled,
            )
            .await
        })
    }
}

pub async fn oauth_login_support(transport: &McpServerTransportConfig) -> McpOAuthLoginSupport {
    let McpServerTransportConfig::StreamableHttp {
        url,
        bearer_token_env_var,
        http_headers,
        env_http_headers,
    } = transport
    else {
        return McpOAuthLoginSupport::Unsupported;
    };

    if bearer_token_env_var.is_some() {
        return McpOAuthLoginSupport::Unsupported;
    }

    match discover_streamable_http_oauth(url, http_headers.clone(), env_http_headers.clone()).await
    {
        Ok(Some(discovery)) => McpOAuthLoginSupport::Supported(McpOAuthLoginConfig {
            url: url.clone(),
            http_headers: http_headers.clone(),
            env_http_headers: env_http_headers.clone(),
            discovered_scopes: discovery.scopes_supported,
        }),
        Ok(None) => McpOAuthLoginSupport::Unsupported,
        Err(err) => McpOAuthLoginSupport::Unknown(err),
    }
}

pub async fn discover_supported_scopes(
    transport: &McpServerTransportConfig,
) -> Option<Vec<String>> {
    match oauth_login_support(transport).await {
        McpOAuthLoginSupport::Supported(config) => config.discovered_scopes,
        McpOAuthLoginSupport::Unsupported | McpOAuthLoginSupport::Unknown(_) => None,
    }
}

pub fn should_retry_without_scopes(scopes: &ResolvedMcpOAuthScopes, error: &anyhow::Error) -> bool {
    scopes.source == McpOAuthScopesSource::Discovered
        && error.downcast_ref::<OAuthProviderError>().is_some()
}

pub async fn compute_auth_statuses<'a, I>(
    servers: I,
    store_mode: OAuthCredentialsStoreMode,
    host_owned_codex_apps_enabled: bool,
) -> HashMap<String, McpAuthStatusEntry>
where
    I: IntoIterator<Item = (&'a String, &'a EffectiveMcpServer)>,
{
    let futures = servers.into_iter().map(|(name, server)| {
        let name = name.clone();
        let config = server.configured_config().cloned();
        let has_runtime_auth = name == CODEX_APPS_MCP_SERVER_NAME
            && host_owned_codex_apps_enabled
            && config.as_ref().is_some_and(|config| {
                matches!(
                    &config.transport,
                    McpServerTransportConfig::StreamableHttp {
                        bearer_token_env_var: None,
                        ..
                    }
                )
            });
        async move {
            let auth_status = match config.as_ref() {
                Some(config) => {
                    match compute_auth_status(&name, config, store_mode, has_runtime_auth).await {
                        Ok(status) => status,
                        Err(error) => {
                            warn!(
                                "failed to determine auth status for MCP server `{name}`: {error:?}"
                            );
                            McpAuthStatus::Unsupported
                        }
                    }
                }
                None => McpAuthStatus::Unsupported,
            };
            let entry = McpAuthStatusEntry {
                config,
                auth_status,
            };
            (name, entry)
        }
    });

    join_all(futures).await.into_iter().collect()
}

async fn compute_auth_status(
    server_name: &str,
    config: &McpServerConfig,
    store_mode: OAuthCredentialsStoreMode,
    has_runtime_auth: bool,
) -> Result<McpAuthStatus> {
    if !config.enabled {
        return Ok(McpAuthStatus::Unsupported);
    }

    if has_runtime_auth {
        return Ok(McpAuthStatus::BearerToken);
    }

    match &config.transport {
        McpServerTransportConfig::Stdio { .. } => Ok(McpAuthStatus::Unsupported),
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => {
            determine_streamable_http_auth_status(
                server_name,
                url,
                bearer_token_env_var.as_deref(),
                http_headers.clone(),
                env_http_headers.clone(),
                store_mode,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::OAuthProviderError;
    use super::should_retry_without_scopes;
    use codex_mcp_types::McpOAuthScopesSource;
    use codex_mcp_types::ResolvedMcpOAuthScopes;

    #[test]
    fn should_retry_without_scopes_only_for_discovered_provider_errors() {
        let discovered = ResolvedMcpOAuthScopes {
            scopes: vec!["scope".to_string()],
            source: McpOAuthScopesSource::Discovered,
        };
        let provider_error = anyhow!(OAuthProviderError::new(
            Some("invalid_scope".to_string()),
            Some("scope rejected".to_string()),
        ));

        assert!(should_retry_without_scopes(&discovered, &provider_error));

        let configured = ResolvedMcpOAuthScopes {
            scopes: vec!["scope".to_string()],
            source: McpOAuthScopesSource::Configured,
        };
        assert!(!should_retry_without_scopes(&configured, &provider_error));
        assert!(!should_retry_without_scopes(
            &discovered,
            &anyhow!("timed out waiting for OAuth callback"),
        ));
    }
}
