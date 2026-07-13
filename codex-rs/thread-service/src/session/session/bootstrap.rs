use super::bootstrap_support::build_post_session_configured_events;
use super::bootstrap_support::derive_session_init_basics;
use super::bootstrap_support::load_auth_and_mcp;
use super::bootstrap_support::start_session_mcp_runtime;
use super::*;

impl Session {
    pub async fn with_cached_approval<K, F, Fut>(
        &self,
        tool_name: &str,
        keys: Vec<K>,
        fetch: F,
    ) -> protocol::protocol::ReviewDecision
    where
        K: serde::Serialize,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = protocol::protocol::ReviewDecision>,
    {
        approval_support_impl::with_cached_approval(&self.services, tool_name, keys, fetch).await
    }

    pub(crate) async fn merge_thread_skills(
        &self,
        additions: Vec<ThreadSkill>,
    ) -> Option<Vec<ThreadSkill>> {
        let mut state = self.state.lock().await;
        let skills = merge_thread_skills(state.thread_skills(), additions)?;
        state.set_thread_skills(skills.clone());
        Some(skills)
    }

    /// Returns the identity shared by the root thread and all descendant threads.
    pub(crate) fn session_id(&self) -> SessionId {
        self.services.agent_control.session_id()
    }

    #[instrument(name = "session_init", level = "info", skip_all)]
    #[allow(clippy::too_many_arguments)]
    #[allow(
        clippy::await_holding_invalid_type,
        reason = "session initialization must serialize access through session-owned manager guards"
    )]
    pub(crate) async fn new(
        mut session_configuration: SessionConfiguration,
        config: Arc<Config>,
        installation_id: String,
        shared_auth_runtime: SharedAuthRuntime,
        provider_auth_manager: Option<SharedModelProviderAuthManager>,
        model_provider_factory: SharedModelProviderFactory,
        exec_policy: Arc<ExecPolicyManager>,
        exec_policy_loader: Arc<dyn ExecPolicyLoader>,
        tx_event: Sender<Event>,
        agent_status: watch::Sender<AgentStatus>,
        initial_history: InitialHistory,
        session_source: SessionSource,
        skill_service: skill_service_api::SharedSkillServiceApi,
        plugins_manager: SharedPluginRuntime,
        mcp_service: Arc<dyn McpServiceApi>,
        mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
        mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
        api_runtime_factory: SharedApiRuntimeFactory,
        session_telemetry_factory: SharedSessionTelemetryFactory,
        memory_tool_developer_instructions_provider: SharedMemoryToolDeveloperInstructionsProvider,
        model_service: SharedModelServiceApi,
        hook_runtime_factory: SharedHookRuntimeFactory,
        sandbox_runtime: codex_sandboxing_api::SharedSandboxRuntime,
        network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
        command_service_api: Arc<dyn CommandServiceApi>,
        extensions: Arc<codex_extension_api::ExtensionRegistry<config_service::Config>>,
        agent_control: AgentControl,
        environment_manager: Arc<dyn ExecEnvironmentProvider>,
        analytics_events_client: Option<AnalyticsEventsClient>,
        thread_store: Arc<dyn ThreadStore>,
        state_db: Option<StateDbHandle>,
        live_thread_factory: Arc<dyn LiveThreadFactory>,
        parent_rollout_thread_trace: ThreadTraceContext,
        attestation_provider: Option<Arc<dyn AttestationProvider>>,
        active_event_subscriptions: Arc<crate::ActiveEventSubscriptionTracker>,
        openai_file_uploader: SharedOpenAiFileUploader,
        code_mode_service: Arc<dyn CodeModeRuntimeService>,
        code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
        approval_service: Arc<dyn ApprovalServiceApi>,
        goal_service: Arc<dyn GoalServiceApi>,
        tool_service: Arc<crate::ToolServiceApi>,
    ) -> anyhow::Result<Arc<Self>> {
        debug!(
            "Configuring session: model={}; provider={:?}",
            session_configuration.collaboration_mode.model(),
            session_configuration.provider
        );
        let init = derive_session_init_basics(
            &initial_history,
            session_configuration.persist_extended_history,
        );
        let child_completion = child_completion_state_for_initial_history(&initial_history);
        let thread_persistence_fut = async {
            if config.ephemeral {
                Ok::<_, anyhow::Error>(None)
            } else {
                let live_thread = match &initial_history {
                    InitialHistory::New | InitialHistory::Cleared | InitialHistory::Forked(_) => {
                        live_thread_factory
                            .create(
                                Arc::clone(&thread_store),
                                CreateThreadParams {
                                    thread_id: init.thread_id,
                                    forked_from_id: init.forked_from_id,
                                    source: session_source,
                                    thread_source: session_configuration.thread_source,
                                    base_instructions: BaseInstructions {
                                        text: session_configuration.base_instructions.clone(),
                                    },
                                    dynamic_tools: session_configuration.dynamic_tools.clone(),
                                    metadata: ThreadPersistenceMetadata {
                                        cwd: Some(config.cwd.to_path_buf()),
                                        model_provider: config.model_provider_id.clone(),
                                        memory_mode: if config.memories.generate_memories {
                                            ThreadMemoryMode::Enabled
                                        } else {
                                            ThreadMemoryMode::Disabled
                                        },
                                    },
                                    event_persistence_mode: init.event_persistence_mode,
                                },
                            )
                            .await?
                    }
                    InitialHistory::Resumed(resumed_history) => {
                        live_thread_factory
                            .resume(
                                Arc::clone(&thread_store),
                                ResumeThreadParams {
                                    thread_id: resumed_history.conversation_id,
                                    rollout_path: resumed_history.rollout_path.clone(),
                                    history: Some(resumed_history.history.clone()),
                                    include_archived: true,
                                    metadata: ThreadPersistenceMetadata {
                                        cwd: Some(config.cwd.to_path_buf()),
                                        model_provider: config.model_provider_id.clone(),
                                        memory_mode: if config.memories.generate_memories {
                                            ThreadMemoryMode::Enabled
                                        } else {
                                            ThreadMemoryMode::Disabled
                                        },
                                    },
                                    event_persistence_mode: init.event_persistence_mode,
                                },
                            )
                            .await?
                    }
                };
                Ok(Some(live_thread))
            }
        }
        .instrument(info_span!(
            "session_init.thread_persistence",
            otel.name = "session_init.thread_persistence",
            session_init.ephemeral = config.ephemeral,
        ));
        let state_db_fut =
            async { if config.ephemeral { None } else { state_db } }.instrument(info_span!(
                "session_init.state_db",
                otel.name = "session_init.state_db",
                session_init.ephemeral = config.ephemeral,
            ));

        let auth_and_mcp_fut = load_auth_and_mcp(
            Arc::clone(&shared_auth_runtime),
            Arc::clone(&config),
            Arc::clone(&mcp_service),
            Arc::clone(&plugins_manager),
            Arc::clone(&mcp_auth_runtime),
        )
        .instrument(info_span!(
            "session_init.auth_mcp",
            otel.name = "session_init.auth_mcp",
        ));

        let (thread_persistence_result, state_db_ctx, (auth_snapshot, mcp_servers, auth_statuses)) =
            tokio::join!(thread_persistence_fut, state_db_fut, auth_and_mcp_fut);

        let mut live_thread_init =
            LiveThreadInitGuard::new(thread_persistence_result.map_err(|e| {
                error!("failed to initialize thread persistence: {e:#}");
                e
            })?);
        let session_result: anyhow::Result<Arc<Self>> = async {
            let rollout_path = if let Some(live_thread) = live_thread_init.as_ref() {
                live_thread.local_rollout_path().await?
            } else {
                None
            };
            let trace_agent_path = session_configuration
                .session_source
                .get_agent_path()
                .unwrap_or_else(protocol::AgentPath::root);
            let trace_task_name =
                (!trace_agent_path.is_root()).then_some(trace_agent_path.name().to_string());
            let trace_metadata = ThreadStartedTraceMetadata {
                thread_id: init.thread_id.to_string(),
                agent_path: trace_agent_path.to_string(),
                task_name: trace_task_name,
                nickname: session_configuration.session_source.get_nickname(),
                agent_role: session_configuration.session_source.get_agent_role(),
                session_source: session_configuration.session_source.clone(),
                cwd: session_configuration.cwd.to_path_buf(),
                rollout_path: rollout_path.clone(),
                model: session_configuration.collaboration_mode.model().to_string(),
                provider_name: config.model_provider_id.clone(),
                approval_policy: session_configuration.approval_policy.value().to_string(),
                sandbox_policy: format!("{:?}", session_configuration.sandbox_policy()),
            };
            let rollout_thread_trace = if matches!(
                session_configuration.session_source,
                SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. })
            ) {
                parent_rollout_thread_trace.start_child_thread_trace_or_disabled(trace_metadata)
            } else {
                ThreadTraceContext::start_root_or_disabled(trace_metadata)
            };

            let mut post_session_configured_events = build_post_session_configured_events(&config);

            let auth_runtime: &dyn AuthRuntime = shared_auth_runtime.as_ref();
            let auth_telemetry = auth_runtime.telemetry_snapshot();
            let auth_mode = auth_telemetry.auth_mode.map(TelemetryAuthMode::from);
            let account_id = auth_telemetry.account_id;
            let account_email = auth_telemetry.account_email;
            let originator = originator().value;
            let terminal_type = session_configuration.terminal_type.clone();
            let session_model = session_configuration.collaboration_mode.model().to_string();
            let auth_env_telemetry = collect_auth_env_telemetry(AuthEnvTelemetryInput {
                provider_env_key: session_configuration.provider.env_key.as_deref(),
                codex_api_key_env_enabled: auth_runtime.codex_api_key_env_enabled(),
            });
            let session_telemetry = session_telemetry_factory.create(
                SessionTelemetryCreateParams {
                    conversation_id: init.thread_id,
                    model: session_model.clone(),
                    slug: session_model.clone(),
                    account_id: account_id.clone(),
                    account_email: account_email.clone(),
                    auth_mode,
                    auth_env: auth_env_telemetry.to_otel_metadata(),
                    originator: originator.clone(),
                    log_user_prompts: config.otel.log_user_prompt,
                    terminal_type: terminal_type.clone(),
                    session_source: session_configuration.session_source.clone(),
                    metrics_service_name: session_configuration.metrics_service_name.clone(),
                },
            );
            let network_proxy_audit_metadata = NetworkProxyAuditMetadata {
                conversation_id: Some(init.thread_id.to_string()),
                app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                user_account_id: account_id,
                auth_mode: auth_mode.map(|mode| mode.to_string()),
                originator: Some(originator),
                user_email: account_email,
                terminal_type: Some(terminal_type),
                model: Some(session_model.clone()),
                slug: Some(session_model),
            };
            emit_feature_metrics(&config.features, session_telemetry.as_ref());
            session_telemetry.counter(
                THREAD_STARTED_METRIC,
                /*inc*/ 1,
                &[(
                    "is_git",
                    if get_git_repo_root(&session_configuration.cwd).is_some() {
                        "true"
                    } else {
                        "false"
                    },
                )],
            );

            let mcp_server_names: Vec<&str> = mcp_servers.keys().map(String::as_str).collect();
            session_telemetry.conversation_starts(
                config.model_provider.name.as_str(),
                session_configuration.collaboration_mode.reasoning_effort(),
                config
                    .model_reasoning_summary
                    .unwrap_or(ReasoningSummaryConfig::Auto),
                config.model_context_window,
                config.model_auto_compact_token_limit,
                config.permissions.approval_policy.value(),
                config
                    .permissions
                    .legacy_sandbox_policy(session_configuration.cwd.as_path()),
                &mcp_server_names,
                config.active_profile.as_deref(),
            );

            let use_zsh_fork_shell = config.features.enabled(Feature::ShellZshFork);
            let mut default_shell = if let Some(user_shell_override) =
                session_configuration.user_shell_override.clone()
            {
                user_shell_override
            } else if use_zsh_fork_shell {
                let zsh_path = config.zsh_path.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "zsh fork feature enabled, but `zsh_path` is not configured; set `zsh_path` in config.toml"
                    )
                })?;
                let zsh_path = zsh_path.to_path_buf();
                shell::get_shell(shell::ShellType::Zsh, Some(&zsh_path)).ok_or_else(|| {
                    anyhow::anyhow!(
                        "zsh fork feature enabled, but zsh_path `{}` is not usable; set `zsh_path` to a valid zsh executable",
                        zsh_path.display()
                    )
                })?
            } else {
                shell::default_user_shell()
            };
            let shell_snapshot_tx = if config.features.enabled(Feature::ShellSnapshot) {
                if let Some(snapshot) = session_configuration.inherited_shell_snapshot.clone() {
                    let (tx, rx) = watch::channel(Some(snapshot));
                    default_shell.shell_snapshot = rx;
                    tx
                } else {
                    ShellSnapshot::start_snapshotting(
                        config.codex_home.clone(),
                        init.thread_id,
                        session_configuration.cwd.clone(),
                        &mut default_shell,
                        session_telemetry.clone(),
                        state_db_ctx.clone(),
                    )
                }
            } else {
                let (tx, rx) = watch::channel(None);
                default_shell.shell_snapshot = rx;
                tx
            };
            let thread_name = thread_title_from_thread_store(
                live_thread_init
                    .as_ref()
                    .map(std::convert::AsRef::as_ref),
                &thread_store,
                init.thread_id,
            )
            .instrument(info_span!(
                "session_init.thread_name_lookup",
                otel.name = "session_init.thread_name_lookup",
            ))
            .await;
            session_configuration.thread_name = thread_name.clone();
            validate_config_lock_if_configured(&session_configuration).await?;
            export_config_lock_if_configured(&session_configuration, init.thread_id).await?;
            let mut state = SessionState::new(session_configuration.clone());
            state.set_thread_skills(initial_thread_skills(&initial_history));
            let managed_network_requirements_configured = config
                .config_layer_stack
                .requirements_toml()
                .network
                .is_some();
            let managed_network_requirements_enabled = config.managed_network_requirements_enabled();
            let network_approval = approval_service.create_session_network_approval();
            let network_policy_decider_session = if managed_network_requirements_configured {
                config.permissions.network.as_ref().map(|_| {
                    Arc::new(RwLock::new(None::<std::sync::Weak<dyn ApprovalSessionCapability>>))
                })
            } else {
                None
            };
            let blocked_request_observer = if managed_network_requirements_configured {
                config
                    .permissions
                    .network
                    .as_ref()
                    .map(|_| Arc::clone(&network_approval).build_blocked_request_observer())
            } else {
                None
            };
            let network_policy_decider =
                network_policy_decider_session
                    .as_ref()
                    .map(|network_policy_decider_session| {
                        Arc::clone(&network_approval).build_network_policy_decider(Arc::clone(
                            network_policy_decider_session,
                        ))
                    });
            let (network_proxy, session_network_proxy) =
                if let Some(spec) = config.permissions.network.as_ref() {
                    let current_exec_policy = exec_policy.current();
                    let (network_proxy, session_network_proxy) = Self::start_managed_network_proxy(
                        spec,
                        network_proxy_runtime_factory.as_ref(),
                        current_exec_policy.as_ref(),
                        config.permissions.permission_profile(),
                        network_policy_decider.as_ref().map(Arc::clone),
                        blocked_request_observer.as_ref().map(Arc::clone),
                        managed_network_requirements_configured,
                        network_proxy_audit_metadata,
                    )
                    .instrument(info_span!(
                        "session_init.network_proxy",
                        otel.name = "session_init.network_proxy",
                        session_init.managed_network_requirements_enabled =
                            managed_network_requirements_enabled,
                    ))
                    .await?;
                    (Some(network_proxy), Some(session_network_proxy))
                } else {
                    (None, None)
                };

            let hooks = build_hooks_for_config(
                &config,
                plugins_manager.as_ref(),
                &default_shell,
                hook_runtime_factory.as_ref(),
            )
            .await;
            for warning in hooks.startup_warnings() {
                post_session_configured_events.push(Event {
                    id: INITIAL_SUBMIT_ID.to_owned(),
                    msg: EventMsg::Warning(WarningEvent {
                        message: warning.clone(),
                    }),
                });
            }

            let analytics_events_client =
                analytics_events_client.unwrap_or_else(AnalyticsEventsClient::disabled);
            let session_id = if session_configuration.session_source.is_non_root_agent() {
                agent_control.session_id()
            } else {
                SessionId::from(init.thread_id)
            };
            let model_client_api = model_service
                .create_client(CreateModelClientRequest {
                    selection: ModelSelectionPolicy {
                        requested_model: Some(
                            session_configuration
                                .collaboration_mode
                                .model()
                                .to_string(),
                        ),
                        provider_hint: Some(config.model_provider_id.clone()),
                        allow_default_fallback: true,
                        refresh: ModelCatalogRefresh::OnlineIfUncached,
                    },
                    installation_id: installation_id.clone(),
                    session_id,
                    thread_id: init.thread_id,
                    session_source: session_configuration.session_source.clone(),
                    reasoning_effort: session_configuration
                        .collaboration_mode
                        .reasoning_effort(),
                    service_tier: crate::session::turn::model_service_tier(
                        session_configuration.service_tier.as_deref(),
                    ),
                    verbosity: config.model_verbosity,
                    chat_completions_max_tokens_by_model: config
                        .model_options
                        .iter()
                        .filter_map(|model_option| {
                            model_option
                                .max_tokens
                                .map(|max_tokens| (model_option.model.clone(), max_tokens))
                        })
                        .collect(),
                    enable_request_compression: config
                        .features
                        .enabled(Feature::EnableRequestCompression),
                    include_timing_metrics: config.features.enabled(Feature::RuntimeMetrics),
                    beta_features_header: Self::build_model_client_beta_features_header(
                        config.as_ref(),
                    ),
                })
                .await
                .map_err(|err| anyhow::anyhow!("failed to create model client api: {err}"))?;
            let agent_control = agent_control.with_session_id(session_id);
            let command_service_state = Arc::new(command_service::CommandSessionState::new(
                config.background_terminal_max_timeout,
            ));
            let session_extension_data =
                codex_extension_api::ExtensionData::new(session_id.to_string());
            session_extension_data.insert(command_service_state.manager_handle());
            let thread_extension_data =
                codex_extension_api::ExtensionData::new(init.thread_id.to_string());
            for contributor in extensions.thread_lifecycle_contributors() {
                contributor.on_thread_start(codex_extension_api::ThreadStartInput {
                    config: config.as_ref(),
                    session_store: &session_extension_data,
                    thread_store: &thread_extension_data,
                });
            }

            let services = SessionServices {
                mcp_connection_manager: Arc::new(RwLock::new(
                    mcp_connection_runtime_factory.uninitialized(
                        &config.permissions.approval_policy,
                        config.permissions.permission_profile().clone(),
                    ),
                )),
                mcp_service,
                mcp_auth_runtime,
                mcp_connection_runtime_factory,
                network_proxy_runtime_factory,
                mcp_startup_cancellation_token: Mutex::new(CancellationToken::new()),
                command_service_state,
                command_service_api,
                shell_zsh_path: config.zsh_path.clone(),
                main_execve_wrapper_exe: config.main_execve_wrapper_exe.clone(),
                analytics_events_client,
                hooks: std::sync::RwLock::new(hooks),
                hook_runtime_factory,
                rollout_thread_trace,
                user_shell: Arc::new(default_shell),
                shell_snapshot_tx,
                show_raw_agent_reasoning: config.show_raw_agent_reasoning,
                exec_policy,
                exec_policy_loader,
                auth_runtime: Arc::clone(&shared_auth_runtime),
                provider_auth_manager: provider_auth_manager.clone(),
                model_provider_factory: Arc::clone(&model_provider_factory),
                api_runtime_factory: Arc::clone(&api_runtime_factory),
                session_telemetry_factory: Arc::clone(&session_telemetry_factory),
                memory_tool_developer_instructions_provider,
                model_service,
                sandbox_runtime,
                session_telemetry,
                tool_approvals: Mutex::new(ApprovalStore::default()),
                guardian_rejections: Mutex::new(HashMap::new()),
                guardian_rejection_circuit_breaker: Mutex::new(Default::default()),
                runtime_handle: tokio::runtime::Handle::current(),
                skill_service,
                plugins_manager: Arc::clone(&plugins_manager),
                extensions,
                session_extension_data,
                thread_extension_data,
                agent_control,
                network_proxy,
                network_approval: Arc::clone(&network_approval),
                state_db: state_db_ctx.clone(),
                live_thread: live_thread_init.as_ref().cloned(),
                thread_store: Arc::clone(&thread_store),
                live_thread_factory,
                attestation_provider: attestation_provider.clone(),
                active_event_subscriptions,
                model_client_api,
                openai_file_uploader,
                code_mode_service,
                code_mode_runtime_factory,
                approval_service,
                goal_service,
                tool_service,
                environment_manager,
            };
            services
                .model_client_api
                .set_window_generation(init.window_generation);
            let (out_of_band_elicitation_paused, _out_of_band_elicitation_paused_rx) =
                watch::channel(false);

            let (mailbox, mailbox_rx) = Mailbox::new();
            let (thread_wait_events, _thread_wait_events_rx) =
                watch::channel(ThreadWaitEventSnapshot::default());
            let sess = Arc::new(Session {
                self_weak: OnceLock::new(),
                conversation_id: init.thread_id,
                installation_id,
                tx_event: tx_event.clone(),
                agent_status,
                out_of_band_elicitation_paused,
                state: Mutex::new(state),
                managed_network_proxy_refresh_lock: Semaphore::new(/*permits*/ 1),
                features: config.features.clone(),
                pending_mcp_server_refresh_config: Mutex::new(None),
                conversation: Arc::new(RealtimeConversationManager::new()),
                active_turn: Mutex::new(None),
                mailbox,
                mailbox_rx: Mutex::new(mailbox_rx),
                idle_pending_input: Mutex::new(Vec::new()),
                scheduler: Mutex::new(()),
                #[cfg(test)]
                goal_continuation_before_launch_hook: Mutex::new(None),
                goal_runtime: GoalRuntimeState::new(),
                guardian_review_session: GuardianReviewSessionManager::default(),
                services,
                next_internal_sub_id: AtomicU64::new(0),
                child_completion,
                thread_wait_events,
                thread_wait_backoff: Mutex::new(ThreadWaitBackoffState::default()),
            });
            let _ = sess.self_weak.set(Arc::downgrade(&sess));
            if let Some(network_policy_decider_session) = network_policy_decider_session {
                let mut guard = network_policy_decider_session.write().await;
                let session_capability: Arc<dyn ApprovalSessionCapability> = sess.clone();
                *guard = Some(Arc::downgrade(&session_capability));
            }
            let initial_messages = initial_history.get_event_msgs();
            let events = std::iter::once(Event {
                id: INITIAL_SUBMIT_ID.to_owned(),
                msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                    session_id,
                    thread_id: init.thread_id,
                    forked_from_id: init.forked_from_id,
                    thread_source: session_configuration.thread_source,
                    thread_name: session_configuration.thread_name.clone(),
                    model: session_configuration.collaboration_mode.model().to_string(),
                    model_provider_id: config.model_provider_id.clone(),
                    service_tier: session_configuration.service_tier.clone(),
                    approval_policy: session_configuration.approval_policy.value(),
                    approvals_reviewer: session_configuration.approvals_reviewer,
                    permission_profile: session_configuration.permission_profile(),
                    active_permission_profile: session_configuration.active_permission_profile(),
                    cwd: session_configuration.cwd.clone(),
                    reasoning_effort: session_configuration.collaboration_mode.reasoning_effort(),
                    initial_messages,
                    network_proxy: session_network_proxy.filter(|_| {
                        Self::managed_network_proxy_active_for_permission_profile(
                            session_configuration
                                .permission_profile_state()
                                .permission_profile(),
                        )
                    }),
                    rollout_path,
                }),
            })
            .chain(post_session_configured_events.into_iter());
            for event in events {
                sess.send_event_raw(event).await;
            }

            start_session_mcp_runtime(
                &sess,
                &config,
                &session_configuration,
                auth_snapshot,
                mcp_servers,
                auth_statuses,
                tx_event.clone(),
            )
            .await?;
            sess.schedule_startup_prewarm(session_configuration.base_instructions.clone())
                .await;
            let session_start_source = match &initial_history {
                InitialHistory::Resumed(_) => hooks_api::SessionStartSource::Resume,
                InitialHistory::New | InitialHistory::Forked(_) => {
                    hooks_api::SessionStartSource::Startup
                }
                InitialHistory::Cleared => hooks_api::SessionStartSource::Clear,
            };

            sess.record_initial_history(initial_history).await;
            {
                let mut state = sess.state.lock().await;
                state.set_pending_session_start_source(Some(session_start_source));
            }

            Ok(sess)
        }
        .await;
        match session_result {
            Ok(sess) => {
                live_thread_init.commit();
                Ok(sess)
            }
            Err(err) => {
                live_thread_init.discard().await;
                Err(err)
            }
        }
    }
}

fn child_completion_state_for_initial_history(
    initial_history: &InitialHistory,
) -> ChildCompletionState {
    let InitialHistory::Resumed(resumed_history) = initial_history else {
        return ChildCompletionState::new();
    };
    let final_status = resumed_history
        .history
        .iter()
        .filter_map(|item| match item {
            protocol::protocol::RolloutItem::EventMsg(event) => {
                codex_agent_runtime::agent_status_from_event(event)
            }
            _ => None,
        })
        .next_back()
        .is_some_and(|status| codex_agent_runtime::is_final(&status));
    if final_status {
        ChildCompletionState::inactive()
    } else {
        ChildCompletionState::new()
    }
}

fn emit_feature_metrics(
    features: &codex_features::Features,
    metrics: &dyn session_telemetry_api::SessionTelemetry,
) {
    for feature in FEATURES {
        if matches!(feature.stage, codex_features::Stage::Removed) {
            continue;
        }
        if features.enabled(feature.id) != feature.default_enabled {
            metrics.counter(
                "codex.feature.state",
                /*inc*/ 1,
                &[
                    ("feature", feature.key),
                    ("value", &features.enabled(feature.id).to_string()),
                ],
            );
        }
    }
}
