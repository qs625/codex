use super::*;
use codex_agent_runtime::ChildCompletionState;
use codex_agent_runtime::GoalRuntimeState;
use codex_api_runtime_api::SharedApiRuntimeFactory;
use codex_auth_types::AuthRuntime;
use codex_auth_types::SharedAuthRuntime;
use codex_code_mode_api::CodeModeRuntimeFactory;
use codex_code_mode_api::CodeModeRuntimeService;
use codex_command_runtime::WaitBackoffState;
use codex_config::ConstraintError;
use codex_config_types::RequirementSource;
use codex_core_plugins_api::SharedPluginRuntime;
use codex_mcp_runtime_api::McpAuthRuntime;
use codex_mcp_runtime_api::McpConnectionRuntimeFactory;
use codex_memories_read_api::SharedMemoryToolDeveloperInstructionsProvider;
use codex_model_provider_api::SharedModelProviderAuthManager;
use codex_protocol::SessionId;
use codex_protocol::ThreadId;
use codex_protocol::protocol::ThreadSkill;
use codex_protocol::protocol::ThreadSource;
use codex_protocol::protocol::TurnEnvironmentSelection;
use crate::SessionPermissionProfileUpdate;
use crate::SessionSettingsApplyCurrent;
use crate::build_session_settings_apply_plan;
use crate::initial_thread_skills;
use crate::merge_thread_skills;
use codex_session_telemetry_api::SessionTelemetryCreateParams;
use codex_session_telemetry_api::SharedSessionTelemetryFactory;
use codex_workflow_api::WorkflowRunController;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::Weak;
use tokio::sync::Semaphore;

/// Context for an initialized model agent
///
/// A session has at most 1 running task at a time, and can be interrupted by user input.
pub struct Session {
    pub(crate) self_weak: OnceLock<Weak<Session>>,
    pub(crate) conversation_id: ThreadId,
    pub(crate) installation_id: String,
    pub(super) tx_event: Sender<Event>,
    pub(super) agent_status: watch::Sender<AgentStatus>,
    pub(super) out_of_band_elicitation_paused: watch::Sender<bool>,
    pub(super) state: Mutex<SessionState>,
    /// Serializes rebuild/apply cycles for the running proxy; each cycle
    /// rebuilds from the current SessionState while holding this lock.
    pub(super) managed_network_proxy_refresh_lock: Semaphore,
    /// The set of enabled features should be invariant for the lifetime of the
    /// session.
    pub(super) features: ManagedFeatures,
    pub(super) pending_mcp_server_refresh_config: Mutex<Option<McpServerRefreshConfig>>,
    pub(crate) conversation: Arc<RealtimeConversationManager>,
    pub(crate) active_turn: Mutex<Option<ActiveTurn>>,
    pub(super) mailbox: Mailbox,
    pub(super) mailbox_rx: Mutex<MailboxReceiver>,
    pub(super) idle_pending_input: Mutex<Vec<crate::PendingInputItem>>,
    pub(crate) goal_runtime: GoalRuntimeState,
    pub(crate) guardian_review_session: GuardianReviewSessionManager,
    pub(crate) workflow_runs: Arc<dyn WorkflowRunController>,
    pub(crate) services: SessionServices,
    pub(super) next_internal_sub_id: AtomicU64,
    pub(super) child_completion: ChildCompletionState,
    pub(super) wait_agent_backoff:
        Mutex<std::collections::HashMap<(ThreadId, ThreadId), WaitBackoffState>>,
}

#[derive(Clone)]
pub(crate) struct SessionConfiguration {
    /// Provider identifier ("openai", "openrouter", ...).
    pub(super) provider: ModelProviderInfo,

    pub(super) collaboration_mode: CollaborationMode,
    pub(super) model_reasoning_summary: Option<ReasoningSummaryConfig>,
    pub(super) service_tier: Option<String>,

    /// Developer instructions that supplement the base instructions.
    pub(super) developer_instructions: Option<String>,

    /// Model instructions that are appended to the base instructions.
    pub(super) user_instructions: Option<String>,

