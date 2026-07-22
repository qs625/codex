#[tokio::test]
async fn regular_turn_emits_turn_started_without_waiting_for_startup_prewarm() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let model_client_api = Arc::clone(&sess.services.model_client_api);
    let (_tx, startup_prewarm_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = startup_prewarm_rx.await;
        model_client_api
            .create_turn_client()
            .await
            .map_err(|err| protocol::error::CodexErr::Fatal(err.to_string()))
    });

    sess.set_session_startup_prewarm(
        crate::session_startup_prewarm::SessionStartupPrewarmHandle::new(
            handle,
            std::time::Instant::now(),
            crate::client::WEBSOCKET_CONNECT_TIMEOUT,
        ),
    )
    .await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        crate::tasks::RegularTask::new(),
    )
    .await;

    let first = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .expect("expected turn started event without waiting for startup prewarm")
        .expect("channel open");
    assert!(matches!(
        first.msg,
        EventMsg::TurnStarted(TurnStartedEvent { turn_id, .. }) if turn_id == tc.sub_id
    ));

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;
}

#[tokio::test]
async fn request_mcp_server_elicitation_auto_accepts_when_auto_deny_is_enabled() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    session
        .services
        .mcp_connection_manager
        .read()
        .await
        .set_elicitations_auto_deny(/*auto_deny*/ true);

    let requested_schema: McpElicitationSchema = serde_json::from_value(json!({
        "type": "object",
        "properties": {},
    }))
    .expect("schema should deserialize");
    let response = session
        .request_mcp_server_elicitation(
            turn_context.as_ref(),
            RequestId::String("request-1".into()),
            McpServerElicitationRequestParams {
                thread_id: session.conversation_id.to_string(),
                turn_id: Some(turn_context.sub_id.clone()),
                server_name: "codex_apps".to_string(),
                request: McpServerElicitationRequest::Form {
                    meta: None,
                    message: "Allow this request?".to_string(),
                    requested_schema,
                },
            },
        )
        .await;

    assert_eq!(
        response,
        Some(ElicitationResponse {
            action: ElicitationAction::Accept,
            content: Some(json!({})),
            meta: None,
        })
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn interrupting_regular_turn_waiting_on_startup_prewarm_emits_turn_aborted() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let model_client_api = Arc::clone(&sess.services.model_client_api);
    let (_tx, startup_prewarm_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = startup_prewarm_rx.await;
        model_client_api
            .create_turn_client()
            .await
            .map_err(|err| protocol::error::CodexErr::Fatal(err.to_string()))
    });

    sess.set_session_startup_prewarm(
        crate::session_startup_prewarm::SessionStartupPrewarmHandle::new(
            handle,
            std::time::Instant::now(),
            crate::client::WEBSOCKET_CONNECT_TIMEOUT,
        ),
    )
    .await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        crate::tasks::RegularTask::new(),
    )
    .await;

    let first = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .expect("expected turn started event without waiting for startup prewarm")
        .expect("channel open");
    assert!(matches!(
        first.msg,
        EventMsg::TurnStarted(TurnStartedEvent { turn_id, .. }) if turn_id == tc.sub_id
    ));

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    let second = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("expected turn aborted event")
        .expect("channel open");
    let EventMsg::TurnAborted(TurnAbortedEvent {
        turn_id,
        reason,
        completed_at,
        duration_ms,
    }) = second.msg
    else {
        panic!("expected turn aborted event");
    };
    assert_eq!(turn_id, Some(tc.sub_id.clone()));
    assert_eq!(reason, TurnAbortReason::Interrupted);
    assert!(completed_at.is_some());
    assert!(duration_ms.is_some());
}

pub(crate) fn build_test_model_service(
    config: &Config,
    session_configuration: &SessionConfiguration,
    provider_auth_manager: Option<model_service_api::SharedModelProviderAuthManager>,
    model_provider_factory: model_service_api::SharedModelProviderFactory,
) -> SharedModelServiceApi {
    Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
        codex_home: config.codex_home.to_path_buf(),
        config_model_catalog: config.model_catalog.clone(),
        api_runtime_factory: Arc::new(model_service::DefaultApiRuntimeFactory),
        provider_auth_manager,
        model_provider_factory,
        default_provider: Some(session_configuration.provider.clone()),
        providers_by_id: config.model_providers.clone(),
        model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
        attestation_provider: None,
    }))
}

pub(crate) fn build_test_model_service_for_config(
    config: &Config,
    provider_auth_manager: Option<model_service_api::SharedModelProviderAuthManager>,
    model_provider_factory: model_service_api::SharedModelProviderFactory,
) -> SharedModelServiceApi {
    Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
        codex_home: config.codex_home.to_path_buf(),
        config_model_catalog: config.model_catalog.clone(),
        api_runtime_factory: Arc::new(model_service::DefaultApiRuntimeFactory),
        provider_auth_manager,
        model_provider_factory,
        default_provider: Some(config.model_provider.clone()),
        providers_by_id: config.model_providers.clone(),
        model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
        attestation_provider: None,
    }))
}

