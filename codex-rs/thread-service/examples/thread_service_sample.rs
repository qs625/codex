use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::io::Read;
use std::io::Write;
use std::sync::Arc;
use std::sync::Weak;

use anyhow::Context;
use anyhow::bail;
use app_server_protocol::item_event_to_server_notification;
use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_code_mode_api::DisabledCodeModeRuntimeFactory;
use codex_config_types::AuthCredentialsStoreMode;
use codex_config_types::History;
use codex_config_types::MemoriesConfig;
use codex_config_types::ModelAvailabilityNuxConfig;
use codex_config_types::Notice;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_config_types::OtelConfig;
use codex_config_types::SessionPickerViewMode;
use codex_config_types::ToolSuggestConfig;
use codex_config_types::TuiKeymap;
use codex_config_types::TuiNotificationSettings;
use codex_config_types::TuiPetAnchor;
use codex_config_types::UriBasedFileOpener;
use codex_exec_server::EnvironmentManager;
use codex_exec_server::ExecServerRuntimePaths;
use codex_extension_api::empty_extension_registry;
use codex_features::Features;
use codex_login::AuthManager;
use codex_login::default_client::set_default_originator;
use codex_login::model_provider_auth_manager;
use codex_utils_absolute_path::AbsolutePathBuf;
use model_service::DefaultModelProviderFactory;
use model_service_api::OPENAI_PROVIDER_ID;
use model_service_api::built_in_model_providers;
use protocol::config_types::AltScreenMode;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::WebSearchMode;
use protocol::models::PermissionProfile;
use protocol::protocol::AskForApproval;
use protocol::protocol::EventMsg;
use protocol::protocol::Op;
use protocol::protocol::SessionSource;
use protocol::user_input::UserInput;
use rollout::StateDbHandle;
use state_api::SharedStateDbRuntime;
use thread_service::CodexThread;
use thread_service::NewThread;
use thread_service::ThreadAuthRuntimes;
use thread_service::ThreadService;
use thread_service::config::Config;
use thread_service::config::ConfigLayerStack;
use thread_service::config::Constrained;
use thread_service::config::GhostSnapshotConfig;
use thread_service::config::MultiAgentV2Config;
use thread_service::config::Permissions;
use thread_service::config::ProjectConfig;
use thread_service::config::RealtimeAudioConfig;
use thread_service::config::RealtimeConfig;
use thread_service::config::TerminalResizeReflowConfig;
use thread_service::config::ThreadStoreConfig;
use thread_service::config::find_codex_home;
use thread_service::resolve_installation_id;
use thread_store::DefaultLiveThreadFactory;
use thread_store::ThreadStore;

async fn init_state_db(config: &Config) -> Option<StateDbHandle> {
    rollout::state_db::init(config).await
}

fn thread_store_from_config(
    config: &Config,
    state_db: Option<StateDbHandle>,
) -> Arc<dyn ThreadStore> {
    match &config.experimental_thread_store {
        ThreadStoreConfig::Local => Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config),
            state_db,
        )),
        ThreadStoreConfig::InMemory { id } => thread_store::InMemoryThreadStore::for_id(id),
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "thread-service-sample",
    about = "Run one Codex turn through ThreadService and print mapped notifications as newline-delimited JSON."
)]
struct Args {
    /// Override the model for this run.
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,

    /// Prompt text. If omitted, the prompt is read from piped stdin.
    #[arg(value_name = "PROMPT", num_args = 0.., trailing_var_arg = true)]
    prompt: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(run_main)
}

