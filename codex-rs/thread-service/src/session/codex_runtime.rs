use super::*;
use codex_file_system::LOCAL_FS;
use model_service::model_info::ensure_inline_artifact_instructions;

/// The high-level interface to the Codex system.
/// It operates as a queue pair where you send submissions and receive events.
pub struct Codex {
    pub(crate) tx_sub: Sender<Submission>,
    pub(crate) rx_event: Receiver<Event>,
    // Last known status of the agent.
    pub(crate) agent_status: watch::Receiver<AgentStatus>,
    pub(crate) session: Arc<Session>,
    // Shared future for the background submission loop completion so multiple
    // callers can wait for shutdown.
    pub(crate) session_loop_termination: SessionLoopTermination,
}

pub(crate) type SessionLoopTermination = Shared<BoxFuture<'static, ()>>;

/// Wrapper returned by [`Codex::spawn`] containing the spawned [`Codex`] and
/// the unique session id.
pub struct CodexSpawnOk {
    pub codex: Codex,
    pub thread_id: ThreadId,
}

pub(crate) struct CodexSpawnArgs {
    pub(crate) config: Config,
    pub(crate) installation_id: String,
    pub(crate) terminal_type: String,
    pub(crate) auth_runtime: SharedAuthRuntime,
    pub(crate) provider_auth_manager: Option<SharedModelProviderAuthManager>,
    pub(crate) model_provider_factory: SharedModelProviderFactory,
    pub(crate) api_runtime_factory: SharedApiRuntimeFactory,
    pub(crate) session_telemetry_factory: SharedSessionTelemetryFactory,
    pub(crate) memory_tool_developer_instructions_provider:
        SharedMemoryToolDeveloperInstructionsProvider,
    pub(crate) hook_runtime_factory: SharedHookRuntimeFactory,
    pub(crate) sandbox_runtime: SharedSandboxRuntime,
    pub(crate) environment_manager: Arc<dyn ExecEnvironmentProvider>,
    pub(crate) skill_service: SharedSkillServiceApi,
    pub(crate) plugins_manager: SharedPluginRuntime,
    pub(crate) mcp_service: Arc<dyn McpServiceApi>,
    pub(crate) mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
    pub(crate) mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
    pub(crate) network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
    pub(crate) command_service_api: Arc<dyn command_service_api::CommandServiceApi>,
    pub(crate) extensions: Arc<codex_extension_api::ExtensionRegistry<config_service::Config>>,
    pub(crate) conversation_history: InitialHistory,
    pub(crate) session_source: SessionSource,
    pub(crate) thread_source: Option<ThreadSource>,
    pub(crate) root_agent_metadata: Option<codex_agent_runtime::AgentMetadata>,
    pub(crate) agent_control: AgentControl,
    pub(crate) dynamic_tools: Vec<DynamicToolSpec>,
    pub(crate) persist_extended_history: bool,
    pub(crate) metrics_service_name: Option<String>,
    pub(crate) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(crate) inherited_exec_policy: Option<Arc<ExecPolicyManager>>,
    pub(crate) exec_policy_loader: Arc<dyn ExecPolicyLoader>,
    /// Parent rollout trace used only to derive fresh spawned child traces.
    ///
    /// Root sessions and non-thread-spawn subagents pass a disabled context;
    /// `Session::new` creates the root trace itself when rollout tracing is enabled.
    pub(crate) parent_rollout_thread_trace: ThreadTraceContext,
    pub(crate) user_shell_override: Option<shell::Shell>,
    pub(crate) parent_trace: Option<W3cTraceContext>,
    pub(crate) environment_selections: ResolvedTurnEnvironments,
    pub(crate) analytics_events_client: Option<AnalyticsEventsClient>,
    pub(crate) thread_store: Arc<dyn ThreadStore>,
    pub(crate) state_db: Option<StateDbHandle>,
    pub(crate) live_thread_factory: Arc<dyn LiveThreadFactory>,
    pub(crate) attestation_provider: Option<Arc<dyn AttestationProvider>>,
    pub(crate) active_event_subscriptions: Arc<crate::ActiveEventSubscriptionTracker>,
    pub(crate) openai_file_uploader: SharedOpenAiFileUploader,
    pub(crate) code_mode_service: Arc<dyn CodeModeRuntimeService>,
    pub(crate) code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
    pub(crate) approval_service: Arc<dyn ApprovalServiceApi>,
    pub(crate) goal_service: Arc<dyn goal_service_api::GoalServiceApi>,
    pub(crate) tool_service: Arc<crate::ToolServiceApi>,
}

