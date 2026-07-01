use super::*;
use codex_config_types::McpServerConfig;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_mcp_types::ElicitationAction;
use codex_mcp_types::ElicitationReviewFuture;
use codex_mcp_types::ElicitationReviewRequest;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::ElicitationReviewer;
use codex_mcp_types::ElicitationReviewerHandle;
use codex_mcp_types::McpServerElicitationRequestParams;
use codex_mcp_types::codex_apps_tools_cache_key;
use codex_mcp_types::effective_mcp_servers_from_configured;
use codex_mcp_types::host_owned_codex_apps_enabled;
use mcp_service_api::McpConnectionRuntimeStartRequest;
use codex_protocol::approvals::ElicitationRequestEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::McpServerRefreshConfig;
use codex_protocol::mcp::RequestId;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

struct GuardianMcpElicitationReviewer {
    session: std::sync::Weak<Session>,
}

impl GuardianMcpElicitationReviewer {
    fn new(session: &Arc<Session>) -> Self {
        Self {
            session: Arc::downgrade(session),
        }
    }
}

impl ElicitationReviewer for GuardianMcpElicitationReviewer {
    fn review(&self, request: ElicitationReviewRequest) -> ElicitationReviewFuture {
        let session = self.session.clone();
        Box::pin(async move {
            let Some(session) = session.upgrade() else {
                return Ok(None);
            };
            review_guardian_mcp_elicitation(session, request).await
        })
    }
}

