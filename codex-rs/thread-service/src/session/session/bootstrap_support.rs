use super::*;

pub(super) struct SessionInitBasics {
    pub(super) forked_from_id: Option<ThreadId>,
    pub(super) thread_id: ThreadId,
    pub(super) window_generation: u64,
    pub(super) event_persistence_mode: ThreadEventPersistenceMode,
}

pub(super) fn derive_session_init_basics(
    initial_history: &InitialHistory,
    persist_extended_history: bool,
) -> SessionInitBasics {
    let event_persistence_mode = if persist_extended_history {
        ThreadEventPersistenceMode::Extended
    } else {
        ThreadEventPersistenceMode::Limited
    };
    let thread_id = match initial_history {
        InitialHistory::New | InitialHistory::Cleared | InitialHistory::Forked(_) => {
            ThreadId::default()
        }
        InitialHistory::Resumed(resumed_history) => resumed_history.conversation_id,
    };
    let window_generation = match initial_history {
        InitialHistory::Resumed(resumed_history) => u64::try_from(
            resumed_history
                .history
                .iter()
                .filter(|item| matches!(item, RolloutItem::Compacted(_)))
                .count(),
        )
        .unwrap_or(u64::MAX),
        InitialHistory::New | InitialHistory::Cleared | InitialHistory::Forked(_) => 0,
    };

    SessionInitBasics {
        forked_from_id: initial_history.forked_from_id(),
        thread_id,
        window_generation,
        event_persistence_mode,
    }
}

pub(super) async fn load_auth_and_mcp(
    shared_auth_runtime: SharedAuthRuntime,
    config: Arc<Config>,
    mcp_service: Arc<dyn McpServiceApi>,
    plugins_manager: SharedPluginRuntime,
    mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
) -> (
    Option<RequestAuthSnapshot>,
    HashMap<String, EffectiveMcpServer>,
    HashMap<String, McpAuthStatusEntry>,
) {
    let auth_snapshot = shared_auth_runtime.auth().await;
    let auth_context = mcp_service.codex_apps_auth_context(auth_snapshot.as_ref());
    let mcp_servers = mcp_service
        .effective_servers(plugins_manager.as_ref(), &config, auth_context.as_ref())
        .await;
    let host_owned_codex_apps_enabled = config.features.apps_enabled_for_auth(
        auth_snapshot
            .as_ref()
            .is_some_and(codex_auth_types::RequestAuthSnapshot::uses_codex_backend),
    );
    let auth_statuses = mcp_auth_runtime
        .compute_auth_statuses(
            mcp_servers
                .iter()
                .map(|(name, server)| (name.clone(), server.clone()))
                .collect(),
            config.mcp_oauth_credentials_store_mode,
            host_owned_codex_apps_enabled,
        )
        .await;
    (auth_snapshot, mcp_servers, auth_statuses)
}

pub(super) fn build_post_session_configured_events(config: &Config) -> Vec<Event> {
    let mut events = Vec::new();

    for usage in config.features.legacy_feature_usages() {
        events.push(Event {
            id: INITIAL_SUBMIT_ID.to_owned(),
            msg: EventMsg::DeprecationNotice(DeprecationNoticeEvent {
                summary: usage.summary.clone(),
                details: usage.details.clone(),
            }),
        });
    }
    for message in &config.startup_warnings {
        events.push(Event {
            id: "".to_owned(),
            msg: EventMsg::Warning(WarningEvent {
                message: message.clone(),
            }),
        });
    }
    let config_path = config.codex_home.join(CONFIG_TOML_FILE);
    if let Some(event) = unstable_features_warning_event(
        config
            .config_layer_stack
            .effective_config()
            .get("features")
            .and_then(TomlValue::as_table),
        config.suppress_unstable_features_warning,
        &config.features,
        &config_path.display().to_string(),
    ) {
        events.push(event);
    }
    if config.permissions.approval_policy.value() == AskForApproval::OnFailure {
        events.push(Event {
            id: "".to_owned(),
            msg: EventMsg::Warning(WarningEvent {
                message: "`on-failure` approval policy is deprecated and will be removed in a future release. Use `on-request` for interactive approvals or `never` for non-interactive runs.".to_string(),
            }),
        });
    }

    events
}

struct SessionMcpStartupPlan {
    required_mcp_servers: Vec<String>,
    enabled_mcp_server_count: usize,
    required_mcp_server_count: usize,
    host_owned_codex_apps_enabled: bool,
    client_elicitation_support: McpClientElicitationSupport,
}