    /// Personality preference for the model.
    pub(super) personality: Option<Personality>,

    /// Base instructions for the session.
    pub(super) base_instructions: String,

    /// Compact prompt override.
    pub(super) compact_prompt: Option<String>,

    /// When to escalate for approval for execution
    pub(super) approval_policy: Constrained<AskForApproval>,
    pub(super) approvals_reviewer: ApprovalsReviewer,
    /// Permission profile state for the session. Keep the constrained profile,
    /// active profile id, and profile-defined workspace roots in sync by using
    /// the methods below instead of mutating the fields independently.
    pub(super) permission_profile_state: PermissionProfileState,
    pub(super) windows_sandbox_level: WindowsSandboxLevel,

    /// Absolute working directory that should be treated as the *root* of the
    /// session. All relative paths supplied by the model as well as the
    /// execution sandbox are resolved against this directory **instead** of
    /// the process-wide current working directory.
    pub(super) cwd: AbsolutePathBuf,
    /// Thread-scoped runtime workspace roots for materializing symbolic
    /// workspace permissions at session runtime.
    pub(super) workspace_roots: Vec<AbsolutePathBuf>,
    /// Directory containing all Codex state for this session.
    pub(super) codex_home: AbsolutePathBuf,
    /// Optional user-facing name for the thread, updated during the session.
    pub(super) thread_name: Option<String>,
    /// Sticky environments for turns that do not provide a turn-local override.
    pub(super) environments: Vec<TurnEnvironmentSelection>,

    // TODO(pakrym): Remove config from here
    pub(super) original_config_do_not_use: Arc<Config>,
    /// Optional service name tag for session metrics.
    pub(super) metrics_service_name: Option<String>,
    /// Terminal identifier resolved by the composition root.
    pub(super) terminal_type: String,
    pub(super) app_server_client_name: Option<String>,
    pub(super) app_server_client_version: Option<String>,
    /// Source of the session (cli, vscode, exec, mcp, ...)
    pub(super) session_source: SessionSource,
    /// Optional analytics source classification for this thread.
    pub(super) thread_source: Option<ThreadSource>,
    pub(super) dynamic_tools: Vec<DynamicToolSpec>,
    pub(super) persist_extended_history: bool,
    pub(super) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(super) user_shell_override: Option<shell::Shell>,
}

impl SessionConfiguration {
    pub(crate) fn codex_home(&self) -> &AbsolutePathBuf {
        &self.codex_home
    }

    pub(super) fn permission_profile_state(&self) -> &PermissionProfileState {
        &self.permission_profile_state
    }

    pub(super) fn permission_profile(&self) -> PermissionProfile {
        self.permission_profile_state
            .permission_profile()
            .clone()
            .materialize_project_roots_with_workspace_roots(&self.workspace_roots)
    }

    pub(super) fn active_permission_profile(&self) -> Option<ActivePermissionProfile> {
        self.permission_profile_state.active_permission_profile()
    }

    pub(super) fn profile_workspace_roots(&self) -> &[AbsolutePathBuf] {
        self.permission_profile_state.profile_workspace_roots()
    }

    #[cfg(test)]
    pub(super) fn set_permission_profile_for_tests(
        &mut self,
        permission_profile: PermissionProfile,
    ) -> ConstraintResult<()> {
        self.permission_profile_state
            .set_legacy_permission_profile(permission_profile)
    }

    pub(super) fn sandbox_policy(&self) -> SandboxPolicy {
        self.permission_profile()
            .to_legacy_sandbox_policy(&self.cwd)
            .unwrap_or_else(|_| {
                let file_system_sandbox_policy = self.file_system_sandbox_policy();
                codex_sandboxing_api::compatibility_sandbox_policy_for_permission_profile(
                    self.permission_profile_state.permission_profile(),
                    &file_system_sandbox_policy,
                    self.network_sandbox_policy(),
                    &self.cwd,
                )
            })
    }

    pub(super) fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy {
        self.permission_profile().file_system_sandbox_policy()
    }

    pub(super) fn network_sandbox_policy(&self) -> NetworkSandboxPolicy {
        self.permission_profile_state
            .permission_profile()
            .network_sandbox_policy()
    }