fn developer_input_texts(items: &[ResponseItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "developer" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            ContentItem::InputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn developer_message_texts(items: &[ResponseItem]) -> Vec<Vec<&str>> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "developer" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .map(|content| {
            content
                .iter()
                .filter_map(|item| match item {
                    ContentItem::InputText { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

fn user_input_texts(items: &[ResponseItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            ContentItem::InputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn write_project_hooks(dot_codex: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dot_codex)?;
    std::fs::write(
        dot_codex.join("hooks.json"),
        r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo hello from hook"
          }
        ]
      }
    ]
  }
}"#,
    )
}

async fn write_project_trust_config(
    codex_home: &Path,
    trusted_projects: &[(&Path, TrustLevel)],
) -> std::io::Result<()> {
    tokio::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        toml::to_string(&ConfigToml {
            projects: Some(
                trusted_projects
                    .iter()
                    .map(|(project, trust_level)| {
                        (
                            project_trust_key(project),
                            ProjectConfig {
                                trust_level: Some(*trust_level),
                            },
                        )
                    })
                    .collect::<std::collections::HashMap<_, _>>(),
            ),
            ..Default::default()
        })
        .expect("serialize config"),
    )
    .await
}

async fn preview_session_start_hooks(
    config: &crate::config::Config,
) -> std::io::Result<Vec<protocol::protocol::HookRunSummary>> {
    let hooks = Hooks::new(HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &config.config_layer_stack,
            ),
        ),
        ..HooksConfig::default()
    });

    Ok(hooks.preview_session_start(&hooks::SessionStartRequest {
        session_id: ThreadId::new(),
        cwd: config.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    }))
}

pub(crate) fn test_tool_inputs(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
) -> Arc<crate::session::turn::TurnToolInputs> {
    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let default_agent_type_description =
        codex_agent_roles::spawn_tool_spec::build(&turn_context.config.agent_roles);
    let result = crate::session::turn::TurnToolInputs {
        session_capability: Arc::downgrade(&session_capability),
        mcp_tools: Vec::new(),
        deferred_mcp_tools: Vec::new(),
        discoverable_tools: Vec::new(),
        default_agent_type_description,
        expose_model_visible_tools: true,
    };
    let _ = (session, turn_context);
    Arc::new(result)
}

#[tokio::test]
async fn built_tools_include_custom_agent_roles_in_spawn_agent_schema() {
    let (session, turn_context, _rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |config| {
            config.agent_roles.insert(
                "custom".to_string(),
                codex_agent_roles::AgentRoleConfig {
                    description: Some("Custom agent role.".to_string()),
                    ..Default::default()
                },
            );
        },
    )
    .await;

    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let tool_inputs = crate::session::turn::built_tools(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::downgrade(&session_capability),
        &[],
        &std::collections::HashSet::new(),
        None,
        &CancellationToken::new(),
    )
    .await
    .expect("build tool inputs");
    let tool_specs = session.services.tool_service.model_visible_specs(
        crate::session::turn::tool_service_request(&session, &turn_context, &tool_inputs),
    );
    let spawn_agent_type_description = tool_specs
        .iter()
        .find_map(|tool| match tool {
            tool_service_api::ToolSpec::Function(tool) if tool.name == "spawn_agent" => tool
                .parameters
                .properties
                .as_ref()
                .and_then(|properties| properties.get("agent_type"))
                .and_then(|schema| schema.description.as_deref()),
            _ => None,
        })
        .expect("spawn_agent agent_type description");

    assert!(spawn_agent_type_description.contains("custom: {\nCustom agent role.\n}"));
}

#[tokio::test]
async fn compact_turn_hides_model_visible_tools_without_affecting_regular_turns() {
    let (session, turn_context, _rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |_config| {},
    )
    .await;

    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let regular_tool_inputs = crate::session::turn::built_tools(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::downgrade(&session_capability),
        &[],
        &std::collections::HashSet::new(),
        None,
        &CancellationToken::new(),
    )
    .await
    .expect("build regular tool inputs");
    assert!(
        regular_tool_inputs.expose_model_visible_tools,
        "regular turns should keep model-visible tools enabled by default"
    );

    let compact_session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let compact_tool_inputs = crate::session::turn::TurnToolInputs {
        session_capability: Arc::downgrade(&compact_session_capability),
        mcp_tools: Vec::new(),
        deferred_mcp_tools: Vec::new(),
        discoverable_tools: Vec::new(),
        default_agent_type_description: String::new(),
        expose_model_visible_tools: false,
    };
    assert!(
        !compact_tool_inputs.expose_model_visible_tools,
        "compact turns should disable model-visible tools"
    );
    let compact_specs =
        crate::session::turn::model_visible_tool_specs(&session, &turn_context, &compact_tool_inputs);
    assert!(
        compact_specs.is_empty(),
        "compact turns should not expose model-visible tools"
    );
}

pub(crate) async fn dispatch_exec_command_via_tool_service(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
    call_id: &str,
    arguments: serde_json::Value,
) -> Result<String, FunctionCallError> {
    let result = dispatch_tool_via_tool_service(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        call_id,
        tool_service_api::ToolName::plain("exec_command"),
        ToolCallSource::Direct,
        ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    )
    .await?;
    let response_item = result.result.to_response_item(call_id, &result.payload);
    match response_item {
        ResponseInputItem::FunctionCallOutput { output, .. }
        | ResponseInputItem::CustomToolCallOutput { output, .. } => {
            Ok(output.body.to_text().unwrap_or_default())
        }
        other => Err(FunctionCallError::Fatal(format!(
            "unexpected exec_command response item: {other:?}"
        ))),
    }
}

pub(crate) async fn dispatch_tool_via_tool_service(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
    call_id: &str,
    tool_name: tool_service_api::ToolName,
    source: ToolCallSource,
    payload: ToolPayload,
) -> Result<tool_service_api::AnyToolResult, FunctionCallError> {
    let tool_inputs = test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context));
    let tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    crate::session::turn::dispatch_tool_call(
        Arc::clone(&session.services.tool_service),
        Arc::clone(&session),
        Arc::clone(&turn_context),
        tool_inputs,
        tracker,
        tool_service_api::ToolCall {
            call_id: call_id.to_string(),
            tool_name,
            payload,
        },
        source,
        CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn beta_features_header_omits_remote_compaction_v2() -> anyhow::Result<()> {
    let mut config = ConfigBuilder::default().build().await?;
    config.features.enable(Feature::RemoteCompactionV2)?;

    let header = Session::build_model_client_beta_features_header(&config);

    let advertised_features = header.unwrap_or_default();
    assert!(
        !advertised_features
            .split(',')
            .any(|feature| feature == "remote_compaction_v2")
    );
    Ok(())
}

#[tokio::test]
async fn start_managed_network_proxy_applies_execpolicy_network_rules() -> anyhow::Result<()> {
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        /*requirements*/ None,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
    )?;
    let mut exec_policy = Policy::empty();
    exec_policy.add_network_rule(
        "example.com",
        NetworkRuleProtocol::Https,
        Decision::Allow,
        /*justification*/ None,
    )?;

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(
        current_cfg.network.allowed_domains(),
        Some(vec!["example.com".to_string()])
    );
    Ok(())
}

