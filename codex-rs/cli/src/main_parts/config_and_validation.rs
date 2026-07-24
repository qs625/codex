fn profile_v2_for_subcommand<'a>(
    interactive: &'a TuiCli,
    subcommand: &Subcommand,
) -> anyhow::Result<Option<&'a ProfileV2Name>> {
    let Some(profile_v2) = interactive.config_profile_v2.as_ref() else {
        return Ok(None);
    };

    match subcommand {
        Subcommand::Exec(_)
        | Subcommand::Review(_)
        | Subcommand::Resume(_)
        | Subcommand::Fork(_)
        | Subcommand::Debug(DebugCommand {
            subcommand: DebugSubcommand::PromptInput(_),
        }) => Ok(Some(profile_v2)),
        _ => anyhow::bail!(
            "--profile-v2 only applies to runtime commands: `codex`, `codex exec`, `codex review`, `codex resume`, `codex fork`, and `codex debug prompt-input`."
        ),
    }
}

async fn run_exec_server_command(
    cmd: ExecServerCommand,
    arg0_paths: &Arg0DispatchPaths,
) -> anyhow::Result<()> {
    let codex_self_exe = arg0_paths
        .codex_self_exe
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Codex executable path is not configured"))?;
    let runtime_paths =
        ExecServerRuntimePaths::new(codex_self_exe, arg0_paths.codex_linux_sandbox_exe.clone())?;
    if let Some(base_url) = cmd.remote {
        let executor_id = cmd
            .executor_id
            .ok_or_else(|| anyhow::anyhow!("--executor-id is required when --remote is set"))?;
        let mut remote_config =
            codex_exec_server::RemoteExecutorConfig::new(base_url, executor_id)?;
        if let Some(name) = cmd.name {
            remote_config.name = name;
        }
        codex_exec_server::run_remote_executor(remote_config, runtime_paths).await?;
        return Ok(());
    }
    let listen_url = cmd
        .listen
        .as_deref()
        .unwrap_or(codex_exec_server::DEFAULT_LISTEN_URL);
    codex_exec_server::run_main(listen_url, runtime_paths)
        .await
        .map_err(anyhow::Error::from_boxed)
}

async fn enable_feature_in_config(interactive: &TuiCli, feature: &str) -> anyhow::Result<()> {
    FeatureToggles::validate_feature(feature)?;
    let codex_home = find_codex_home()?;
    ConfigEditsBuilder::new(&codex_home)
        .with_profile(interactive.config_profile.as_deref())
        .set_feature_enabled(feature, /*enabled*/ true)
        .apply()
        .await?;
    println!("Enabled feature `{feature}` in config.toml.");
    maybe_print_under_development_feature_warning(&codex_home, interactive, feature);
    Ok(())
}

async fn disable_feature_in_config(interactive: &TuiCli, feature: &str) -> anyhow::Result<()> {
    FeatureToggles::validate_feature(feature)?;
    let codex_home = find_codex_home()?;
    ConfigEditsBuilder::new(&codex_home)
        .with_profile(interactive.config_profile.as_deref())
        .set_feature_enabled(feature, /*enabled*/ false)
        .apply()
        .await?;
    println!("Disabled feature `{feature}` in config.toml.");
    Ok(())
}

fn loader_overrides_for_profile(
    profile_v2: Option<&ProfileV2Name>,
) -> anyhow::Result<LoaderOverrides> {
    match profile_v2 {
        Some(profile_v2) => {
            let codex_home = find_codex_home()?;
            Ok(LoaderOverrides {
                user_config_path: Some(resolve_profile_v2_config_path(&codex_home, profile_v2)),
                user_config_profile: Some(profile_v2.clone()),
                ..Default::default()
            })
        }
        None => Ok(LoaderOverrides::default()),
    }
}

