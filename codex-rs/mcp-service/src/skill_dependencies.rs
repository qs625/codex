use std::collections::HashMap;
use std::collections::HashSet;

use transport_client_identity::is_first_party_originator;
use transport_client_identity::originator;
use config_service::Config;
use config_service::ConfigEditsBuilder;
use config_service::load_global_mcp_servers;
use codex_config_types::McpServerConfig;
use codex_config_types::McpServerTransportConfig;
use codex_config_types::OAuthCredentialsStoreMode;
use mcp_service_api::McpOAuthLoginRequest;
use mcp_service_api::McpRuntimeFuture;
use mcp_types::ElicitationReviewerHandle;
use mcp_types::McpOAuthLoginSupport;
use mcp_types::McpPermissionPromptAutoApproveContext;
use mcp_types::ResolvedMcpOAuthScopes;
use mcp_types::mcp_permission_prompt_is_auto_approved;
use mcp_types::resolve_oauth_scopes;
use protocol::models::PermissionProfile;
use protocol::protocol::AskForApproval;
use protocol::request_user_input::RequestUserInputArgs;
use protocol::request_user_input::RequestUserInputQuestion;
use protocol::request_user_input::RequestUserInputQuestionOption;
use protocol::request_user_input::RequestUserInputResponse;
use skill_service_api::SkillMetadata;
use skill_service_api::model::SkillToolDependency;
use tokio_util::sync::CancellationToken;
use tracing::warn;

const SKILL_MCP_DEPENDENCY_PROMPT_ID: &str = "skill_mcp_dependency_install";
const MCP_DEPENDENCY_OPTION_INSTALL: &str = "Install";
const MCP_DEPENDENCY_OPTION_SKIP: &str = "Continue anyway";

/// Turn-scoped values needed by skill MCP dependency installation.
///
/// Hosts build this from their concrete turn/session runtime. The dependency
/// installer only needs these stable policy fields and does not depend on a
/// concrete core `TurnContext`.
pub struct McpSkillDependencyTurnContext<'a> {
    pub sub_id: &'a str,
    pub approval_policy: AskForApproval,
    pub permission_profile: PermissionProfile,
}

/// Host boundary for installing MCP servers required by selected skills.
///
/// Implementations own session state, user prompting, MCP auth, and live MCP
/// refresh wiring. This runtime owns the config/dedup/OAuth policy and calls
/// back through this trait for effects that remain session-specific.
pub trait McpSkillDependencyHost: Send + Sync {
    fn configured_servers<'a>(
        &'a self,
        config: &'a Config,
    ) -> McpRuntimeFuture<'a, HashMap<String, McpServerConfig>>;

    fn prompted_dependency_keys(&self) -> McpRuntimeFuture<'_, HashSet<String>>;

    fn record_prompted_dependency_keys<'a>(
        &'a self,
        names: Vec<String>,
    ) -> McpRuntimeFuture<'a, ()>;

    fn request_user_input<'a>(
        &'a self,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> McpRuntimeFuture<'a, Option<RequestUserInputResponse>>;

    fn notify_user_input_response<'a>(
        &'a self,
        sub_id: &'a str,
        response: RequestUserInputResponse,
    ) -> McpRuntimeFuture<'a, ()>;

    fn oauth_login_support<'a>(
        &'a self,
        transport: &'a McpServerTransportConfig,
    ) -> McpRuntimeFuture<'a, McpOAuthLoginSupport>;

    fn perform_oauth_login<'a>(
        &'a self,
        request: McpOAuthLoginRequest,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>>;

    fn should_retry_without_scopes(
        &self,
        scopes: &ResolvedMcpOAuthScopes,
        error: &anyhow::Error,
    ) -> bool;

    fn refresh_mcp_servers_now<'a>(
        &'a self,
        servers: HashMap<String, McpServerConfig>,
        store_mode: OAuthCredentialsStoreMode,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()>;
}