#[tokio::test]
async fn start_managed_network_proxy_ignores_invalid_execpolicy_network_rules() -> anyhow::Result<()>
{
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            domains: Some(NetworkDomainPermissionsToml {
                entries: std::collections::BTreeMap::from([(
                    "managed.example.com".to_string(),
                    NetworkDomainPermissionToml::Allow,
                )]),
            }),
            managed_allowed_domains_only: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
    )?;
    let mut exec_policy = Policy::empty();
    exec_policy.add_network_rule(
        "example.com",
        NetworkRuleProtocol::Https,
        Decision::Allow,
        /*justification*/ None,
    )?;

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(
        current_cfg.network.allowed_domains(),
        Some(vec!["managed.example.com".to_string()])
    );
    Ok(())
}

#[tokio::test]
async fn managed_network_proxy_decider_survives_full_access_start() -> anyhow::Result<()> {
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
    )?;
    let exec_policy = Policy::empty();
    let decider_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let network_policy_decider: Arc<dyn NetworkPolicyDecider> = Arc::new({
        let decider_calls = Arc::clone(&decider_calls);
        move |_request| {
            decider_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { NetworkDecision::ask("not_allowed") }
        }
    });

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
        Some(network_policy_decider),
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ true,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let spec = spec.recompute_for_permission_profile(&permission_profile_for_sandbox_policy(
        &SandboxPolicy::new_workspace_write_policy(),
    ))?;
    spec.apply_to_started_proxy(&started_proxy).await?;
    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(current_cfg.network.allowed_domains(), None);

    use tokio::io::AsyncReadExt as _;
    use tokio::io::AsyncWriteExt as _;

    let mut stream = tokio::net::TcpStream::connect(started_proxy.proxy().http_addr()).await?;
    stream
        .write_all(
            b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
        )
        .await?;
    let mut buffer = [0_u8; 4096];
    let bytes_read = tokio::time::timeout(StdDuration::from_secs(2), stream.read(&mut buffer))
        .await
        .expect("timed out waiting for proxy response")?;
    let response = String::from_utf8_lossy(&buffer[..bytes_read]);

    assert!(
        response.starts_with("HTTP/1.1 403 Forbidden"),
        "unexpected proxy response: {response}"
    );
    assert!(
        response.contains("x-proxy-error: blocked-by-allowlist"),
        "unexpected proxy response: {response}"
    );
    assert_eq!(
        decider_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "unexpected proxy response: {response}"
    );
    Ok(())
}

#[tokio::test]
async fn new_turn_refreshes_managed_network_proxy_for_sandbox_change() -> anyhow::Result<()> {
    let (mut session, _turn_context) = make_session_and_context().await;
    let initial_policy = SandboxPolicy::new_workspace_write_policy();

    let mut network_config = NetworkProxyConfig::default();
    network_config
        .network
        .set_allowed_domains(vec!["evil.com".to_string()]);
    let requirements = NetworkConstraints {
        domains: Some(NetworkDomainPermissionsToml {
            entries: std::collections::BTreeMap::from([(
                "*.example.com".to_string(),
                NetworkDomainPermissionToml::Allow,
            )]),
        }),
        ..Default::default()
    };
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        network_config,
        Some(requirements),
        &permission_profile_for_sandbox_policy(&initial_policy),
    )?;
    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &Policy::empty(),
        &permission_profile_for_sandbox_policy(&initial_policy),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;
    assert_eq!(
        started_proxy
            .proxy()
            .current_config()
            .await?
            .network
            .allowed_domains(),
        Some(vec!["*.example.com".to_string(), "evil.com".to_string()])
    );

    {
        let mut state = session.state.lock().await;
        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config.permissions.network = Some(spec);
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(initial_policy.clone(), cwd.as_path())
            .expect("test setup should allow sandbox policy");
        state.session_configuration.original_config_do_not_use = Arc::new(config);
        state
            .session_configuration
            .set_permission_profile_for_tests(PermissionProfile::from_legacy_sandbox_policy(
                &initial_policy,
            ))
            .expect("test setup should allow permission profile");
    }
    session.services.network_proxy = Some(started_proxy);

    session
        .new_turn_with_sub_id(
            "sandbox-policy-change".to_string(),
            SessionSettingsUpdate {
                sandbox_policy: Some(SandboxPolicy::DangerFullAccess),
                ..Default::default()
            },
        )
        .await?;

    let started_proxy = session
        .services
        .network_proxy
        .as_ref()
        .expect("managed network proxy should be present");
    assert_eq!(
        started_proxy
            .proxy()
            .current_config()
            .await?
            .network
            .allowed_domains(),
        Some(vec!["*.example.com".to_string()])
    );

    Ok(())
}

#[tokio::test]
async fn danger_full_access_turns_do_not_expose_managed_network_proxy() -> anyhow::Result<()> {
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
    )?;

    let session = make_session_with_config(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(SandboxPolicy::DangerFullAccess, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_none());
    Ok(())
}

#[tokio::test]
async fn workspace_write_turns_continue_to_expose_managed_network_proxy() -> anyhow::Result<()> {
    let sandbox_policy = SandboxPolicy::new_workspace_write_policy();
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&sandbox_policy),
    )?;

    let session = make_session_with_config(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(sandbox_policy, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_some());
    Ok(())
}