    pub(super) fn thread_config_snapshot(&self) -> ThreadConfigSnapshot {
        ThreadConfigSnapshot {
            model: self.collaboration_mode.model().to_string(),
            model_provider_id: self.original_config_do_not_use.model_provider_id.clone(),
            service_tier: self.service_tier.clone(),
            approval_policy: self.approval_policy.value(),
            approvals_reviewer: self.approvals_reviewer,
            permission_profile: self.permission_profile(),
            active_permission_profile: self.active_permission_profile(),
            cwd: self.cwd.clone(),
            workspace_roots: self.workspace_roots.clone(),
            profile_workspace_roots: self.profile_workspace_roots().to_vec(),
            ephemeral: self.original_config_do_not_use.ephemeral,
            reasoning_effort: self.collaboration_mode.reasoning_effort(),
            personality: self.personality,
            session_source: self.session_source.clone(),
            thread_source: self.thread_source,
        }
    }

    pub(crate) fn apply(&self, updates: &SessionSettingsUpdate) -> ConstraintResult<Self> {
        let mut next_configuration = self.clone();
        let current_sandbox_policy = self.sandbox_policy();
        let current_file_system_sandbox_policy = self.file_system_sandbox_policy();
        let current_network_sandbox_policy = self.network_sandbox_policy();
        let current_permission_profile = self.permission_profile();

        let absolute_cwd = updates
            .cwd
            .as_ref()
            .map(|cwd| {
                AbsolutePathBuf::relative_to_current_dir(normalize_for_native_workdir(
                    cwd.as_path(),
                ))
                .unwrap_or_else(|e| {
                    warn!("failed to normalize update cwd: {cwd:?}: {e}");
                    self.cwd.clone()
                })
            })
            .unwrap_or_else(|| self.cwd.clone());

        let plan = build_session_settings_apply_plan(
            updates,
            SessionSettingsApplyCurrent {
                collaboration_mode: &self.collaboration_mode,
                service_tier: self.service_tier.clone(),
                personality: self.personality,
                cwd: &self.cwd,
                workspace_roots: &self.workspace_roots,
                permission_profile: &current_permission_profile,
                active_permission_profile: self.active_permission_profile(),
                sandbox_policy: &current_sandbox_policy,
                file_system_sandbox_policy: &current_file_system_sandbox_policy,
                network_sandbox_policy: current_network_sandbox_policy,
                app_server_client_name: self.app_server_client_name.clone(),
                app_server_client_version: self.app_server_client_version.clone(),
            },
            absolute_cwd,
            next_configuration
                .original_config_do_not_use
                .model_options
                .iter()
                .map(|model_option| (model_option.model.as_str(), model_option.provider.as_str())),
        );

        next_configuration.collaboration_mode = plan.collaboration_mode;
        if let Some(model_provider_id) = plan.model_provider_update {
            let Some(model_provider) = next_configuration
                .original_config_do_not_use
                .model_providers
                .get(&model_provider_id)
                .cloned()
            else {
                let allowed = next_configuration
                    .original_config_do_not_use
                    .model_providers
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(ConstraintError::InvalidValue {
                    field_name: "model_provider",
                    candidate: model_provider_id,
                    allowed,
                    requirement_source: RequirementSource::Unknown,
                });
            };
            let mut config = (*next_configuration.original_config_do_not_use).clone();
            config.model_provider_id = model_provider_id;
            config.model_provider = model_provider.clone();
            next_configuration.original_config_do_not_use = Arc::new(config);
            next_configuration.provider = model_provider;
        }
        if let Some(summary) = plan.model_reasoning_summary {
            next_configuration.model_reasoning_summary = Some(summary);
        }
        next_configuration.service_tier = plan.service_tier;
        next_configuration.personality = plan.personality;
        if let Some(approval_policy) = updates.approval_policy {
            next_configuration.approval_policy.set(approval_policy)?;
        }
        if let Some(approvals_reviewer) = updates.approvals_reviewer {
            next_configuration.approvals_reviewer = approvals_reviewer;
        }
        if let Some(windows_sandbox_level) = updates.windows_sandbox_level {
            next_configuration.windows_sandbox_level = windows_sandbox_level;
        }

        next_configuration.cwd = plan.cwd;
        next_configuration.workspace_roots = plan.workspace_roots;
        if let Some(permission_profile_update) = plan.permission_profile_update {
            match permission_profile_update {
                SessionPermissionProfileUpdate::ActiveProfile {
                    permission_profile,
                    active_permission_profile,
                    profile_workspace_roots,
                } => next_configuration
                    .permission_profile_state
                    .set_active_permission_profile(
                        permission_profile,
                        active_permission_profile,
                        profile_workspace_roots,
                    )?,
                SessionPermissionProfileUpdate::LegacyProfile(permission_profile) => {
                    next_configuration
                        .permission_profile_state
                        .set_legacy_permission_profile(permission_profile)?;
                }
            }
        }
        next_configuration.app_server_client_name = plan.app_server_client_name;
        next_configuration.app_server_client_version = plan.app_server_client_version;
        Ok(next_configuration)
    }
}