pub async fn maybe_prompt_and_install_mcp_dependencies(
    host: &impl McpSkillDependencyHost,
    turn_context: &McpSkillDependencyTurnContext<'_>,
    config: &Config,
    cancellation_token: &CancellationToken,
    mentioned_skills: &[SkillMetadata],
    elicitation_reviewer: Option<ElicitationReviewerHandle>,
) {
    let originator_value = originator().value;
    if !is_first_party_originator(originator_value.as_str()) {
        // Only support first-party clients for now.
        return;
    }

    if mentioned_skills.is_empty()
        || !config
            .features
            .enabled(codex_features::Feature::SkillMcpDependencyInstall)
    {
        return;
    }

    let installed = host.configured_servers(config).await;
    let missing = collect_missing_mcp_dependencies(mentioned_skills, &installed);
    if missing.is_empty() {
        return;
    }

    let unprompted_missing = filter_prompted_mcp_dependencies(host, &missing).await;
    if unprompted_missing.is_empty() {
        return;
    }

    if should_install_mcp_dependencies(host, turn_context, &unprompted_missing, cancellation_token)
        .await
    {
        maybe_install_mcp_dependencies(
            host,
            turn_context,
            config,
            mentioned_skills,
            elicitation_reviewer,
        )
        .await;
    }
}

pub async fn maybe_install_mcp_dependencies(
    host: &impl McpSkillDependencyHost,
    _turn_context: &McpSkillDependencyTurnContext<'_>,
    config: &Config,
    mentioned_skills: &[SkillMetadata],
    elicitation_reviewer: Option<ElicitationReviewerHandle>,
) {
    if mentioned_skills.is_empty()
        || !config
            .features
            .enabled(codex_features::Feature::SkillMcpDependencyInstall)
    {
        return;
    }

    let codex_home = config.codex_home.clone();
    let installed = host.configured_servers(config).await;
    let missing = collect_missing_mcp_dependencies(mentioned_skills, &installed);
    if missing.is_empty() {
        return;
    }

    let mut servers = match load_global_mcp_servers(&codex_home).await {
        Ok(servers) => servers,
        Err(err) => {
            warn!("failed to load MCP servers while installing skill dependencies: {err}");
            return;
        }
    };

    let mut updated = false;
    let mut added = Vec::new();
    for (name, config) in missing {
        if servers.contains_key(&name) {
            continue;
        }
        servers.insert(name.clone(), config.clone());
        added.push((name, config));
        updated = true;
    }

    if !updated {
        return;
    }

    if let Err(err) = ConfigEditsBuilder::new(&codex_home)
        .replace_mcp_servers(&servers)
        .apply()
        .await
    {
        warn!("failed to persist MCP dependencies for mentioned skills: {err}");
        return;
    }

    for (name, server_config) in added {
        let oauth_config = match host.oauth_login_support(&server_config.transport).await {
            McpOAuthLoginSupport::Supported(config) => config,
            McpOAuthLoginSupport::Unsupported => continue,
            McpOAuthLoginSupport::Unknown(err) => {
                warn!("MCP server may or may not require login for dependency {name}: {err}");
                continue;
            }
        };

        let resolved_scopes = resolve_oauth_scopes(
            /*explicit_scopes*/ None,
            server_config.scopes.clone(),
            oauth_config.discovered_scopes.clone(),
        );
        let oauth_client_id = server_config.oauth_client_id().map(str::to_string);
        let oauth_resource = server_config.oauth_resource.clone();
        let callback_url = config.mcp_oauth_callback_url.clone();
        let first_attempt = host
            .perform_oauth_login(McpOAuthLoginRequest {
                server_name: name.clone(),
                server_url: oauth_config.url.clone(),
                store_mode: config.mcp_oauth_credentials_store_mode,
                http_headers: oauth_config.http_headers.clone(),
                env_http_headers: oauth_config.env_http_headers.clone(),
                scopes: resolved_scopes.scopes.clone(),
                oauth_client_id: oauth_client_id.clone(),
                oauth_resource: oauth_resource.clone(),
                callback_port: config.mcp_oauth_callback_port,
                callback_url: callback_url.clone(),
            })
            .await;

        if let Err(err) = first_attempt {
            if host.should_retry_without_scopes(&resolved_scopes, &err) {
                if let Err(err) = host
                    .perform_oauth_login(McpOAuthLoginRequest {
                        server_name: name.clone(),
                        server_url: oauth_config.url,
                        store_mode: config.mcp_oauth_credentials_store_mode,
                        http_headers: oauth_config.http_headers,
                        env_http_headers: oauth_config.env_http_headers,
                        scopes: Vec::new(),
                        oauth_client_id,
                        oauth_resource,
                        callback_port: config.mcp_oauth_callback_port,
                        callback_url,
                    })
                    .await
                {
                    warn!("failed to login to MCP dependency {name}: {err}");
                }
            } else {
                warn!("failed to login to MCP dependency {name}: {err}");
            }
        }
    }

    // Refresh from the config-backed merged MCP map (global + repo + managed)
    // and overlay the updated global servers so we don't drop repo-scoped
    // servers. Runtime additions such as built-ins are rebuilt by the refresh
    // path from the current config.
    let mut refresh_servers = host.configured_servers(config).await;
    for (name, server_config) in &servers {
        refresh_servers
            .entry(name.clone())
            .or_insert_with(|| server_config.clone());
    }
    host.refresh_mcp_servers_now(
        refresh_servers,
        config.mcp_oauth_credentials_store_mode,
        elicitation_reviewer,
    )
    .await;
}