async fn run_main(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    if let Err(err) = set_default_originator("codex_thread_service_sample".to_string()) {
        tracing::warn!("failed to set originator: {err:?}");
    }

    let args = Args::parse();
    let prompt = if args.prompt.is_empty() {
        if std::io::stdin().is_terminal() {
            bail!("no prompt provided; pass a prompt argument or pipe one into stdin");
        }

        let mut prompt = String::new();
        std::io::stdin()
            .read_to_string(&mut prompt)
            .context("read prompt from stdin")?;
        let prompt = prompt.replace("\r\n", "\n").replace('\r', "\n");
        if prompt.trim().is_empty() {
            bail!("no prompt provided via stdin");
        }
        prompt
    } else {
        args.prompt.join(" ")
    };

    let config = new_config(args.model, arg0_paths)?;
    let state_db = init_state_db(&config).await;

    let auth_manager =
        AuthManager::shared_from_config(&config, /*enable_codex_api_key_env*/ false).await;
    let local_runtime_paths = ExecServerRuntimePaths::from_optional_paths(
        config.codex_self_exe.clone(),
        config.codex_linux_sandbox_exe.clone(),
    )?;
    let thread_store = thread_store_from_config(&config, state_db.clone());
    let shared_state_db: Option<SharedStateDbRuntime> = state_db
        .clone()
        .map(|state_db| state_db as SharedStateDbRuntime);
    let environment_manager = Arc::new(
        EnvironmentManager::from_codex_home(config.codex_home.clone(), local_runtime_paths).await?,
    );
    let installation_id = resolve_installation_id(&config.codex_home).await?;
    let thread_service: Arc<ThreadService> =
        Arc::new_cyclic(|thread_service: &Weak<ThreadService>| {
            let auth_runtimes = ThreadAuthRuntimes::from_auth_runtime(
                auth_manager.clone(),
                model_provider_auth_manager(Some(auth_manager.clone())),
            );
            let thread_service_api: Weak<dyn thread_service_api::ThreadServiceApi> =
                thread_service.clone();
            let command_service = Arc::new(command_service::CommandService::new());
            let approval_service = Arc::new(approval_service::ApprovalService);
            let goal_service = Arc::new(goal_service::GoalService);
            let mcp_service = Arc::new(mcp_service::McpService::new(approval_service.clone()));
            let workflow_service = Arc::new(codex_workflow::WorkflowService::new(
                config.codex_home.clone(),
                thread_service_api.clone(),
            ));
            let tool_service = Arc::new(codex_tool_service::ToolService::new(
                approval_service.clone(),
                command_service.clone(),
                goal_service.clone(),
                mcp_service.clone(),
                Arc::new(permissions_service::PermissionsService),
                workflow_service,
                thread_service_api,
            ));
            ThreadService::new(
                &config,
                auth_runtimes,
                SessionSource::Exec,
                environment_manager.clone(),
                empty_extension_registry(),
                /*analytics_events_client*/ None,
                Arc::clone(&thread_store),
                shared_state_db.clone(),
                Arc::new(DefaultLiveThreadFactory),
                installation_id.clone(),
                /*attestation_provider*/ None,
                Arc::new(DefaultModelProviderFactory),
                Arc::new(DisabledCodeModeRuntimeFactory),
                command_service,
                approval_service,
                goal_service,
                tool_service,
                mcp_service,
            )
        });

    let NewThread {
        thread_id, thread, ..
    } = thread_service
        .start_thread(config)
        .await
        .context("start Codex thread")?;

    let thread_id_string = thread_id.to_string();
    let turn_output = run_turn(&thread, &thread_id_string, prompt).await;
    let shutdown_result = thread.shutdown_and_wait().await;
    let _ = thread_service.remove_thread(&thread_id).await;

    turn_output?;
    shutdown_result.context("shut down Codex thread")?;

    Ok(())
}