impl Session {
    pub(crate) fn mcp_elicitation_reviewer(self: &Arc<Self>) -> ElicitationReviewerHandle {
        Arc::new(GuardianMcpElicitationReviewer::new(self))
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub(crate) async fn request_mcp_server_elicitation(
        &self,
        turn_context: &TurnContext,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> Option<ElicitationResponse> {
        if self
            .services
            .mcp_connection_manager
            .read()
            .await
            .elicitations_auto_deny()
        {
            return Some(ElicitationResponse {
                action: ElicitationAction::Accept,
                content: Some(serde_json::json!({})),
                meta: None,
            });
        }

        let server_name = params.server_name.clone();
        let request = match params.request {
            codex_mcp_types::McpServerElicitationRequest::Form {
                meta,
                message,
                requested_schema,
            } => {
                let requested_schema = match serde_json::to_value(requested_schema) {
                    Ok(requested_schema) => requested_schema,
                    Err(err) => {
                        tracing::warn!(
                            "failed to serialize MCP elicitation schema for server_name: {server_name}, request_id: {request_id}: {err:#}"
                        );
                        return None;
                    }
                };
                codex_protocol::approvals::ElicitationRequest::Form {
                    meta,
                    message,
                    requested_schema,
                }
            }
            codex_mcp_types::McpServerElicitationRequest::Url {
                meta,
                message,
                url,
                elicitation_id,
            } => codex_protocol::approvals::ElicitationRequest::Url {
                meta,
                message,
                url,
                elicitation_id,
            },
        };

        let (tx_response, rx_response) = oneshot::channel();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_elicitation(
                        server_name.clone(),
                        request_id.clone(),
                        tx_response,
                    )
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            tracing::warn!(
                "Overwriting existing pending elicitation for server_name: {server_name}, request_id: {request_id}"
            );
        }
        let event = EventMsg::ElicitationRequest(ElicitationRequestEvent {
            turn_id: params.turn_id,
            server_name,
            id: request_id,
            request,
        });
        turn_context
            .turn_metadata_state
            .mark_user_input_requested_during_turn();
        self.send_event(turn_context, event).await;
        rx_response.await.ok()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and manager fallback must stay serialized"
    )]
    pub(crate) async fn resolve_elicitation(
        &self,
        server_name: String,
        id: RequestId,
        response: ElicitationResponse,
    ) -> anyhow::Result<()> {
        let entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.remove_pending_elicitation(&server_name, &id)
                }
                None => None,
            }
        };
        if let Some(tx_response) = entry {
            tx_response
                .send(response)
                .map_err(|e| anyhow::anyhow!("failed to send elicitation response: {e:?}"))?;
            return Ok(());
        }

        self.services
            .mcp_connection_manager
            .read()
            .await
            .resolve_elicitation(server_name, id, response)
            .await
    }

    pub(crate) async fn refresh_mcp_servers_now(
        &self,
        turn_context: &TurnContext,
        mcp_servers: HashMap<String, McpServerConfig>,
        store_mode: OAuthCredentialsStoreMode,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) {
        let auth_snapshot = match turn_context.auth_runtime.as_ref() {
            Some(auth_runtime) => auth_runtime.auth().await,
            None => None,
        };
        let config = self.get_config().await;
        let mcp_config = config
            .to_mcp_config(self.services.plugins_manager.as_ref())
            .await;
        let tool_plugin_provenance = self
            .services
            .mcp_service
            .tool_plugin_provenance(self.services.plugins_manager.as_ref(), config.as_ref())
            .await;
        let auth_context = self
            .services
            .mcp_service
            .codex_apps_auth_context(auth_snapshot.as_ref());
        let mcp_servers =
            effective_mcp_servers_from_configured(mcp_servers, &mcp_config, auth_context.as_ref());
        let host_owned_codex_apps_enabled =
            host_owned_codex_apps_enabled(&mcp_config, auth_context.as_ref());
        let auth_statuses = self
            .services
            .mcp_auth_runtime
            .compute_auth_statuses(
                mcp_servers
                    .iter()
                    .map(|(name, server)| (name.clone(), server.clone()))
                    .collect(),
                store_mode,
                host_owned_codex_apps_enabled,
            )
            .await;
        let local_environment = self.services.environment_manager.local_environment();
        let mcp_runtime_environment = match turn_context.environments.primary() {
            Some(turn_environment) => self.services.mcp_service.build_runtime_environment(
                Arc::clone(&turn_environment.environment),
                Arc::clone(&local_environment),
                turn_environment.cwd.to_path_buf(),
            ),
            None => {
                let environment = self
                    .services
                    .environment_manager
                    .default_environment()
                    .unwrap_or_else(|| Arc::clone(&local_environment));
                self.services.mcp_service.build_runtime_environment(
                    environment,
                    local_environment,
                    #[allow(deprecated)]
                    turn_context.cwd.to_path_buf(),
                )
            }
        };
        {
            let mut guard = self.services.mcp_startup_cancellation_token.lock().await;
            guard.cancel();
            *guard = CancellationToken::new();
        }
        let refreshed_runtime = self
            .services
            .mcp_service
            .start_connection_runtime(
            self.services.mcp_connection_runtime_factory.as_ref(),
            McpConnectionRuntimeStartRequest {
                mcp_servers,
                store_mode,
                auth_entries: auth_statuses,
                approval_policy: turn_context.approval_policy.clone(),
                submit_id: turn_context.sub_id.clone(),
                tx_event: self.get_tx_event(),
                initial_permission_profile: turn_context.permission_profile().clone(),
                runtime_environment: mcp_runtime_environment,
                codex_home: config.codex_home.to_path_buf(),
                codex_apps_tools_cache_key: codex_apps_tools_cache_key(auth_context.as_ref()),
                host_owned_codex_apps_enabled,
                client_elicitation_support: mcp_config.client_elicitation_support,
                tool_plugin_provenance,
                codex_apps_auth_provider: self.services.mcp_service.codex_apps_auth_provider(
                    auth_snapshot.as_ref(),
                ),
                elicitation_reviewer,
            },
        )
        .await;
        let refreshed_manager = refreshed_runtime.runtime;
        let cancel_token = refreshed_runtime.startup_cancellation_token;
        {
            let current_manager = self.services.mcp_connection_manager.read().await;
            refreshed_manager.set_elicitations_auto_deny(current_manager.elicitations_auto_deny());
        }
        {
            let mut guard = self.services.mcp_startup_cancellation_token.lock().await;
            if guard.is_cancelled() {
                cancel_token.cancel();
            }
            *guard = cancel_token;
        }

        let mut old_manager = {
            let mut manager = self.services.mcp_connection_manager.write().await;
            std::mem::replace(&mut *manager, refreshed_manager)
        };
        old_manager.shutdown().await;
    }

    pub(crate) async fn refresh_mcp_servers_if_requested(
        &self,
        turn_context: &TurnContext,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) {
        let refresh_config = { self.pending_mcp_server_refresh_config.lock().await.take() };
        let Some(refresh_config) = refresh_config else {
            return;
        };

        let McpServerRefreshConfig {
            mcp_servers,
            mcp_oauth_credentials_store_mode,
        } = refresh_config;

        let mcp_servers = match serde_json::from_value::<HashMap<String, McpServerConfig>>(mcp_servers)
        {
            Ok(servers) => servers,
            Err(err) => {
                tracing::warn!("failed to parse MCP server refresh config: {err}");
                return;
            }
        };
        let store_mode =
            match serde_json::from_value::<OAuthCredentialsStoreMode>(mcp_oauth_credentials_store_mode)
            {
                Ok(mode) => mode,
                Err(err) => {
                    tracing::warn!("failed to parse MCP OAuth refresh config: {err}");
                    return;
                }
            };

        self.refresh_mcp_servers_now(turn_context, mcp_servers, store_mode, elicitation_reviewer)
            .await;
    }

    #[cfg(test)]
    pub(crate) async fn mcp_startup_cancellation_token(&self) -> CancellationToken {
        self.services
            .mcp_startup_cancellation_token
            .lock()
            .await
            .clone()
    }

    pub(crate) async fn cancel_mcp_startup(&self) {
        self.services
            .mcp_startup_cancellation_token
            .lock()
            .await
            .cancel();
    }
}

async fn review_guardian_mcp_elicitation(
    session: Arc<Session>,
    request: ElicitationReviewRequest,
) -> anyhow::Result<Option<ElicitationResponse>> {
    let Some((turn_context, _cancellation_token)) =
        session.active_turn_context_and_cancellation_token().await
    else {
        return Ok(None);
    };

    session
        .services
        .mcp_service
        .review_guardian_elicitation(
            Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>,
            Arc::clone(&turn_context) as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
            request,
        )
        .await
}