async fn should_install_mcp_dependencies(
    host: &impl McpSkillDependencyHost,
    turn_context: &McpSkillDependencyTurnContext<'_>,
    missing: &HashMap<String, McpServerConfig>,
    cancellation_token: &CancellationToken,
) -> bool {
    if mcp_permission_prompt_is_auto_approved(
        turn_context.approval_policy,
        &turn_context.permission_profile,
        McpPermissionPromptAutoApproveContext::default(),
    ) {
        return true;
    }

    let server_list = format_missing_mcp_dependencies(missing);
    let question = RequestUserInputQuestion {
        id: SKILL_MCP_DEPENDENCY_PROMPT_ID.to_string(),
        header: "Install MCP servers?".to_string(),
        question: format!(
            "The following MCP servers are required by the selected skills but are not installed yet: {server_list}. Install them now?"
        ),
        is_other: false,
        is_secret: false,
        options: Some(vec![
            RequestUserInputQuestionOption {
                label: MCP_DEPENDENCY_OPTION_INSTALL.to_string(),
                description:
                    "Install and enable the missing MCP servers in your global config."
                        .to_string(),
            },
            RequestUserInputQuestionOption {
                label: MCP_DEPENDENCY_OPTION_SKIP.to_string(),
                description: "Skip installation for now and do not show again for these MCP servers in this session."
                    .to_string(),
            },
        ]),
    };
    let args = RequestUserInputArgs {
        questions: vec![question],
    };
    let sub_id = turn_context.sub_id;
    let call_id = format!("mcp-deps-{sub_id}");
    let response_fut = host.request_user_input(call_id, args);
    let response = tokio::select! {
        biased;
        _ = cancellation_token.cancelled() => {
            let empty = RequestUserInputResponse {
                answers: HashMap::new(),
            };
            host.notify_user_input_response(sub_id, empty.clone()).await;
            empty
        }
        response = response_fut => response.unwrap_or_else(|| RequestUserInputResponse {
            answers: HashMap::new(),
        }),
    };

    let install = response
        .answers
        .get(SKILL_MCP_DEPENDENCY_PROMPT_ID)
        .is_some_and(|answer| {
            answer
                .answers
                .iter()
                .any(|entry| entry == MCP_DEPENDENCY_OPTION_INSTALL)
        });

    let prompted_keys = missing
        .iter()
        .map(|(name, config)| canonical_mcp_server_key(name, config))
        .collect();
    host.record_prompted_dependency_keys(prompted_keys).await;

    install
}