pub(crate) const INITIAL_SUBMIT_ID: &str = "";
pub(crate) const SUBMISSION_CHANNEL_CAPACITY: usize = 512;
pub(crate) const CYBER_VERIFY_URL: &str = "https://chatgpt.com/cyber";
pub(crate) const CYBER_SAFETY_URL: &str =
    "https://developers.openai.com/codex/concepts/cyber-safety";

fn initial_agent_status_from_history(initial_history: &InitialHistory) -> AgentStatus {
    let InitialHistory::Resumed(resumed) = initial_history else {
        return AgentStatus::PendingInit;
    };
    resumed
        .history
        .iter()
        .filter_map(|item| match item {
            RolloutItem::EventMsg(event) => agent_status_from_event(event),
            _ => None,
        })
        .next_back()
        .filter(is_final)
        .unwrap_or(AgentStatus::PendingInit)
}

/// Owns a live thread while session initialization is still fallible.
pub(crate) struct LiveThreadInitGuard {
    live_thread: Option<SharedLiveThread>,
}

impl LiveThreadInitGuard {
    pub(crate) fn new(live_thread: Option<SharedLiveThread>) -> Self {
        Self { live_thread }
    }

    pub(crate) fn as_ref(&self) -> Option<&SharedLiveThread> {
        self.live_thread.as_ref()
    }

    pub(crate) fn commit(&mut self) {
        self.live_thread = None;
    }

    pub(crate) async fn discard(&mut self) {
        let Some(live_thread) = self.live_thread.take() else {
            return;
        };
        if let Err(err) = live_thread.discard().await {
            warn!("failed to discard thread persistence for failed session init: {err}");
        }
    }
}

impl Drop for LiveThreadInitGuard {
    fn drop(&mut self) {
        let Some(live_thread) = self.live_thread.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            warn!("failed to discard thread persistence for failed session init: no Tokio runtime");
            return;
        };
        handle.spawn(async move {
            if let Err(err) = live_thread.discard().await {
                warn!("failed to discard thread persistence for failed session init: {err}");
            }
        });
    }
}

impl Codex {
    /// Spawn a new [`Codex`] and initialize the session.
    pub(crate) async fn spawn(args: CodexSpawnArgs) -> CodexResult<CodexSpawnOk> {
        let parent_trace = match args.parent_trace {
            Some(trace) => {
                if context_from_w3c_trace_context(&trace).is_some() {
                    Some(trace)
                } else {
                    warn!("ignoring invalid thread spawn trace carrier");
                    None
                }
            }
            None => None,
        };
        let thread_spawn_span = info_span!("thread_spawn", otel.name = "thread_spawn");
        if let Some(trace) = parent_trace.as_ref() {
            let _ = set_parent_from_w3c_trace_context(&thread_spawn_span, trace);
        }
        Self::spawn_internal(CodexSpawnArgs {
            parent_trace,
            ..args
        })
        .instrument(thread_spawn_span)
        .await
    }