#[tokio::test]
async fn user_shell_commands_do_not_inherit_managed_network_proxy() -> anyhow::Result<()> {
    let sandbox_policy = SandboxPolicy::new_workspace_write_policy();
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&sandbox_policy),
    )?;

    let (session, rx) = make_session_with_config_and_rx(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(sandbox_policy, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_some());

    #[cfg(windows)]
    let command = r#"$val = $env:HTTP_PROXY; if ([string]::IsNullOrEmpty($val)) { $val = 'not-set' } ; [System.Console]::Write($val)"#.to_string();
    #[cfg(not(windows))]
    let command = r#"sh -c "printf '%s' \"${HTTP_PROXY:-not-set}\"""#.to_string();

    execute_user_shell_command(
        Arc::clone(&session),
        turn_context,
        command,
        CancellationToken::new(),
        UserShellCommandMode::StandaloneTurn,
    )
    .await;

    loop {
        let event = rx.recv().await.expect("channel open");
        if let EventMsg::ExecCommandEnd(event) = event.msg {
            assert_eq!(event.exit_code, 0);
            assert_eq!(event.stdout.trim(), "not-set");
            break;
        }
    }

    Ok(())
}

#[tokio::test]
async fn get_base_instructions_no_user_content() {
    let prompt_with_apply_patch_instructions =
        include_str!("../../../prompt_with_apply_patch_instructions.md");
    let models_response = bundled_models_response()
        .unwrap_or_else(|err| panic!("bundled models.json should parse: {err}"));
    let model_info_for_slug = |slug: &str, config: &Config| {
        let model = models_response
            .models
            .iter()
            .find(|candidate| candidate.slug == slug)
            .cloned()
            .unwrap_or_else(|| panic!("model slug {slug} is missing from models.json"));
        model_info::with_config_overrides(model, &config.to_models_manager_config())
    };
    let test_cases = vec![
        InstructionsTestCase {
            slug: "gpt-5.4",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.4-mini",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.3-codex",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.2",
            expects_apply_patch_description: false,
        },
    ];

    let (session, _turn_context) = make_session_and_context().await;
    let config = test_config().await;

    for test_case in test_cases {
        let model_info = model_info_for_slug(test_case.slug, &config);
        if test_case.expects_apply_patch_description {
            assert_eq!(
                model_info.base_instructions.as_str(),
                prompt_with_apply_patch_instructions
            );
        }

        {
            let mut state = session.state.lock().await;
            state.session_configuration.base_instructions = model_info.base_instructions.clone();
        }

        let base_instructions = session.get_base_instructions().await;
        assert_eq!(base_instructions.text, model_info.base_instructions);
    }
}

#[tokio::test]
async fn reload_user_config_layer_updates_effective_apps_config() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    std::fs::write(
        &config_toml_path,
        "[apps.calendar]\nenabled = false\ndestructive_enabled = false\n",
    )
    .expect("write user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    let apps_toml = config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("apps"))
        .cloned()
        .expect("apps table");
    let apps = config_service::types::AppsConfigToml::deserialize(apps_toml)
        .expect("deserialize apps config");
    let app = apps
        .apps
        .get("calendar")
        .expect("calendar app config exists");

    assert!(!app.enabled);
    assert_eq!(app.destructive_enabled, Some(false));
}

#[tokio::test]
async fn reload_user_config_layer_updates_base_and_selected_profile_layers() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let base_config_path = codex_home.join(CONFIG_TOML_FILE);
    let profile_config_path = codex_home.join("work.config.toml");
    std::fs::write(
        &base_config_path,
        "model = \"base\"\napproval_policy = \"on-failure\"\n",
    )
    .expect("write base user config");
    std::fs::write(&profile_config_path, "model = \"profile-old\"\n")
        .expect("write profile user config");
    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.to_path_buf())
        .loader_overrides(LoaderOverrides {
            user_config_path: Some(profile_config_path.abs()),
            user_config_profile: Some("work".parse().expect("profile-v2 name")),
            ..LoaderOverrides::without_managed_config_for_tests()
        })
        .build()
        .await
        .expect("load profile config");
    {
        let mut state = session.state.lock().await;
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    std::fs::write(
        &base_config_path,
        "model = \"base\"\napproval_policy = \"never\"\n",
    )
    .expect("update base user config");
    std::fs::write(&profile_config_path, "model = \"profile-new\"\n")
        .expect("update profile user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    assert_eq!(
        config
            .config_layer_stack
            .get_user_config_file()
            .map(codex_utils_absolute_path::AbsolutePathBuf::as_path),
        Some(profile_config_path.as_path())
    );
    let effective_user_config = config
        .config_layer_stack
        .effective_user_config()
        .expect("merged user config");
    assert_eq!(
        effective_user_config
            .get("model")
            .and_then(toml::Value::as_str),
        Some("profile-new")
    );
    assert_eq!(
        effective_user_config
            .get("approval_policy")
            .and_then(toml::Value::as_str),
        Some("never")
    );
}