async fn filter_prompted_mcp_dependencies(
    host: &impl McpSkillDependencyHost,
    missing: &HashMap<String, McpServerConfig>,
) -> HashMap<String, McpServerConfig> {
    let prompted = host.prompted_dependency_keys().await;
    if prompted.is_empty() {
        return missing.clone();
    }

    missing
        .iter()
        .filter(|(name, config)| !prompted.contains(&canonical_mcp_server_key(name, config)))
        .map(|(name, config)| (name.clone(), config.clone()))
        .collect()
}

fn format_missing_mcp_dependencies(missing: &HashMap<String, McpServerConfig>) -> String {
    let mut names = missing.keys().cloned().collect::<Vec<_>>();
    names.sort();
    names.join(", ")
}

fn canonical_mcp_key(transport: &str, identifier: &str, fallback: &str) -> String {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        fallback.to_string()
    } else {
        format!("mcp__{transport}__{identifier}")
    }
}

fn canonical_mcp_server_key(name: &str, config: &McpServerConfig) -> String {
    match &config.transport {
        McpServerTransportConfig::Stdio { command, .. } => {
            canonical_mcp_key("stdio", command, name)
        }
        McpServerTransportConfig::StreamableHttp { url, .. } => {
            canonical_mcp_key("streamable_http", url, name)
        }
    }
}

fn canonical_mcp_dependency_key(dependency: &SkillToolDependency) -> Result<String, String> {
    let transport = dependency.transport.as_deref().unwrap_or("streamable_http");
    if transport.eq_ignore_ascii_case("streamable_http") {
        let url = dependency
            .url
            .as_ref()
            .ok_or_else(|| "missing url for streamable_http dependency".to_string())?;
        return Ok(canonical_mcp_key("streamable_http", url, &dependency.value));
    }
    if transport.eq_ignore_ascii_case("stdio") {
        let command = dependency
            .command
            .as_ref()
            .ok_or_else(|| "missing command for stdio dependency".to_string())?;
        return Ok(canonical_mcp_key("stdio", command, &dependency.value));
    }
    Err(format!("unsupported transport {transport}"))
}

fn mcp_dependency_to_server_config(
    dependency: &SkillToolDependency,
) -> Result<McpServerConfig, String> {
    let transport = dependency.transport.as_deref().unwrap_or("streamable_http");
    if transport.eq_ignore_ascii_case("streamable_http") {
        let url = dependency
            .url
            .as_ref()
            .ok_or_else(|| "missing url for streamable_http dependency".to_string())?;
        return Ok(McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: url.clone(),
                bearer_token_env_var: None,
                http_headers: None,
                env_http_headers: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        });
    }

    if transport.eq_ignore_ascii_case("stdio") {
        let command = dependency
            .command
            .as_ref()
            .ok_or_else(|| "missing command for stdio dependency".to_string())?;
        return Ok(McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: command.clone(),
                args: Vec::new(),
                env: None,
                env_vars: Vec::new(),
                cwd: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        });
    }

    Err(format!("unsupported transport {transport}"))
}

