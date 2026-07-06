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
    let tool_plugin_provenance = sess
        .services
        .mcp_service
        .tool_plugin_provenance(sess.services.plugins_manager.as_ref(), config.as_ref())
        .await;
    let codex_apps_auth_context = sess
        .services
        .mcp_service
        .codex_apps_auth_context(auth_snapshot.as_ref());
    let host_owned_codex_apps_enabled = config.features.apps_enabled_for_auth(
        auth_snapshot
            .as_ref()
            .is_some_and(codex_auth_types::RequestAuthSnapshot::uses_codex_backend),
    );
    let client_elicitation_support = McpClientElicitationSupport::from_auth_elicitation_enabled(
        config.features.enabled(Feature::AuthElicitation),
    );
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
                host_owned_codex_apps_enabled,
                client_elicitation_support,
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
            session_init.enabled_mcp_server_count = enabled_mcp_server_count,
            session_init.required_mcp_server_count = required_mcp_server_count,
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
    if !required_mcp_servers.is_empty() {
        let failures = sess
            .services
            .mcp_connection_manager
            .read()
            .await
            .required_startup_failures(&required_mcp_servers)
            .instrument(info_span!(
                "session_init.required_mcp_wait",
                otel.name = "session_init.required_mcp_wait",
                session_init.required_mcp_server_count = required_mcp_server_count,
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