#[tokio::test]
async fn reload_user_config_layer_refreshes_hooks() -> anyhow::Result<()> {
    let session = make_session_with_config(|config| {
        config
            .features
            .enable(Feature::CodexHooks)
            .expect("enable Codex hooks");
    })
    .await?;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home)?;
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    let user_config: config_service::TomlValue = serde_json::from_value(serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": "python3 /tmp/user.py",
                }],
            }],
        },
    }))?;

    let request = hooks::SessionStartRequest {
        session_id: session.conversation_id,
        cwd: session.get_config().await.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    };
    assert!(session.hooks().preview_session_start(&request).is_empty());

    let config = session.get_config().await;
    let hook_list = hooks::list_hooks(hooks::HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &config
                    .config_layer_stack
                    .with_user_config(&config_toml_path, user_config.clone()),
            ),
        ),
        ..hooks::HooksConfig::default()
    });
    assert_eq!(hook_list.hooks.len(), 1);
    assert_eq!(
        hook_list.hooks[0].trust_status,
        protocol::protocol::HookTrustStatus::Untrusted
    );

    let trusted_user_config: config_service::TomlValue =
        serde_json::from_value(serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "type": "command",
                        "command": "python3 /tmp/user.py",
                    }],
                }],
                "state": {
                    hook_list.hooks[0].key.clone(): {
                        "trusted_hash": hook_list.hooks[0].current_hash.clone(),
                    },
                },
            },
        }))?;
    std::fs::write(&config_toml_path, toml::to_string(&trusted_user_config)?)?;

    session.reload_user_config_layer().await;

    assert_eq!(session.hooks().preview_session_start(&request).len(), 1);
    Ok(())
}

#[tokio::test]
async fn refresh_runtime_config_refreshes_hooks() -> anyhow::Result<()> {
    let (session, _turn_context) = make_session_and_context().await;
    {
        let mut state = session.state.lock().await;
        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config
            .features
            .enable(Feature::CodexHooks)
            .expect("enable Codex hooks");
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home)?;
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    #[derive(serde::Serialize)]
    struct NormalizedHookIdentity {
        event_name: &'static str,
        #[serde(flatten)]
        group: config_service::MatcherGroup,
    }
    let trusted_hash = {
        let identity = NormalizedHookIdentity {
            event_name: "session_start",
            group: config_service::MatcherGroup {
                matcher: None,
                hooks: vec![config_service::HookHandlerConfig::Command {
                    command: "python3 /tmp/user.py".to_string(),
                    command_windows: None,
                    timeout_sec: Some(600),
                    r#async: false,
                    status_message: None,
                }],
            },
        };
        let identity = config_service::TomlValue::try_from(identity)?;
        config_service::version_for_toml(&identity)
    };
    let hook_key = format!("{}:session_start:0:0", config_toml_path.display());
    let trusted_user_config: config_service::TomlValue =
        serde_json::from_value(serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "type": "command",
                        "command": "python3 /tmp/user.py",
                    }],
                }],
                "state": {
                    hook_key: {
                        "trusted_hash": trusted_hash,
                    },
                },
            },
        }))?;
    std::fs::write(&config_toml_path, toml::to_string(&trusted_user_config)?)?;

    let request = hooks::SessionStartRequest {
        session_id: session.conversation_id,
        cwd: session.get_config().await.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    };
    assert!(session.hooks().preview_session_start(&request).is_empty());

    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    assert_eq!(session.hooks().preview_session_start(&request).len(), 1);
    Ok(())
}

#[tokio::test]
async fn reload_user_config_layer_updates_effective_tool_suggest_config() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    std::fs::write(
        &config_toml_path,
        r#"[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    assert_eq!(
        config.tool_suggest.disabled_tools,
        vec![
            ToolSuggestDisabledTool::connector("calendar"),
            ToolSuggestDisabledTool::plugin("slack@openai-curated"),
        ]
    );
}

#[tokio::test]
async fn refresh_runtime_config_updates_runtime_refreshable_fields_and_keeps_session_static_settings()
 {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"[apps.calendar]
enabled = false
destructive_enabled = false

[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");

    let original = session.get_config().await;
    let mut next_config = load_latest_config_for_session(&session).await;
    next_config.model = Some("gpt-5.4".to_string());
    next_config.notify = Some(vec!["echo".to_string()]);

    session.refresh_runtime_config(next_config).await;

    let config = session.get_config().await;
    let apps_toml = config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("apps"))
        .cloned()
        .expect("apps table");
    let apps = config_service::types::AppsConfigToml::deserialize(apps_toml)
        .expect("deserialize apps config");
    let app = apps
        .apps
        .get("calendar")
        .expect("calendar app config exists");

    assert!(!app.enabled);
    assert_eq!(app.destructive_enabled, Some(false));
    assert_eq!(config.model, original.model);
    assert_eq!(config.notify, original.notify);
    assert_eq!(
        config.tool_suggest.disabled_tools,
        vec![
            ToolSuggestDisabledTool::connector("calendar"),
            ToolSuggestDisabledTool::plugin("slack@openai-curated"),
        ]
    );
}

#[tokio::test]
async fn reconstruct_history_matches_live_compactions() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    let reconstruction_turn = session.new_default_turn().await;
    let reconstructed = session
        .reconstruct_history_from_rollout(reconstruction_turn.as_ref(), &rollout_items)
        .await;

    assert_eq!(expected, reconstructed.history);
}

#[tokio::test]
async fn reconstruct_history_uses_replacement_history_verbatim() {
    let (session, turn_context) = make_session_and_context().await;
    let summary_item = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    };
    let replacement_history = vec![
        summary_item.clone(),
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "stale developer instructions".to_string(),
            }],
            phase: None,
        },
    ];
    let rollout_items = vec![RolloutItem::Compacted(CompactedItem {
        message: String::new(),
        replacement_history: Some(replacement_history.clone()),
    })];

    let reconstructed = session
        .reconstruct_history_from_rollout(&turn_context, &rollout_items)
        .await;

    assert_eq!(reconstructed.history, replacement_history);
}

#[tokio::test]
async fn record_initial_history_reconstructs_resumed_transcript() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let history = session.state.lock().await.clone_history();
    assert_eq!(expected, history.raw_items());
}