    async fn spawn_internal(args: CodexSpawnArgs) -> CodexResult<CodexSpawnOk> {
        let CodexSpawnArgs {
            mut config,
            installation_id,
            terminal_type,
            auth_runtime,
            provider_auth_manager,
            model_provider_factory,
            api_runtime_factory,
            session_telemetry_factory,
            environment_manager,
            sandbox_runtime,
            skill_service,
            plugins_manager,
            mcp_service,
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
            network_proxy_runtime_factory,
            command_service_api,
            hook_runtime_factory,
            extensions,
            conversation_history,
            session_source,
            thread_source,
            root_agent_metadata,
            agent_control,
            dynamic_tools,
            persist_extended_history,
            metrics_service_name,
            inherited_shell_snapshot,
            user_shell_override,
            inherited_exec_policy,
            exec_policy_loader,
            parent_rollout_thread_trace,
            parent_trace: _,
            environment_selections,
            analytics_events_client,
            thread_store,
            state_db,
            live_thread_factory,
            attestation_provider,
            active_event_subscriptions,
            openai_file_uploader,
            code_mode_service,
            code_mode_runtime_factory,
            approval_service,
            goal_service,
            tool_service,
            memory_tool_developer_instructions_provider,
        } = args;
        let (tx_sub, rx_sub) = async_channel::bounded(SUBMISSION_CHANNEL_CAPACITY);
        let (tx_event, rx_event) = async_channel::unbounded();
        let fs = environment_selections.primary_filesystem();
        let plugins_input = config.plugins_config_input();
        merge_plugin_agent_roles_for_config(
            plugins_manager.as_ref(),
            &plugins_input,
            &mut config.agent_roles,
            &mut config.startup_warnings,
        )
        .await;
        let effective_skill_roots = plugins_manager
            .effective_skill_roots_for_config(&plugins_input)
            .await;
        let skills_input = build_skill_service_input_from_config(&config, effective_skill_roots);
        let loaded_skills = skill_service.skills_for_config(&skills_input, fs).await;

        for err in &loaded_skills.errors {
            error!(
                "failed to load skill {}: {}",
                err.path.display(),
                err.message
            );
        }

        let user_instructions = AgentsMdManager::new(&config)
            .user_instructions_with_fs(LOCAL_FS.as_ref())
            .await;

        let exec_policy = if is_guardian_reviewer_source(&session_source) {
            // Guardian review should rely on the built-in shell safety checks,
            // not on caller-provided exec-policy rules that could shape the
            // reviewer or silently auto-approve commands.
            Arc::new(ExecPolicyManager::default())
        } else if let Some(exec_policy) = &inherited_exec_policy {
            Arc::clone(exec_policy)
        } else {
            Arc::new(
                ExecPolicyManager::load(&config.config_layer_stack, exec_policy_loader.as_ref())
                    .await
                    .map_err(|err| CodexErr::Fatal(format!("failed to load rules: {err}")))?,
            )
        };

        let config = Arc::new(config);
        let model_service = Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
            codex_home: config.codex_home.to_path_buf(),
            config_model_catalog: config.model_catalog.clone(),
            api_runtime_factory: Arc::clone(&api_runtime_factory),
            provider_auth_manager: provider_auth_manager.clone(),
            model_provider_factory: Arc::clone(&model_provider_factory),
            default_provider: Some(config.model_provider.clone()),
            providers_by_id: config.model_providers.clone(),
            model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
            attestation_provider: attestation_provider.clone(),
        }));
        let refresh = if session_source.is_non_root_agent() {
            ModelCatalogRefresh::Offline
        } else {
            ModelCatalogRefresh::OnlineIfUncached
        };
        if config.model.is_none() || !matches!(refresh, ModelCatalogRefresh::Offline) {
            let _ = model_service
                .list_models(ListModelsRequest {
                    include_hidden: true,
                    refresh,
                })
                .await;
        }
        let model = model_service
            .resolve_default_model(ResolveDefaultModelRequest {
                selection: ModelSelectionPolicy {
                    requested_model: config.model.clone(),
                    provider_hint: Some(config.model_provider_id.clone()),
                    allow_default_fallback: true,
                    refresh,
                },
            })
            .await
            .map_err(|err| CodexErr::Fatal(format!("failed to resolve default model: {err}")))?
            .map(|preset| preset.model)
            .unwrap_or_else(|| config.model.clone().unwrap_or_default());

        // Resolve base instructions for the session. Priority order:
        // 1. config.base_instructions override
        // 2. conversation history => session_meta.base_instructions
        // 3. base_instructions for current model
        let model_info = model_service
            .get_model_info(model.as_str())
            .await
            .map_err(|err| CodexErr::Fatal(format!("failed to resolve model info: {err}")))?;
        let base_instructions = config
            .base_instructions
            .clone()
            .or_else(|| conversation_history.get_base_instructions().map(|s| s.text))
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality));
        let base_instructions = ensure_inline_artifact_instructions(base_instructions);

        // Respect thread-start tools. When missing (resumed/forked threads), read from the db
        // first, then fall back to rollout-file tools.
        let persisted_tools = if dynamic_tools.is_empty() {
            let thread_id = match &conversation_history {
                InitialHistory::Resumed(resumed) => Some(resumed.conversation_id),
                InitialHistory::Forked(_) => conversation_history.forked_from_id(),
                InitialHistory::New | InitialHistory::Cleared => None,
            };
            match thread_id {
                Some(thread_id) => {
                    let state_db_ctx = if config.ephemeral {
                        None
                    } else {
                        state_db.clone()
                    };
                    state_db::get_dynamic_tools(state_db_ctx.as_deref(), thread_id, "codex_spawn")
                        .await
                }
                None => None,
            }
        } else {
            None
        };
        let dynamic_tools = if dynamic_tools.is_empty() {
            persisted_tools
                .or_else(|| conversation_history.get_dynamic_tools())
                .unwrap_or_default()
        } else {
            dynamic_tools
        };
        // TODO (aibrahim): Consolidate config.model and config.model_reasoning_effort into config.collaboration_mode
        // to avoid extracting these fields separately and constructing CollaborationMode here.
        let collaboration_mode = CollaborationMode {
            mode: ModeKind::Default,
            settings: Settings {
                model: model.clone(),
                reasoning_effort: config.model_reasoning_effort,
                developer_instructions: None,
            },
        };
        let auth_runtime_ref: &dyn AuthRuntime = auth_runtime.as_ref();
        let uses_enterprise_default_service_tier = auth_runtime_ref
            .telemetry_snapshot()
            .uses_enterprise_default_service_tier;
        let service_tier = resolve_session_service_tier(
            config.service_tier.clone(),
            config.notices.fast_default_opt_out.unwrap_or(false),
            uses_enterprise_default_service_tier,
            config.features.enabled(Feature::FastMode),
        );
        let session_configuration = SessionConfiguration {
            provider: config.model_provider.clone(),
            collaboration_mode,
            model_reasoning_summary: config.model_reasoning_summary,
            service_tier,
            developer_instructions: config.developer_instructions.clone(),
            user_instructions,
            personality: config.personality,
            base_instructions,
            compact_prompt: config.compact_prompt.clone(),
            approval_policy: config.permissions.approval_policy.clone(),
            approval_policy_is_session_override:
                SessionConfiguration::approval_policy_is_session_override(&config),
            approvals_reviewer: config.approvals_reviewer,
            permission_profile_state: session_permission_profile_state_from_config(&config)?,
            permission_profile_is_session_override:
                SessionConfiguration::permission_profile_is_session_override(&config),
            windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
            cwd: config.cwd.clone(),
            workspace_roots: config.workspace_roots.clone(),
            codex_home: config.codex_home.clone(),
            thread_name: None,
            environments: environment_selections.to_selections(),
            original_config_do_not_use: Arc::clone(&config),
            metrics_service_name,
            terminal_type,
            app_server_client_name: None,
            app_server_client_version: None,
            session_source,
            thread_source,
            root_agent_metadata,
            dynamic_tools,
            persist_extended_history,
            inherited_shell_snapshot,
            user_shell_override,
        };

        // Generate a unique ID for the lifetime of this Codex session.
        let session_source_clone = session_configuration.session_source.clone();
        let initial_agent_status = initial_agent_status_from_history(&conversation_history);
        let (agent_status_tx, agent_status_rx) = watch::channel(initial_agent_status);

        let session = Session::new(
            session_configuration,
            config.clone(),
            installation_id,
            auth_runtime,
            provider_auth_manager,
            model_provider_factory,
            exec_policy,
            exec_policy_loader,
            tx_event.clone(),
            agent_status_tx.clone(),
            conversation_history,
            session_source_clone,
            skill_service,
            plugins_manager,
            mcp_service,
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
            api_runtime_factory,
            session_telemetry_factory,
            memory_tool_developer_instructions_provider,
            model_service,
            hook_runtime_factory,
            sandbox_runtime,
            network_proxy_runtime_factory,
            command_service_api,
            extensions,
            agent_control,
            environment_manager,
            analytics_events_client,
            thread_store,
            state_db,
            live_thread_factory,
            parent_rollout_thread_trace,
            attestation_provider,
            active_event_subscriptions,
            openai_file_uploader,
            code_mode_service,
            code_mode_runtime_factory,
            approval_service,
            goal_service,
            tool_service,
        )
        .await
        .map_err(|e| {
            error!("Failed to create session: {e:#}");
            map_session_init_error(&e, &config.codex_home)
        })?;
        let thread_id = session.conversation_id;

        // This task will run until Op::Shutdown is received.
        let session_for_loop = Arc::clone(&session);
        let session_loop_handle = tokio::spawn(async move {
            submission_loop(session_for_loop, config, rx_sub)
                .instrument(info_span!("session_loop", thread_id = %thread_id))
                .await;
        });
        let codex = Codex {
            tx_sub,
            rx_event,
            agent_status: agent_status_rx,
            session,
            session_loop_termination: session_loop_termination_from_handle(session_loop_handle),
        };

        Ok(CodexSpawnOk { codex, thread_id })
    }

    /// Submit the `op` wrapped in a `Submission` with a unique ID.
    pub async fn submit(&self, op: Op) -> CodexResult<String> {
        self.submit_with_trace(op, /*trace*/ None).await
    }

    pub async fn submit_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> CodexResult<String> {
        let id = Uuid::now_v7().to_string();
        let sub = Submission {
            id: id.clone(),
            op,
            trace,
        };
        self.submit_with_id(sub).await?;
        Ok(id)
    }

    /// Use sparingly: prefer `submit()` so Codex is responsible for generating
    /// unique IDs for each submission.
    pub async fn submit_with_id(&self, mut sub: Submission) -> CodexResult<()> {
        if sub.trace.is_none() {
            sub.trace = current_span_w3c_trace_context();
        }
        self.tx_sub
            .send(sub)
            .await
            .map_err(|_| CodexErr::InternalAgentDied)?;
        Ok(())
    }

    /// Persist a thread-level memory mode update for the active session.
    ///
    /// This is a local-only operation that updates rollout metadata directly
    /// and does not involve the model.
    pub async fn set_thread_memory_mode(
        &self,
        mode: protocol::protocol::ThreadMemoryMode,
    ) -> anyhow::Result<()> {
        handlers::persist_thread_memory_mode_update(&self.session, mode).await
    }

    pub async fn shutdown_and_wait(&self) -> CodexResult<()> {
        let session_loop_termination = self.session_loop_termination.clone();
        match self.submit(Op::Shutdown).await {
            Ok(_) => {}
            Err(CodexErr::InternalAgentDied) => {}
            Err(err) => return Err(err),
        }
        session_loop_termination.await;
        Ok(())
    }

    pub async fn next_event(&self) -> CodexResult<Event> {
        let event = self
            .rx_event
            .recv()
            .await
            .map_err(|_| CodexErr::InternalAgentDied)?;
        Ok(event)
    }

    pub async fn steer_input(
        &self,
        input: Vec<UserInput>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<String, SteerInputError> {
        self.session
            .steer_input(input, expected_turn_id, responsesapi_client_metadata)
            .await
    }

    pub(crate) async fn set_app_server_client_info(
        &self,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
        mcp_elicitations_auto_deny: bool,
    ) -> ConstraintResult<()> {
        self.session
            .update_settings(SessionSettingsUpdate {
                app_server_client_name,
                app_server_client_version,
                ..Default::default()
            })
            .await?;
        let mcp_connection_manager = self.session.services.mcp_connection_manager.read().await;
        mcp_connection_manager.set_elicitations_auto_deny(mcp_elicitations_auto_deny);
        Ok(())
    }

    pub(crate) async fn agent_status(&self) -> AgentStatus {
        self.agent_status.borrow().clone()
    }

    pub(crate) async fn thread_config_snapshot(&self) -> ThreadConfigSnapshot {
        let state = self.session.state.lock().await;
        state.session_configuration.thread_config_snapshot()
    }

    pub(crate) async fn thread_environment_selections(&self) -> Vec<TurnEnvironmentSelection> {
        let state = self.session.state.lock().await;
        state.session_configuration.environments.clone()
    }

    pub(crate) fn state_db(&self) -> Option<state_db::StateDbHandle> {
        self.session.state_db()
    }

    pub(crate) fn enabled(&self, feature: Feature) -> bool {
        self.session.enabled(feature)
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::SessionCommandHandle for Codex {
    fn submit_op(
        &self,
        op: Op,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.submit(op)
    }

    fn submit_op_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.submit_with_trace(op, trace)
    }

    fn submit_with_id(
        &self,
        submission: Submission,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        Codex::submit_with_id(self, submission)
    }

    fn shutdown(&self) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        self.shutdown_and_wait()
    }

    fn append_conversation_item(
        &self,
        item: ResponseItem,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        async move {
            let submission_id = uuid::Uuid::new_v4().to_string();
            let should_start_turn = self
                .session
                .enqueue_async_input(PendingInputItem::from(item))
                .await;
            if should_start_turn {
                self.session.maybe_start_turn_for_pending_work().await;
            }
            Ok(submission_id)
        }
    }
}

impl thread_service_api::SessionStatusHandle for Codex {
    fn agent_status(
        &self,
    ) -> impl std::future::Future<Output = protocol::protocol::AgentStatus> + Send + '_ {
        Codex::agent_status(self)
    }
}