pub(crate) struct AppServerClientMetadata {
    pub(crate) client_name: Option<String>,
    pub(crate) client_version: Option<String>,
}

impl Session {
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
    #[expect(
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
        models_manager: SharedModelsManager,
        exec_policy: Arc<ExecPolicyManager>,
        exec_policy_loader: Arc<dyn ExecPolicyLoader>,
        tx_event: Sender<Event>,
        agent_status: watch::Sender<AgentStatus>,
        initial_history: InitialHistory,
        session_source: SessionSource,
        skills_manager: codex_core_skills_api::SharedSkillsRuntime,
        plugins_manager: SharedPluginRuntime,
        mcp_manager: Arc<McpManager>,
        mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
        mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
        api_runtime_factory: SharedApiRuntimeFactory,
        session_telemetry_factory: SharedSessionTelemetryFactory,
        memory_tool_developer_instructions_provider: SharedMemoryToolDeveloperInstructionsProvider,
        hook_runtime_factory: SharedHookRuntimeFactory,
        sandbox_runtime: codex_sandboxing_api::SharedSandboxRuntime,
        network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
        extensions: Arc<codex_extension_api::ExtensionRegistry<codex_config::Config>>,
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
        tool_service: Arc<crate::CoreToolServiceApi>,
        workflow_runs: Arc<dyn WorkflowRunController>,
    ) -> anyhow::Result<Arc<Self>> {
        debug!(
            "Configuring session: model={}; provider={:?}",
            session_configuration.collaboration_mode.model(),
            session_configuration.provider
        );
        let forked_from_id = initial_history.forked_from_id();

        let event_persistence_mode = if session_configuration.persist_extended_history {
            ThreadEventPersistenceMode::Extended
        } else {
            ThreadEventPersistenceMode::Limited
        };
        let thread_id = match &initial_history {
            InitialHistory::New | InitialHistory::Cleared | InitialHistory::Forked(_) => {
                ThreadId::default()
            }
            InitialHistory::Resumed(resumed_history) => resumed_history.conversation_id,
        };
        let window_generation = match &initial_history {
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
        // Kick off independent async setup tasks in parallel to reduce startup latency.
        //
        // - initialize thread persistence with new or resumed session info
        // - perform default shell discovery
        // - load history metadata (skipped for subagents)
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
                                    thread_id,
                                    forked_from_id,
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
                                    event_persistence_mode,
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
                                    event_persistence_mode,
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

        let auth_runtime_for_mcp = Arc::clone(&shared_auth_runtime);
        let config_for_mcp = Arc::clone(&config);
        let mcp_manager_for_mcp = Arc::clone(&mcp_manager);
        let mcp_auth_runtime_for_mcp = Arc::clone(&mcp_auth_runtime);
        let auth_and_mcp_fut = async move {
            let auth_snapshot = auth_runtime_for_mcp.auth().await;
            let auth_context = crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
            let mcp_servers = mcp_manager_for_mcp
                .effective_servers(&config_for_mcp, auth_context.as_ref())
                .await;
            let host_owned_codex_apps_enabled = config_for_mcp.features.apps_enabled_for_auth(
                auth_snapshot
                    .as_ref()
                    .is_some_and(|auth| auth.uses_codex_backend()),
            );
            let auth_statuses = mcp_auth_runtime_for_mcp
                .compute_auth_statuses(
                    mcp_servers
                        .iter()
                        .map(|(name, server)| (name.clone(), server.clone()))
                        .collect(),
                    config_for_mcp.mcp_oauth_credentials_store_mode,
                    host_owned_codex_apps_enabled,
                )
                .await;
            (auth_snapshot, mcp_servers, auth_statuses)
        }
        .instrument(info_span!(
            "session_init.auth_mcp",
            otel.name = "session_init.auth_mcp",
        ));

        // Join all independent futures.
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
                .unwrap_or_else(codex_protocol::AgentPath::root);
            let trace_task_name =
                (!trace_agent_path.is_root()).then(|| trace_agent_path.name().to_string());
            let trace_metadata = ThreadStartedTraceMetadata {
                thread_id: thread_id.to_string(),
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
                // Spawned child threads are part of their root rollout tree. If the
                // parent had no trace bundle, do not create an orphan child bundle
                // that looks like an independent rollout.
                parent_rollout_thread_trace.start_child_thread_trace_or_disabled(trace_metadata)
            } else {
                ThreadTraceContext::start_root_or_disabled(trace_metadata)
            };

            let mut post_session_configured_events = Vec::<Event>::new();

            for usage in config.features.legacy_feature_usages() {
                post_session_configured_events.push(Event {
                    id: INITIAL_SUBMIT_ID.to_owned(),
                    msg: EventMsg::DeprecationNotice(DeprecationNoticeEvent {
                        summary: usage.summary.clone(),
                        details: usage.details.clone(),
                    }),
                });
            }
            for message in &config.startup_warnings {
                post_session_configured_events.push(Event {
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
                post_session_configured_events.push(event);
            }
            if config.permissions.approval_policy.value() == AskForApproval::OnFailure {
                post_session_configured_events.push(Event {
                    id: "".to_owned(),
                    msg: EventMsg::Warning(WarningEvent {
                        message: "`on-failure` approval policy is deprecated and will be removed in a future release. Use `on-request` for interactive approvals or `never` for non-interactive runs.".to_string(),
                    }),
                });
            }

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
                    conversation_id: thread_id,
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
                conversation_id: Some(thread_id.to_string()),
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
            // Create the mutable state for the Session.
            let shell_snapshot_tx = if config.features.enabled(Feature::ShellSnapshot) {
                if let Some(snapshot) = session_configuration.inherited_shell_snapshot.clone() {
                    let (tx, rx) = watch::channel(Some(snapshot));
                    default_shell.shell_snapshot = rx;
                    tx
                } else {
                    ShellSnapshot::start_snapshotting(
                        config.codex_home.clone(),
                        thread_id,
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
            let thread_name =
                thread_title_from_thread_store(
                    live_thread_init
                        .as_ref()
                        .map(|live_thread| live_thread.as_ref()),
                    &thread_store,
                    thread_id,
                )
                .instrument(info_span!(
                        "session_init.thread_name_lookup",
                        otel.name = "session_init.thread_name_lookup",
                    ))
                    .await;
            session_configuration.thread_name = thread_name.clone();
            validate_config_lock_if_configured(&session_configuration).await?;
            export_config_lock_if_configured(&session_configuration, thread_id).await?;
            let mut state = SessionState::new(session_configuration.clone());
            state.set_thread_skills(initial_thread_skills(&initial_history));
            let managed_network_requirements_configured = config
                .config_layer_stack
                .requirements_toml()
                .network
                .is_some();
            let managed_network_requirements_enabled = config.managed_network_requirements_enabled();
            let network_approval = Arc::new(NetworkApprovalService::default());
            // The managed proxy can call back into core for allowlist-miss decisions.
            let network_policy_decider_session = if managed_network_requirements_configured {
                config
                    .permissions
                    .network
                    .as_ref()
                    .map(|_| Arc::new(RwLock::new(std::sync::Weak::<Session>::new())))
            } else {
                None
            };
            let blocked_request_observer = if managed_network_requirements_configured {
                config
                    .permissions
                    .network
                    .as_ref()
                    .map(|_| build_blocked_request_observer(Arc::clone(&network_approval)))
            } else {
                None
            };
            let network_policy_decider =
                network_policy_decider_session
                    .as_ref()
                    .map(|network_policy_decider_session| {
                        build_network_policy_decider(
                            Arc::clone(&network_approval),
                            Arc::clone(network_policy_decider_session),
                        )
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
                SessionId::from(thread_id)
            };
            let agent_control = agent_control.with_session_id(session_id);
            let command_service_state = Arc::new(codex_command_service::CommandSessionState::new(
                config.background_terminal_max_timeout,
            ));
            let session_extension_data =
                codex_extension_api::ExtensionData::new(session_id.to_string());
            session_extension_data.insert(command_service_state.manager_handle());
            let thread_extension_data =
                codex_extension_api::ExtensionData::new(thread_id.to_string());
            for contributor in extensions.thread_lifecycle_contributors() {
                contributor.on_thread_start(codex_extension_api::ThreadStartInput {
                    config: config.as_ref(),
                    session_store: &session_extension_data,
                    thread_store: &thread_extension_data,
                });
            }

            let services = SessionServices {
                // Initialize the MCP connection runtime with an uninitialized
                // instance. It will be replaced with a started runtime once all
                // constructor args are available. This also ensures
                // `SessionConfigured` is emitted before any MCP-related events.
                mcp_connection_manager: Arc::new(RwLock::new(
                    mcp_connection_runtime_factory.uninitialized(
                        &config.permissions.approval_policy,
                        config.permissions.permission_profile().clone(),
                    ),
                )),
                mcp_auth_runtime,
                mcp_connection_runtime_factory,
                network_proxy_runtime_factory,
                mcp_startup_cancellation_token: Mutex::new(CancellationToken::new()),
                command_service_state,
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
                model_provider_factory: Arc::clone(&model_provider_factory),
                api_runtime_factory: Arc::clone(&api_runtime_factory),
                session_telemetry_factory: Arc::clone(&session_telemetry_factory),
                memory_tool_developer_instructions_provider,
                sandbox_runtime,
                session_telemetry,
                models_manager: Arc::clone(&models_manager),
                tool_approvals: Mutex::new(ApprovalStore::default()),
                guardian_rejections: Mutex::new(HashMap::new()),
                guardian_rejection_circuit_breaker: Mutex::new(Default::default()),
                runtime_handle: tokio::runtime::Handle::current(),
                skills_manager,
                plugins_manager: Arc::clone(&plugins_manager),
                mcp_manager: Arc::clone(&mcp_manager),
                extensions,
                // TODO(jif): extract session to share between sub-agents
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
                model_client: ModelClient::new(
                    provider_auth_manager,
                    session_id,
                    thread_id,
                    installation_id.clone(),
                    api_runtime_factory,
                    Arc::clone(&model_provider_factory),
                    session_configuration.provider.clone(),
                    session_configuration.session_source.clone(),
                    config.model_verbosity,
                    config
                        .model_options
                        .iter()
                        .filter_map(|model_option| {
                            model_option
                                .max_tokens
                                .map(|max_tokens| (model_option.model.clone(), max_tokens))
                        })
                        .collect(),
                    config.features.enabled(Feature::EnableRequestCompression),
                    config.features.enabled(Feature::RuntimeMetrics),
                    Self::build_model_client_beta_features_header(config.as_ref()),
                    attestation_provider,
                ),
                openai_file_uploader,
                code_mode_service,
                code_mode_runtime_factory,
                tool_service,
                environment_manager,
            };
            services
                .model_client
                .set_window_generation(window_generation);
            let (out_of_band_elicitation_paused, _out_of_band_elicitation_paused_rx) =
                watch::channel(false);

            let (mailbox, mailbox_rx) = Mailbox::new();
            let sess = Arc::new(Session {
                self_weak: OnceLock::new(),
                conversation_id: thread_id,
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
                goal_runtime: GoalRuntimeState::new(),
                guardian_review_session: GuardianReviewSessionManager::default(),
                workflow_runs,
                services,
                next_internal_sub_id: AtomicU64::new(0),
                child_completion: ChildCompletionState::new(),
                wait_agent_backoff: Mutex::new(std::collections::HashMap::new()),
            });
            let _ = sess.self_weak.set(Arc::downgrade(&sess));
            if let Some(network_policy_decider_session) = network_policy_decider_session {
                let mut guard = network_policy_decider_session.write().await;
                *guard = Arc::downgrade(&sess);
            }
            // Dispatch the SessionConfiguredEvent first and then report any errors.
            // If resuming, include converted initial messages in the payload so UIs can render them immediately.
            let initial_messages = initial_history.get_event_msgs();
            let events = std::iter::once(Event {
                id: INITIAL_SUBMIT_ID.to_owned(),
                msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                    session_id,
                    thread_id,
                    forked_from_id,
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

            let mut required_mcp_servers: Vec<String> = mcp_servers
                .iter()
                .filter(|(_, server)| server.enabled() && server.required())
                .map(|(name, _)| name.clone())
                .collect();
            required_mcp_servers.sort();
            let enabled_mcp_server_count =
                mcp_servers.values().filter(|server| server.enabled()).count();
            let required_mcp_server_count = required_mcp_servers.len();
            let tool_plugin_provenance = mcp_manager.tool_plugin_provenance(config.as_ref()).await;
            let codex_apps_auth_context =
                crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
            let host_owned_codex_apps_enabled = config
                .features
                .apps_enabled_for_auth(auth_snapshot.as_ref().is_some_and(|auth| {
                    auth.uses_codex_backend()
                }));
            let client_elicitation_support =
                McpClientElicitationSupport::from_auth_elicitation_enabled(
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
                Some(turn_environment) => crate::mcp::mcp_runtime_environment(
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
                    crate::mcp::mcp_runtime_environment(
                        environment,
                        local_environment,
                        session_configuration.cwd.to_path_buf(),
                    )
                }
            };
            let mcp_connection_runtime_start = sess
                .services
                .mcp_connection_runtime_factory
                .start(McpConnectionRuntimeStartRequest {
                    mcp_servers,
                    store_mode: config.mcp_oauth_credentials_store_mode,
                    auth_entries: auth_statuses,
                    approval_policy: session_configuration.approval_policy.clone(),
                    submit_id: INITIAL_SUBMIT_ID.to_owned(),
                    tx_event: tx_event.clone(),
                    initial_permission_profile: session_configuration.permission_profile().clone(),
                    runtime_environment: mcp_runtime_environment,
                    codex_home: config.codex_home.to_path_buf(),
                    codex_apps_tools_cache_key: codex_apps_tools_cache_key(
                        codex_apps_auth_context.as_ref(),
                    ),
                    host_owned_codex_apps_enabled,
                    client_elicitation_support,
                    tool_plugin_provenance,
                    codex_apps_auth_provider: crate::mcp::codex_apps_auth_provider(
                        auth_snapshot.as_ref(),
                    ),
                    elicitation_reviewer: Some(sess.mcp_elicitation_reviewer()),
                })
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
            sess.schedule_startup_prewarm(session_configuration.base_instructions.clone())
                .await;
            let session_start_source = match &initial_history {
                InitialHistory::Resumed(_) => codex_hooks_api::SessionStartSource::Resume,
                InitialHistory::New | InitialHistory::Forked(_) => {
                    codex_hooks_api::SessionStartSource::Startup
                }
                InitialHistory::Cleared => codex_hooks_api::SessionStartSource::Clear,
            };

            // record_initial_history can emit events. We record only after the SessionConfiguredEvent is emitted.
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

fn emit_feature_metrics(
    features: &codex_features::Features,
    metrics: &dyn codex_session_telemetry_api::SessionTelemetry,
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