#[tokio::test]
async fn record_initial_history_new_materializes_initial_context_immediately() {
    let (mut session, _turn_context) = make_session_and_context().await;
    let rollout_path = attach_thread_persistence(&mut session).await;

    session.record_initial_history(InitialHistory::New).await;

    let history = session.clone_history().await;
    assert!(
        !history.raw_items().is_empty(),
        "new threads should record initial context into history immediately"
    );
    let current_context = session.reference_context_item().await;
    assert!(
        current_context.is_some(),
        "new threads should seed a context baseline"
    );
    assert_eq!(session.previous_turn_settings().await, None);

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    assert!(
        resumed.history.iter().any(|item| matches!(
            item,
            RolloutItem::ResponseItem(ResponseItem::Message { .. })
        )),
        "materialized rollout should include the initial context messages"
    );
    let persisted_turn_context = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => Some(ctx.clone()),
        _ => None,
    });
    assert_eq!(
        serde_json::to_value(persisted_turn_context).expect("serialize persisted turn context"),
        serde_json::to_value(current_context).expect("serialize current turn context")
    );
}

#[tokio::test]
async fn resumed_history_injects_initial_context_on_first_context_update_only() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, mut expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let history_before_seed = session.state.lock().await.clone_history();
    assert_eq!(expected, history_before_seed.raw_items());

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    expected.extend(
        session
            .build_initial_context_for_model_visible_tools(&turn_context)
            .await,
    );
    let history_after_seed = session.clone_history().await;
    assert_eq!(expected, history_after_seed.raw_items());

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    let history_after_second_seed = session.clone_history().await;
    assert_eq!(
        history_after_seed.raw_items(),
        history_after_second_seed.raw_items()
    );
}

#[tokio::test]
async fn record_initial_history_seeds_token_info_from_rollout() {
    let (session, turn_context) = make_session_and_context().await;
    let (mut rollout_items, _expected) = sample_rollout(&session, &turn_context).await;

    let info1 = TokenUsageInfo {
        total_token_usage: TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
            total_tokens: 30,
        },
        last_token_usage: TokenUsage {
            input_tokens: 3,
            cached_input_tokens: 0,
            output_tokens: 4,
            reasoning_output_tokens: 0,
            total_tokens: 7,
        },
        model_context_window: Some(1_000),
    };
    let info2 = TokenUsageInfo {
        total_token_usage: TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 50,
            output_tokens: 200,
            reasoning_output_tokens: 25,
            total_tokens: 375,
        },
        last_token_usage: TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 5,
            total_tokens: 35,
        },
        model_context_window: Some(2_000),
    };

    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(info1),
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: None,
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(info2.clone()),
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: None,
            rate_limits: None,
        },
    )));

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let actual = session.state.lock().await.token_info();
    assert_eq!(actual, Some(info2));
}

#[tokio::test]
async fn thread_context_usage_recomputes_after_resume_without_persisted_snapshot() {
    let (session, turn_context) = make_session_and_context().await;
    let (mut rollout_items, _expected) = sample_rollout(&session, &turn_context).await;
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(TokenUsageInfo {
                total_token_usage: TokenUsage {
                    input_tokens: 100,
                    cached_input_tokens: 50,
                    output_tokens: 200,
                    reasoning_output_tokens: 25,
                    total_tokens: 375,
                },
                last_token_usage: TokenUsage {
                    input_tokens: 10,
                    cached_input_tokens: 0,
                    output_tokens: 20,
                    reasoning_output_tokens: 5,
                    total_tokens: 35,
                },
                model_context_window: Some(1_000),
            }),
            rate_limits: None,
        },
    )));

    assert!(
        rollout_items.iter().all(|item| !matches!(
            item,
            RolloutItem::EventMsg(EventMsg::ThreadContextUsageUpdated(_))
        )),
        "test history should reproduce rollouts without persisted context usage"
    );

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let usage = session.thread_context_usage().await;

    assert!(usage.total_bytes > 0);
    assert!(usage.categories.user_messages > 0);
    assert!(usage.categories.llm_messages > 0);
    assert_eq!(usage.budget_used_percent, Some(37));
}

#[tokio::test]
async fn thread_context_usage_counts_compaction_summary_as_compact() {
    let (session, turn_context) = make_session_and_context().await;
    let summary = format!(
        "{}\nThe earlier conversation was compacted into this summary.",
        crate::compact::SUMMARY_PREFIX
    );
    let item = user_message(&summary);
    session
        .record_into_history(std::slice::from_ref(&item), &turn_context)
        .await;

    let usage = session.thread_context_usage().await;

    assert!(usage.categories.compact > 0);
    assert_eq!(usage.categories.user_messages, 0);
}

#[tokio::test]
async fn thread_context_usage_counts_compaction_replacement_seed_as_compact() {
    let (session, _turn_context) = make_session_and_context().await;
    let user_item = user_message("recent user message");
    let compact_seed = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "compact final output seed".to_string(),
        }],
        phase: None,
    };
    let replacement_history = vec![user_item, compact_seed.clone()];

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: vec![RolloutItem::Compacted(CompactedItem {
                message: "compact final output seed".to_string(),
                replacement_history: Some(replacement_history),
            })],
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let compact_seed_bytes =
        codex_context_manager::estimate_response_item_model_visible_bytes(&compact_seed);
    let usage = session.thread_context_usage().await;

    assert_eq!(usage.categories.compact, compact_seed_bytes);
    assert_eq!(usage.categories.llm_messages, 0);
    assert!(usage.categories.user_messages > 0);
}