fn collect_missing_mcp_dependencies(
    mentioned_skills: &[SkillMetadata],
    installed: &HashMap<String, McpServerConfig>,
) -> HashMap<String, McpServerConfig> {
    let mut missing = HashMap::new();
    let installed_keys: HashSet<String> = installed
        .iter()
        .map(|(name, config)| canonical_mcp_server_key(name, config))
        .collect();
    let mut seen_canonical_keys = HashSet::new();

    for skill in mentioned_skills {
        let Some(dependencies) = skill.dependencies.as_ref() else {
            continue;
        };

        for tool in &dependencies.tools {
            if !tool.r#type.eq_ignore_ascii_case("mcp") {
                continue;
            }
            let dependency_key = match canonical_mcp_dependency_key(tool) {
                Ok(key) => key,
                Err(err) => {
                    let dependency = tool.value.as_str();
                    let skill_name = skill.name.as_str();
                    warn!(
                        "unable to auto-install MCP dependency {dependency} for skill {skill_name}: {err}",
                    );
                    continue;
                }
            };
            if installed_keys.contains(&dependency_key)
                || seen_canonical_keys.contains(&dependency_key)
            {
                continue;
            }

            let config = match mcp_dependency_to_server_config(tool) {
                Ok(config) => config,
                Err(err) => {
                    let dependency = dependency_key.as_str();
                    let skill_name = skill.name.as_str();
                    warn!(
                        "unable to auto-install MCP dependency {dependency} for skill {skill_name}: {err}",
                    );
                    continue;
                }
            };

            missing.insert(tool.value.clone(), config);
            seen_canonical_keys.insert(dependency_key);
        }
    }

    missing
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use protocol::protocol::SkillScope;
    use skill_service_api::model::SkillDependencies;
    use skill_service_api::model::SkillToolDependency;
    use tempfile::tempdir;

    use super::*;

    fn skill(name: &str, tools: Vec<SkillToolDependency>) -> SkillMetadata {
        let dir = tempdir().expect("temp dir");
        SkillMetadata {
            name: name.to_string(),
            description: String::new(),
            short_description: None,
            interface: None,
            dependencies: Some(SkillDependencies { tools }),
            policy: None,
            path_to_skills_md: dir
                .path()
                .join("SKILL.md")
                .try_into()
                .expect("absolute skill path"),
            scope: SkillScope::User,
            plugin_id: None,
        }
    }

    fn streamable_dependency(value: &str, url: &str) -> SkillToolDependency {
        SkillToolDependency {
            r#type: "mcp".to_string(),
            value: value.to_string(),
            description: None,
            transport: Some("streamable_http".to_string()),
            command: None,
            url: Some(url.to_string()),
        }
    }

    fn stdio_dependency(value: &str, command: &str) -> SkillToolDependency {
        SkillToolDependency {
            r#type: "mcp".to_string(),
            value: value.to_string(),
            description: None,
            transport: Some("stdio".to_string()),
            command: Some(command.to_string()),
            url: None,
        }
    }

    #[test]
    fn collect_missing_mcp_dependencies_builds_server_configs() {
        let mentioned = vec![skill(
            "analysis",
            vec![
                streamable_dependency("remote-mcp", "https://example.com/mcp"),
                stdio_dependency("local-mcp", "local-mcp"),
            ],
        )];

        let missing = collect_missing_mcp_dependencies(&mentioned, &HashMap::new());

        assert_eq!(missing.len(), 2);
        assert_eq!(
            missing.get("remote-mcp").map(|server| &server.transport),
            Some(&McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: None,
                http_headers: None,
                env_http_headers: None,
            })
        );
        assert_eq!(
            missing.get("local-mcp").map(|server| &server.transport),
            Some(&McpServerTransportConfig::Stdio {
                command: "local-mcp".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: Vec::new(),
                cwd: None,
            })
        );
    }

    #[test]
    fn collect_missing_mcp_dependencies_skips_installed_and_duplicate_servers() {
        let mentioned = vec![skill(
            "analysis",
            vec![
                streamable_dependency("remote-a", "https://example.com/mcp"),
                streamable_dependency("remote-b", "https://example.com/mcp"),
                stdio_dependency("local-mcp", "local-mcp"),
            ],
        )];
        let installed = HashMap::from([(
            "installed-local".to_string(),
            mcp_dependency_to_server_config(&stdio_dependency("local-mcp", "local-mcp"))
                .expect("stdio config"),
        )]);

        let missing = collect_missing_mcp_dependencies(&mentioned, &installed);

        assert_eq!(
            missing.keys().cloned().collect::<Vec<_>>(),
            vec!["remote-a".to_string()]
        );
    }
}
