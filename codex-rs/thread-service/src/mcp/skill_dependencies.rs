use std::collections::HashMap;
use std::collections::HashSet;

use codex_config_types::McpServerConfig;
use codex_config_types::McpServerTransportConfig;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_mcp_types::ElicitationReviewerHandle;
use codex_mcp_types::McpOAuthLoginSupport;
use codex_mcp_types::ResolvedMcpOAuthScopes;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use mcp_service::McpSkillDependencyHost;
use mcp_service_api::McpOAuthLoginRequest;
use mcp_service_api::McpRuntimeFuture;
use tokio_util::sync::CancellationToken;

use crate::SkillMetadata;
use crate::config::Config;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

pub(crate) async fn maybe_prompt_and_install_mcp_dependencies(
    sess: &Session,
    turn_context: &TurnContext,
    cancellation_token: &CancellationToken,
    mentioned_skills: &[SkillMetadata],
    elicitation_reviewer: Option<ElicitationReviewerHandle>,
) {
    let host = CoreMcpSkillDependencyHost { sess, turn_context };
    let dependency_turn_context = turn_context.mcp_skill_dependency_turn_context();
    mcp_service::maybe_prompt_and_install_mcp_dependencies(
        &host,
        &dependency_turn_context,
        turn_context.mcp_skill_dependency_config(),
        cancellation_token,
        mentioned_skills,
        elicitation_reviewer,
    )
    .await;
}

struct CoreMcpSkillDependencyHost<'a> {
    sess: &'a Session,
    turn_context: &'a TurnContext,
}

impl McpSkillDependencyHost for CoreMcpSkillDependencyHost<'_> {
    fn configured_servers<'a>(
        &'a self,
        config: &'a Config,
    ) -> McpRuntimeFuture<'a, HashMap<String, McpServerConfig>> {
        Box::pin(async move { self.sess.configured_mcp_servers(config).await })
    }

    fn prompted_dependency_keys(&self) -> McpRuntimeFuture<'_, HashSet<String>> {
        Box::pin(async move { self.sess.mcp_dependency_prompted().await })
    }

    fn record_prompted_dependency_keys<'a>(
        &'a self,
        names: Vec<String>,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move {
            self.sess.record_mcp_dependency_prompted(names).await;
        })
    }

    fn request_user_input<'a>(
        &'a self,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> McpRuntimeFuture<'a, Option<RequestUserInputResponse>> {
        Box::pin(async move {
            self.sess
                .request_user_input(self.turn_context, call_id, args)
                .await
        })
    }

    fn notify_user_input_response<'a>(
        &'a self,
        sub_id: &'a str,
        response: RequestUserInputResponse,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move {
            self.sess.notify_user_input_response(sub_id, response).await;
        })
    }

    fn oauth_login_support<'a>(
        &'a self,
        transport: &'a McpServerTransportConfig,
    ) -> McpRuntimeFuture<'a, McpOAuthLoginSupport> {
        Box::pin(async move { self.sess.mcp_oauth_login_support(transport).await })
    }

    fn perform_oauth_login<'a>(
        &'a self,
        request: McpOAuthLoginRequest,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { self.sess.perform_mcp_oauth_login(request).await })
    }

    fn should_retry_without_scopes(
        &self,
        scopes: &ResolvedMcpOAuthScopes,
        error: &anyhow::Error,
    ) -> bool {
        self.sess
            .should_retry_mcp_oauth_without_scopes(scopes, error)
    }

    fn refresh_mcp_servers_now<'a>(
        &'a self,
        servers: HashMap<String, McpServerConfig>,
        store_mode: OAuthCredentialsStoreMode,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move {
            self.sess
                .refresh_mcp_servers_now(
                    self.turn_context,
                    servers,
                    store_mode,
                    elicitation_reviewer,
                )
                .await;
        })
    }
}