#[tokio::test]
async fn recompute_token_usage_uses_session_base_instructions() {
    let (session, turn_context) = make_session_and_context().await;

    let override_instructions = "SESSION_OVERRIDE_INSTRUCTIONS_ONLY".repeat(120);
    {
        let mut state = session.state.lock().await;
        state.session_configuration.base_instructions = override_instructions.clone();
    }

    let item = user_message("hello");
    session
        .record_into_history(std::slice::from_ref(&item), &turn_context)
        .await;

    let history = session.clone_history().await;
    let session_base_instructions = BaseInstructions {
        text: override_instructions,
    };
    let expected_tokens = history
        .estimate_token_count_with_base_instructions(&session_base_instructions)
        .expect("estimate with session base instructions");
    let model_estimated_tokens = history
        .estimate_token_count_with_base_instructions(&BaseInstructions {
            text: turn_context.model_info.get_model_instructions(
                turn_context.personality.or(turn_context.config.personality),
            ),
        })
        .expect("estimate with model instructions");
    assert_ne!(expected_tokens, model_estimated_tokens);

    session.recompute_token_usage(&turn_context).await;

    let actual_tokens = session
        .state
        .lock()
        .await
        .token_info()
        .expect("token info")
        .last_token_usage
        .total_tokens;
    assert_eq!(actual_tokens, expected_tokens.max(0));
}

#[tokio::test]
async fn recompute_token_usage_updates_model_context_window() {
    let (session, mut turn_context) = make_session_and_context().await;

    {
        let mut state = session.state.lock().await;
        state.set_token_info(Some(TokenUsageInfo {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(258_400),
        }));
    }

    turn_context.model_info.context_window = Some(128_000);
    turn_context.model_info.effective_context_window_percent = 100;

    session.recompute_token_usage(&turn_context).await;

    let actual = session.state.lock().await.token_info().expect("token info");
    assert_eq!(actual.model_context_window, Some(128_000));
}

#[tokio::test]
async fn record_token_usage_info_notifies_extension_contributors() {
    struct SessionTokenUsageMarker;
    struct ThreadTokenUsageMarker;

    #[derive(Debug, PartialEq, Eq)]
    struct RecordedTokenUsage {
        session_level_id: String,
        thread_level_id: String,
        turn_level_id: String,
        token_usage: TokenUsageInfo,
        saw_session_store: bool,
        saw_thread_store: bool,
    }

    struct TokenUsageRecorder {
        records: Arc<std::sync::Mutex<Vec<RecordedTokenUsage>>>,
    }

    impl codex_extension_api::TokenUsageContributor for TokenUsageRecorder {
        fn on_token_usage(
            &self,
            session_store: &codex_extension_api::ExtensionData,
            thread_store: &codex_extension_api::ExtensionData,
            turn_store: &codex_extension_api::ExtensionData,
            token_usage: &TokenUsageInfo,
        ) {
            self.records
                .lock()
                .expect("token usage records lock")
                .push(RecordedTokenUsage {
                    session_level_id: session_store.level_id().to_string(),
                    thread_level_id: thread_store.level_id().to_string(),
                    turn_level_id: turn_store.level_id().to_string(),
                    token_usage: token_usage.clone(),
                    saw_session_store: session_store.get::<SessionTokenUsageMarker>().is_some(),
                    saw_thread_store: thread_store.get::<ThreadTokenUsageMarker>().is_some(),
                });
        }
    }

    let (mut session, turn_context) = make_session_and_context().await;
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.token_usage_contributor(Arc::new(TokenUsageRecorder {
        records: Arc::clone(&records),
    }));
    session.services.extensions = Arc::new(builder.build());
    session
        .services
        .session_extension_data
        .insert(SessionTokenUsageMarker);
    session
        .services
        .thread_extension_data
        .insert(ThreadTokenUsageMarker);

    let first_usage = TokenUsage {
        input_tokens: 10,
        cached_input_tokens: 2,
        output_tokens: 20,
        reasoning_output_tokens: 3,
        total_tokens: 33,
    };
    let second_usage = TokenUsage {
        input_tokens: 7,
        cached_input_tokens: 1,
        output_tokens: 8,
        reasoning_output_tokens: 5,
        total_tokens: 20,
    };

    session
        .record_token_usage_info(&turn_context, Some(&first_usage))
        .await;
    session
        .record_token_usage_info(&turn_context, Some(&second_usage))
        .await;

    let mut expected_total_usage = first_usage.clone();
    expected_total_usage.add_assign(&second_usage);
    let expected = vec![
        RecordedTokenUsage {
            session_level_id: session.session_id().to_string(),
            thread_level_id: session.conversation_id.to_string(),
            turn_level_id: turn_context.sub_id.clone(),
            token_usage: TokenUsageInfo {
                total_token_usage: first_usage.clone(),
                last_token_usage: first_usage,
                model_context_window: turn_context.model_context_window(),
            },
            saw_session_store: true,
            saw_thread_store: true,
        },
        RecordedTokenUsage {
            session_level_id: session.session_id().to_string(),
            thread_level_id: session.conversation_id.to_string(),
            turn_level_id: turn_context.sub_id.clone(),
            token_usage: TokenUsageInfo {
                total_token_usage: expected_total_usage,
                last_token_usage: second_usage,
                model_context_window: turn_context.model_context_window(),
            },
            saw_session_store: true,
            saw_thread_store: true,
        },
    ];
    let actual = records
        .lock()
        .expect("token usage records lock")
        .drain(..)
        .collect::<Vec<_>>();
    assert_eq!(expected, actual);
}