fn derive_session_mcp_startup_plan(
    features: &ManagedFeatures,
    auth_snapshot: Option<&RequestAuthSnapshot>,
    mcp_servers: &HashMap<String, EffectiveMcpServer>,
) -> SessionMcpStartupPlan {
    let mut required_mcp_servers: Vec<String> = mcp_servers
        .iter()
        .filter(|(_, server)| server.enabled() && server.required())
        .map(|(name, _)| name.clone())
        .collect();
    required_mcp_servers.sort();
    let enabled_mcp_server_count = mcp_servers
        .values()
        .filter(|server| server.enabled())
        .count();
    let required_mcp_server_count = required_mcp_servers.len();
    let host_owned_codex_apps_enabled = features.apps_enabled_for_auth(
        auth_snapshot.is_some_and(codex_auth_types::RequestAuthSnapshot::uses_codex_backend),
    );
    let client_elicitation_support = McpClientElicitationSupport::from_auth_elicitation_enabled(
        features.enabled(Feature::AuthElicitation),
    );

    SessionMcpStartupPlan {
        required_mcp_servers,
        enabled_mcp_server_count,
        required_mcp_server_count,
        host_owned_codex_apps_enabled,
        client_elicitation_support,
    }
}

#[allow(clippy::await_holding_invalid_type)]
pub(super) async fn start_session_mcp_runtime(
    sess: &Arc<Session>,
    config: &Arc<Config>,
    session_configuration: &SessionConfiguration,
    auth_snapshot: Option<RequestAuthSnapshot>,
    mcp_servers: HashMap<String, EffectiveMcpServer>,
    auth_statuses: HashMap<String, McpAuthStatusEntry>,
    tx_event: Sender<Event>,
) -> anyhow::Result<()> {
    let startup_plan =
        derive_session_mcp_startup_plan(&config.features, auth_snapshot.as_ref(), &mcp_servers);
    let tool_plugin_provenance = sess
        .services
        .mcp_service
        .tool_plugin_provenance(sess.services.plugins_manager.as_ref(), config.as_ref())
        .await;
    let codex_apps_auth_context = sess
        .services
        .mcp_service
        .codex_apps_auth_context(auth_snapshot.as_ref());
    {
        let mut cancel_guard = sess.services.mcp_startup_cancellation_token.lock().await;
        cancel_guard.cancel();
        *cancel_guard = CancellationToken::new();
    }
    let turn_environment = crate::environment_selection::resolve_environment_selections(
        sess.services.environment_manager.as_ref(),
        &session_configuration.environments,
    )
    .map_err(|err| {
        CodexErr::InvalidRequest(err.to_string().replace(
            "unknown turn environment id",
            "unknown stored MCP environment id",
        ))
    })?
    .primary()
    .cloned();
    let local_environment = sess.services.environment_manager.local_environment();
    let mcp_runtime_environment = match turn_environment {
        Some(turn_environment) => sess.services.mcp_service.build_runtime_environment(
            Arc::clone(&turn_environment.environment),
            Arc::clone(&local_environment),
            turn_environment.cwd.to_path_buf(),
        ),
        None => {
            let environment = sess
                .services
                .environment_manager
                .default_environment()
                .unwrap_or_else(|| Arc::clone(&local_environment));
            sess.services.mcp_service.build_runtime_environment(
                environment,
                local_environment,
                session_configuration.cwd.to_path_buf(),
            )
        }
    };
    let mcp_connection_runtime_start = sess
        .services
        .mcp_service
        .start_connection_runtime(
            sess.services.mcp_connection_runtime_factory.as_ref(),
            mcp_service_api::McpConnectionRuntimeStartRequest {
                mcp_servers,
                store_mode: config.mcp_oauth_credentials_store_mode,
                auth_entries: auth_statuses,
                approval_policy: session_configuration.approval_policy.clone(),
                submit_id: INITIAL_SUBMIT_ID.to_owned(),
                tx_event,
                initial_permission_profile: session_configuration.permission_profile().clone(),
                runtime_environment: mcp_runtime_environment,
                codex_home: config.codex_home.to_path_buf(),
                codex_apps_tools_cache_key: codex_apps_tools_cache_key(
                    codex_apps_auth_context.as_ref(),
                ),
                host_owned_codex_apps_enabled: startup_plan.host_owned_codex_apps_enabled,
                client_elicitation_support: startup_plan.client_elicitation_support,
                tool_plugin_provenance,
                codex_apps_auth_provider: sess
                    .services
                    .mcp_service
                    .codex_apps_auth_provider(auth_snapshot.as_ref()),
                elicitation_reviewer: Some(sess.mcp_elicitation_reviewer()),
            },
        )
        .instrument(info_span!(
            "session_init.mcp_manager_init",
            otel.name = "session_init.mcp_manager_init",
            session_init.enabled_mcp_server_count = startup_plan.enabled_mcp_server_count,
            session_init.required_mcp_server_count = startup_plan.required_mcp_server_count,
        ))
        .await;
    let mcp_connection_manager = mcp_connection_runtime_start.runtime;
    let cancel_token = mcp_connection_runtime_start.startup_cancellation_token;
    {
        let mut manager_guard = sess.services.mcp_connection_manager.write().await;
        *manager_guard = mcp_connection_manager;
    }
    {
        let mut cancel_guard = sess.services.mcp_startup_cancellation_token.lock().await;
        if cancel_guard.is_cancelled() {
            cancel_token.cancel();
        }
        *cancel_guard = cancel_token;
    }
    if !startup_plan.required_mcp_servers.is_empty() {
        let failures = sess
            .services
            .mcp_connection_manager
            .read()
            .await
            .required_startup_failures(&startup_plan.required_mcp_servers)
            .instrument(info_span!(
                "session_init.required_mcp_wait",
                otel.name = "session_init.required_mcp_wait",
                session_init.required_mcp_server_count = startup_plan.required_mcp_server_count,
            ))
            .await;
        if !failures.is_empty() {
            let details = failures
                .iter()
                .map(|failure| format!("{}: {}", failure.server, failure.error))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("required MCP servers failed to initialize: {details}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_auth_types::AuthMode;
    use codex_auth_types::BearerRequestAuthSnapshot;
    use codex_config_types::McpServerConfig;
    use codex_config_types::McpServerTransportConfig;
    use codex_features::Features;

    fn managed_features(enabled_features: &[Feature]) -> ManagedFeatures {
        let mut features = Features::with_defaults();
        for feature in enabled_features {
            features.enable(*feature);
        }
        ManagedFeatures::from(features)
    }

    fn auth_snapshot(auth_mode: AuthMode) -> RequestAuthSnapshot {
        RequestAuthSnapshot::Bearer(BearerRequestAuthSnapshot {
            auth_mode,
            token: None,
            account_id: None,
            chatgpt_user_id: None,
            is_workspace_account: false,
            is_fedramp_account: false,
        })
    }

    fn mcp_server(enabled: bool, required: bool) -> EffectiveMcpServer {
        EffectiveMcpServer::configured(McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "mcp-server".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: Vec::new(),
                cwd: None,
            },
            experimental_environment: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled,
            required,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: Default::default(),
        })
    }

    #[test]
    fn mcp_startup_plan_sorts_only_enabled_required_servers() {
        let features = managed_features(&[]);
        let mcp_servers = HashMap::from([
            ("z-required".to_string(), mcp_server(true, true)),
            ("a-required".to_string(), mcp_server(true, true)),
            ("disabled-required".to_string(), mcp_server(false, true)),
            ("optional".to_string(), mcp_server(true, false)),
        ]);

        let plan = derive_session_mcp_startup_plan(&features, None, &mcp_servers);

        assert_eq!(
            plan.required_mcp_servers,
            vec!["a-required".to_string(), "z-required".to_string()]
        );
        assert_eq!(plan.enabled_mcp_server_count, 3);
        assert_eq!(plan.required_mcp_server_count, 2);
    }

    #[test]
    fn mcp_startup_plan_derives_auth_and_elicitation_flags() {
        let features = managed_features(&[Feature::Apps, Feature::AuthElicitation]);
        let mcp_servers = HashMap::new();

        let chatgpt_plan = derive_session_mcp_startup_plan(
            &features,
            Some(&auth_snapshot(AuthMode::Chatgpt)),
            &mcp_servers,
        );
        assert!(chatgpt_plan.host_owned_codex_apps_enabled);
        assert_eq!(
            chatgpt_plan.client_elicitation_support,
            McpClientElicitationSupport::AuthElicitation
        );

        let api_key_plan = derive_session_mcp_startup_plan(
            &features,
            Some(&auth_snapshot(AuthMode::ApiKey)),
            &mcp_servers,
        );
        assert!(!api_key_plan.host_owned_codex_apps_enabled);
        assert_eq!(
            api_key_plan.client_elicitation_support,
            McpClientElicitationSupport::AuthElicitation
        );
    }
}