fn maybe_print_under_development_feature_warning(
    codex_home: &std::path::Path,
    interactive: &TuiCli,
    feature: &str,
) {
    if interactive.config_profile.is_some() {
        return;
    }

    let Some(spec) = FEATURES.iter().find(|spec| spec.key == feature) else {
        return;
    };
    if !matches!(spec.stage, Stage::UnderDevelopment) {
        return;
    }

    let config_path = codex_home.join(CONFIG_TOML_FILE);
    eprintln!(
        "Under-development features enabled: {feature}. Under-development features are incomplete and may behave unpredictably. To suppress this warning, set `suppress_unstable_features_warning = true` in {}.",
        config_path.display()
    );
}

async fn run_debug_trace_reduce_command(cmd: DebugTraceReduceCommand) -> anyhow::Result<()> {
    let output = cmd
        .output
        .unwrap_or_else(|| cmd.trace_bundle.join(REDUCED_STATE_FILE_NAME));

    let trace = replay_bundle(&cmd.trace_bundle)?;
    let reduced_json = serde_json::to_vec_pretty(&trace)?;
    tokio::fs::write(&output, reduced_json).await?;
    println!("{}", output.display());

    Ok(())
}

async fn run_debug_prompt_input_command(
    cmd: DebugPromptInputCommand,
    root_config_overrides: CliConfigOverrides,
    interactive: TuiCli,
    arg0_paths: Arg0DispatchPaths,
) -> anyhow::Result<()> {
    let loader_overrides = loader_overrides_for_profile(interactive.config_profile_v2.as_ref())?;
    let shared = interactive.shared.into_inner();
    let mut cli_kv_overrides = root_config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    if interactive.web_search {
        cli_kv_overrides.push((
            "web_search".to_string(),
            toml::Value::String("live".to_string()),
        ));
    }

    let approval_policy = if shared.dangerously_bypass_approvals_and_sandbox {
        Some(AskForApproval::Never)
    } else {
        interactive.approval_policy.map(Into::into)
    };
    let sandbox_mode = if shared.dangerously_bypass_approvals_and_sandbox {
        Some(protocol::config_types::SandboxMode::DangerFullAccess)
    } else {
        shared.sandbox_mode.map(Into::into)
    };
    let overrides = ConfigOverrides {
        model: shared.model,
        config_profile: shared.config_profile,
        approval_policy,
        sandbox_mode,
        cwd: shared.cwd,
        codex_self_exe: arg0_paths.codex_self_exe,
        codex_linux_sandbox_exe: arg0_paths.codex_linux_sandbox_exe,
        main_execve_wrapper_exe: arg0_paths.main_execve_wrapper_exe,
        show_raw_agent_reasoning: shared.oss.then_some(true),
        ephemeral: Some(true),
        bypass_hook_trust: shared.bypass_hook_trust.then_some(true),
        additional_writable_roots: shared.add_dir,
        ..Default::default()
    };
    let config = config_builder()
        .cli_overrides(cli_kv_overrides)
        .harness_overrides(overrides)
        .loader_overrides(loader_overrides)
        .build()
        .await?;

    let mut input = shared
        .images
        .into_iter()
        .chain(cmd.images)
        .map(|path| UserInput::LocalImage { path })
        .collect::<Vec<_>>();
    if let Some(prompt) = cmd.prompt.or(interactive.prompt) {
        input.push(UserInput::Text {
            text: prompt.replace("\r\n", "\n").replace('\r', "\n"),
            text_elements: Vec::new(),
        });
    }

    let local_runtime_paths = ExecServerRuntimePaths::from_optional_paths(
        config.codex_self_exe.clone(),
        config.codex_linux_sandbox_exe.clone(),
    )?;
    let environment_provider = Arc::new(
        EnvironmentManager::from_codex_home(config.codex_home.clone(), local_runtime_paths).await?,
    );

    let state_db: Option<StateDbHandle> = None;
    let thread_store = thread_store_from_config(&config, state_db.clone());
    let auth_manager =
        AuthManager::shared_from_config(&config, /*enable_codex_api_key_env*/ false).await;
    let auth_runtimes = ThreadAuthRuntimes::from_auth_runtime(
        auth_manager.clone(),
        codex_login::model_provider_auth_manager(Some(auth_manager)),
    );
    let missing_thread_runtime: Weak<dyn codex_workflow::WorkflowThreadRuntime> =
        Weak::<thread_service::ThreadService>::new();
    let missing_agent_runtime: Weak<dyn codex_tool_service::AgentToolRuntime> =
        Weak::<thread_service::ThreadService>::new();
    let workflow_service = Arc::new(codex_workflow::WorkflowService::new(
        config.codex_home.clone(),
        missing_thread_runtime,
    ));
    let prompt_input = thread_service::build_prompt_input(
        config,
        input,
        state_db.clone(),
        environment_provider,
        thread_store,
        Arc::new(thread_store::DefaultLiveThreadFactory),
        auth_runtimes,
        Arc::new(model_service::DefaultModelProviderFactory),
        Arc::new(approval_service::ApprovalService),
        Arc::new(codex_tool_service::ToolService::new(
            Arc::new(approval_service::ApprovalService),
            Arc::new(command_service::CommandService::new()),
            Arc::new(goal_service::GoalService),
            Arc::new(mcp_service::McpService::new(Arc::new(
                approval_service::ApprovalService,
            ))),
            Arc::new(permissions_service::PermissionsService),
            workflow_service,
            missing_agent_runtime,
        )),
        Arc::new(mcp_service::DefaultMcpAuthRuntime),
        Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&prompt_input)?);

    Ok(())
}