#[tokio::test]
async fn config_change_contributor_observes_effective_config_changes() {
    struct SessionConfigMarker;
    struct ThreadConfigMarker;

    #[derive(Debug, PartialEq)]
    struct RecordedConfigChange {
        previous_model: Option<String>,
        new_model: Option<String>,
        previous_disabled_tools: Vec<ToolSuggestDisabledTool>,
        new_disabled_tools: Vec<ToolSuggestDisabledTool>,
        saw_session_store: bool,
        saw_thread_store: bool,
    }

    struct ConfigRecorder {
        records: Arc<std::sync::Mutex<Vec<RecordedConfigChange>>>,
    }

    impl codex_extension_api::ConfigContributor<crate::config::Config> for ConfigRecorder {
        fn on_config_changed(
            &self,
            session_store: &codex_extension_api::ExtensionData,
            thread_store: &codex_extension_api::ExtensionData,
            previous_config: &crate::config::Config,
            new_config: &crate::config::Config,
        ) {
            self.records
                .lock()
                .expect("config change records lock")
                .push(RecordedConfigChange {
                    previous_model: previous_config.model.clone(),
                    new_model: new_config.model.clone(),
                    previous_disabled_tools: previous_config.tool_suggest.disabled_tools.clone(),
                    new_disabled_tools: new_config.tool_suggest.disabled_tools.clone(),
                    saw_session_store: session_store.get::<SessionConfigMarker>().is_some(),
                    saw_thread_store: thread_store.get::<ThreadConfigMarker>().is_some(),
                });
        }
    }

    let (mut session, _turn_context) = make_session_and_context().await;
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.config_contributor(Arc::new(ConfigRecorder {
        records: Arc::clone(&records),
    }));
    session.services.extensions = Arc::new(builder.build());
    session
        .services
        .session_extension_data
        .insert(SessionConfigMarker);
    session
        .services
        .thread_extension_data
        .insert(ThreadConfigMarker);

    let original_model = session.collaboration_mode().await.model().to_string();
    let original_disabled_tools = session
        .get_config()
        .await
        .tool_suggest
        .disabled_tools
        .clone();
    let next_model = if original_model == "gpt-5.4" {
        "gpt-5.2"
    } else {
        "gpt-5.4"
    };
    let collaboration_mode = session.collaboration_mode().await.with_updates(
        Some(next_model.to_string()),
        /*effort*/ None,
        /*developer_instructions*/ None,
    );
    session
        .update_settings(SessionSettingsUpdate {
            collaboration_mode: Some(collaboration_mode),
            ..Default::default()
        })
        .await
        .expect("update settings");

    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");
    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    let expected_disabled_tools = vec![
        ToolSuggestDisabledTool::connector("calendar"),
        ToolSuggestDisabledTool::plugin("slack@openai-curated"),
    ];
    let expected = vec![
        RecordedConfigChange {
            previous_model: Some(original_model),
            new_model: Some(next_model.to_string()),
            previous_disabled_tools: original_disabled_tools.clone(),
            new_disabled_tools: original_disabled_tools.clone(),
            saw_session_store: true,
            saw_thread_store: true,
        },
        RecordedConfigChange {
            previous_model: Some(next_model.to_string()),
            new_model: Some(next_model.to_string()),
            previous_disabled_tools: original_disabled_tools,
            new_disabled_tools: expected_disabled_tools,
            saw_session_store: true,
            saw_thread_store: true,
        },
    ];
    let actual = records
        .lock()
        .expect("config change records lock")
        .drain(..)
        .collect::<Vec<_>>();
    assert_eq!(expected, actual);
}

#[tokio::test]
async fn record_initial_history_reconstructs_forked_transcript() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Forked(rollout_items))
        .await;

    let history = session.state.lock().await.clone_history();
    assert_eq!(expected, history.raw_items());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_configured_reports_permission_profile_for_external_sandbox() -> anyhow::Result<()>
{
    let server = start_mock_server().await;
    let sandbox_policy = SandboxPolicy::ExternalSandbox {
        network_access: protocol::protocol::NetworkAccess::Restricted,
    };
    let expected_sandbox_policy = sandbox_policy.clone();
    let mut builder = test_codex().with_config(move |config| {
        config
            .permissions
            .set_permission_profile(PermissionProfile::from_legacy_sandbox_policy(
                &sandbox_policy,
            ))
            .expect("set permission profile");
        config
            .set_legacy_sandbox_policy(sandbox_policy)
            .expect("set sandbox policy");
    });

    let test = builder.build(&server).await?;

    let expected_permission_profile =
        protocol::models::PermissionProfile::from_legacy_sandbox_policy(&expected_sandbox_policy);
    assert_eq!(
        test.session_configured.permission_profile, expected_permission_profile,
        "ExternalSandbox is represented explicitly instead of as a lossy root-write profile"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_permission_profile_rebinds_runtime_workspace_roots() -> anyhow::Result<()> {
    let codex_home = tempfile::TempDir::new()?;
    let cwd = tempfile::TempDir::new()?;
    let old_root = test_path_buf("/workspace/old").abs();
    let new_root = test_path_buf("/workspace/new").abs();
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(crate::config::ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            default_permissions: Some(BUILT_IN_PERMISSION_PROFILE_WORKSPACE.to_string()),
            additional_writable_roots: vec![old_root.to_path_buf()],
            ..Default::default()
        })
        .build()
        .await?;

    let session_permission_profile_state = session_permission_profile_state_from_config(&config)?;
    let stored_file_system_policy = session_permission_profile_state
        .permission_profile()
        .file_system_sandbox_policy();
    assert!(
        !stored_file_system_policy
            .can_write_path_with_cwd(old_root.as_path(), config.cwd.as_path()),
        "session permission profile state should keep runtime workspace roots symbolic"
    );

    let mut session_configuration = make_session_configuration_for_tests().await;
    session_configuration.cwd = config.cwd.clone();
    session_configuration.workspace_roots = config.workspace_roots.clone();
    session_configuration.permission_profile_state = session_permission_profile_state;

    let initial_policy = session_configuration.file_system_sandbox_policy();
    assert!(initial_policy.can_write_path_with_cwd(old_root.as_path(), config.cwd.as_path()));

    let updated = session_configuration.apply(&SessionSettingsUpdate {
        workspace_roots: Some(vec![new_root.clone()]),
        ..Default::default()
    })?;
    let updated_policy = updated.file_system_sandbox_policy();
    assert!(updated_policy.can_write_path_with_cwd(new_root.as_path(), updated.cwd.as_path()));
    assert!(!updated_policy.can_write_path_with_cwd(old_root.as_path(), updated.cwd.as_path()));
    Ok(())
}