fn new_config(model: Option<String>, arg0_paths: Arg0DispatchPaths) -> anyhow::Result<Config> {
    let codex_home = find_codex_home().context("find Codex home")?;
    let cwd = AbsolutePathBuf::current_dir().context("resolve current directory")?;
    let model_provider_id = OPENAI_PROVIDER_ID.to_string();
    let model_providers = built_in_model_providers(/*openai_base_url*/ None);
    let model_provider = model_providers
        .get(&model_provider_id)
        .context("OpenAI model provider should be available")?
        .clone();

    let mut config = Config {
        config_layer_stack: ConfigLayerStack::default(),
        startup_warnings: Vec::new(),
        bypass_hook_trust: false,
        model,
        service_tier: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_auto_compact_soft_ratio: None,
        model_auto_compact_hard_ratio: None,
        model_provider_id,
        model_provider,
        model_options: Vec::new(),
        personality: None,
        permissions: Permissions::from_approval_and_profile(
            Constrained::allow_any(AskForApproval::Never),
            Constrained::allow_any(PermissionProfile::read_only()),
        )?,
        approvals_reviewer: ApprovalsReviewer::User,
        enforce_residency: Constrained::allow_any(/*initial_value*/ None),
        hide_agent_reasoning: false,
        show_raw_agent_reasoning: false,
        user_instructions: None,
        base_instructions: None,
        developer_instructions: None,
        guardian_policy_config: None,
        include_permissions_instructions: false,
        include_apps_instructions: false,
        include_collaboration_mode_instructions: false,
        include_skill_instructions: false,
        include_environment_context: false,
        compact_prompt: None,
        notify: None,
        tui_notifications: TuiNotificationSettings::default(),
        animations: true,
        show_tooltips: true,
        model_availability_nux: ModelAvailabilityNuxConfig::default(),
        tui_alternate_screen: AltScreenMode::Auto,
        tui_status_line: None,
        tui_status_line_use_colors: true,
        tui_terminal_title: None,
        tui_theme: None,
        tui_raw_output_mode: false,
        tui_pet: None,
        tui_pet_anchor: TuiPetAnchor::Composer,
        terminal_resize_reflow: TerminalResizeReflowConfig::default(),
        tui_keymap: TuiKeymap::default(),
        tui_session_picker_view: SessionPickerViewMode::Dense,
        tui_vim_mode_default: false,
        cwd: cwd.clone(),
        workspace_roots: vec![cwd],
        workspace_roots_explicit: false,
        cli_auth_credentials_store_mode: AuthCredentialsStoreMode::File,
        mcp_servers: Constrained::allow_any(HashMap::new()),
        mcp_oauth_credentials_store_mode: OAuthCredentialsStoreMode::File,
        mcp_oauth_callback_port: None,
        mcp_oauth_callback_url: None,
        model_providers,
        project_doc_max_bytes: 32 * 1024,
        project_doc_fallback_filenames: Vec::new(),
        instruction_files: Vec::new(),
        tool_output_token_limit: None,
        agent_max_threads: Some(6),
        agent_job_max_runtime_seconds: None,
        agent_interrupt_message_enabled: false,
        agent_max_depth: 1,
        agent_roles: BTreeMap::new(),
        agent_tool_patterns: None,
        agent_skill_patterns: None,
        memories: MemoriesConfig::default(),
        sqlite_home: codex_home.to_path_buf(),
        log_dir: codex_home.join("log").to_path_buf(),
        config_lock_export_dir: None,
        config_lock_allow_codex_version_mismatch: false,
        config_lock_save_fields_resolved_from_model_catalog: true,
        config_lock_toml: None,
        codex_home,
        history: History::default(),
        ephemeral: true,
        file_opener: UriBasedFileOpener::VsCode,
        codex_self_exe: arg0_paths.codex_self_exe,
        codex_linux_sandbox_exe: arg0_paths.codex_linux_sandbox_exe,
        main_execve_wrapper_exe: arg0_paths.main_execve_wrapper_exe,
        zsh_path: None,
        model_reasoning_effort: None,
        plan_mode_reasoning_effort: None,
        model_reasoning_summary: None,
        model_supports_reasoning_summaries: None,
        model_catalog: None,
        model_verbosity: None,
        chatgpt_base_url: "https://chatgpt.com/backend-api/".to_string(),
        apps_mcp_path_override: None,
        realtime_audio: RealtimeAudioConfig::default(),
        experimental_realtime_ws_base_url: None,
        experimental_realtime_ws_model: None,
        realtime: RealtimeConfig::default(),
        experimental_realtime_ws_backend_prompt: None,
        experimental_realtime_ws_startup_context: None,
        experimental_realtime_start_instructions: None,
        experimental_thread_config_endpoint: None,
        experimental_thread_store: ThreadStoreConfig::Local,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search_mode: Constrained::allow_any(WebSearchMode::Disabled),
        web_search_config: None,
        use_experimental_unified_exec_tool: false,
        background_terminal_max_timeout: 300_000,
        ghost_snapshot: GhostSnapshotConfig::default(),
        multi_agent_v2: MultiAgentV2Config::default(),
        features: Default::default(),
        suppress_unstable_features_warning: false,
        active_profile: None,
        active_project: ProjectConfig { trust_level: None },
        notices: Notice::default(),
        check_for_update_on_startup: false,
        disable_paste_burst: false,
        analytics_enabled: Some(false),
        feedback_enabled: false,
        tool_suggest: ToolSuggestConfig::default(),
        otel: OtelConfig::default(),
    };
    config
        .features
        .set(Features::with_defaults())
        .context("configure default features")?;
    Ok(config)
}