fn thread_store_from_config(
    config: &Config,
    state_db: Option<StateDbHandle>,
) -> Arc<dyn thread_store::ThreadStore> {
    match &config.experimental_thread_store {
        ThreadStoreConfig::Local => Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config),
            state_db,
        )),
        ThreadStoreConfig::InMemory { id } => thread_store::InMemoryThreadStore::for_id(id),
    }
}

async fn run_debug_models_command(
    cmd: DebugModelsCommand,
    root_config_overrides: CliConfigOverrides,
) -> anyhow::Result<()> {
    let catalog = if cmd.bundled {
        bundled_models_response()?
    } else {
        let cli_overrides = root_config_overrides
            .parse_overrides()
            .map_err(anyhow::Error::msg)?;
        let config = config_builder()
            .cli_overrides(cli_overrides)
            .build()
            .await?;
        let auth_manager =
            AuthManager::shared_from_config(&config, /*enable_codex_api_key_env*/ true).await;
        Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
            codex_home: config.codex_home.to_path_buf(),
            config_model_catalog: config.model_catalog.clone(),
            api_runtime_factory: Arc::new(model_service::DefaultApiRuntimeFactory),
            provider_auth_manager: codex_login::model_provider_auth_manager(Some(auth_manager)),
            model_provider_factory: Arc::new(model_service::DefaultModelProviderFactory),
            default_provider: Some(config.model_provider.clone()),
            providers_by_id: config.model_providers.clone(),
            model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
            attestation_provider: None,
        }))
        .raw_model_catalog(ModelCatalogRefresh::OnlineIfUncached)
        .await
        .map_err(anyhow::Error::msg)?
    };

    serde_json::to_writer(std::io::stdout(), &catalog)?;
    println!();
    Ok(())
}

async fn run_debug_clear_memories_command(
    root_config_overrides: &CliConfigOverrides,
    interactive: &TuiCli,
) -> anyhow::Result<()> {
    let cli_kv_overrides = root_config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    let overrides = ConfigOverrides {
        config_profile: interactive.config_profile.clone(),
        ..Default::default()
    };
    let config = config_builder()
        .cli_overrides(cli_kv_overrides)
        .harness_overrides(overrides)
        .build()
        .await?;

    let state_path = state_db_path(config.sqlite_home.as_path());
    let mut cleared_state_db = false;
    if tokio::fs::try_exists(&state_path).await? {
        let state_db =
            StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
                .await?;
        state_db.clear_memory_data().await?;
        cleared_state_db = true;
    }

    clear_memory_roots_contents(&config.codex_home).await?;

    let mut message = if cleared_state_db {
        format!("Cleared memory state from {}.", state_path.display())
    } else {
        format!("No state db found at {}.", state_path.display())
    };
    message.push_str(&format!(
        " Cleared memory directories under {}.",
        config.codex_home.display()
    ));

    println!("{message}");

    Ok(())
}