async fn run_turn(thread: &CodexThread, thread_id: &str, prompt: String) -> anyhow::Result<()> {
    thread
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: prompt,
                text_elements: Vec::new(),
            }],
            environments: None,
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await
        .context("submit user input")?;

    let mut current_turn_id: Option<String> = None;
    let mut stdout = std::io::stdout().lock();
    loop {
        let event = thread.next_event().await.context("read Codex event")?;
        let notification = match &event.msg {
            EventMsg::TurnStarted(event) => {
                current_turn_id = Some(event.turn_id.clone());
                None
            }
            EventMsg::DynamicToolCallResponse(_)
            | EventMsg::McpToolCallBegin(_)
            | EventMsg::McpToolCallEnd(_)
            | EventMsg::CollabAgentSpawnBegin(_)
            | EventMsg::CollabAgentSpawnEnd(_)
            | EventMsg::CollabAgentInteractionBegin(_)
            | EventMsg::CollabAgentInteractionEnd(_)
            | EventMsg::CollabListAgentsBegin(_)
            | EventMsg::CollabListAgentsEnd(_)
            | EventMsg::CollabWaitingBegin(_)
            | EventMsg::CollabWaitingEnd(_)
            | EventMsg::CollabCloseBegin(_)
            | EventMsg::CollabCloseEnd(_)
            | EventMsg::CollabResumeBegin(_)
            | EventMsg::CollabResumeEnd(_)
            | EventMsg::AgentMessageContentDelta(_)
            | EventMsg::PlanDelta(_)
            | EventMsg::ReasoningContentDelta(_)
            | EventMsg::ReasoningRawContentDelta(_)
            | EventMsg::AgentReasoningSectionBreak(_)
            | EventMsg::ItemStarted(_)
            | EventMsg::ItemCompleted(_)
            | EventMsg::PatchApplyBegin(_)
            | EventMsg::PatchApplyUpdated(_)
            | EventMsg::TerminalInteraction(_)
            | EventMsg::ExecCommandBegin(_)
            | EventMsg::ExecCommandOutputDelta(_)
            | EventMsg::ExecCommandEnd(_) => item_event_to_server_notification(
                event.msg.clone(),
                thread_id,
                current_turn_id
                    .as_deref()
                    .context("mapped notification arrived before turn started")?,
            ),
            _ => None,
        };
        if let Some(notification) = notification {
            serde_json::to_writer(&mut stdout, &notification)
                .context("serialize mapped notification")?;
            stdout
                .write_all(b"\n")
                .context("write notification newline")?;
            stdout.flush().context("flush notification output")?;
        }

        match event.msg {
            EventMsg::TurnComplete(_) => {
                return Ok(());
            }
            EventMsg::Error(event) => {
                bail!(event.message);
            }
            EventMsg::TurnAborted(_) => {
                bail!("turn aborted");
            }
            EventMsg::ExecApprovalRequest(_) => {
                bail!("turn requested exec approval");
            }
            EventMsg::ApplyPatchApprovalRequest(_) => {
                bail!("turn requested patch approval");
            }
            EventMsg::RequestPermissions(_) => {
                bail!("turn requested permissions");
            }
            EventMsg::RequestUserInput(_) => {
                bail!("turn requested user input");
            }
            EventMsg::DynamicToolCallRequest(_) => {
                bail!("turn requested a dynamic tool call");
            }
            _ => {}
        }
    }
}
