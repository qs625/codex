use super::*;
use crate::live_thread_runtime::AppServerLiveThreadCommandRuntime;
use crate::live_thread_runtime::AppServerLiveThreadInspectionRuntime;
use crate::live_thread_runtime::AppServerLiveThreadListenerRuntime;
use crate::live_thread_runtime::AppServerLiveThreadSkillWatchRuntime;
use crate::live_thread_runtime::AppServerLiveThreadStatusRuntime;
use crate::live_thread_runtime::AppServerLiveThreadTurnRuntime;
use crate::memory_service_wiring::MemoryServiceHost;
use crate::request_processors::thread_processor::thread_processor_new_thread;
use futures::future::BoxFuture;
use model_service_api::SharedModelServiceApi;
use thread_service_api::AppServerClientInfo;
use thread_store_api::ReadThreadParams;

pub(crate) trait TurnProcessorRuntime: Send + Sync {
    fn validate_environment_selections(
        &self,
        environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<()>;

    fn live_thread_config<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> BoxFuture<'a, CodexResult<Arc<Config>>>;

    fn inject_thread_conversation_items<'a>(
        &'a self,
        thread_id: ThreadId,
        items: Vec<ResponseItem>,
    ) -> BoxFuture<'a, CodexResult<()>>;

    fn steer_thread_input<'a>(
        &'a self,
        thread_id: ThreadId,
        input: Vec<CoreInputItem>,
        expected_turn_id: Option<String>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> BoxFuture<'a, CodexResult<Result<String, SteerInputError>>>;

    fn fork_detached_review_thread<'a>(
        &'a self,
        parent_thread_id: ThreadId,
        config: Config,
        trace: Option<protocol::protocol::W3cTraceContext>,
    ) -> BoxFuture<'a, CodexResult<DetachedReviewThread>>;

    fn read_detached_review_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> BoxFuture<'a, CodexResult<StoredThread>>;
}

pub(crate) struct DetachedReviewThread {
    thread_id: ThreadId,
}

impl TurnProcessorRuntime for ThreadService {
    fn validate_environment_selections(
        &self,
        environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<()> {
        ThreadService::validate_environment_selections(self, environments)
    }

    fn live_thread_config<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> BoxFuture<'a, CodexResult<Arc<Config>>> {
        Box::pin(ThreadService::live_thread_config(self, thread_id))
    }

    fn inject_thread_conversation_items<'a>(
        &'a self,
        thread_id: ThreadId,
        items: Vec<ResponseItem>,
    ) -> BoxFuture<'a, CodexResult<()>> {
        Box::pin(ThreadService::inject_thread_conversation_items(
            self, thread_id, items,
        ))
    }

    fn steer_thread_input<'a>(
        &'a self,
        thread_id: ThreadId,
        input: Vec<CoreInputItem>,
        expected_turn_id: Option<String>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> BoxFuture<'a, CodexResult<Result<String, SteerInputError>>> {
        Box::pin(async move {
            ThreadService::steer_thread_input(
                self,
                thread_id,
                input,
                expected_turn_id.as_deref(),
                responsesapi_client_metadata,
            )
            .await
        })
    }

    fn fork_detached_review_thread<'a>(
        &'a self,
        parent_thread_id: ThreadId,
        config: Config,
        trace: Option<protocol::protocol::W3cTraceContext>,
    ) -> BoxFuture<'a, CodexResult<DetachedReviewThread>> {
        Box::pin(async move {
            let new_thread = ThreadService::fork_live_thread_from_current_history(
                self,
                parent_thread_id,
                ForkSnapshot::Interrupted,
                config,
                /*thread_source*/ None,
                /*persist_extended_history*/ false,
                trace,
            )
            .await?;
            let new_thread = thread_processor_new_thread(new_thread);
            let thread_id = new_thread.thread_id;
            Ok(DetachedReviewThread { thread_id })
        })
    }

    fn read_detached_review_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> BoxFuture<'a, CodexResult<StoredThread>> {
        Box::pin(async move {
            ThreadService::read_thread(
                self,
                ReadThreadParams {
                    thread_id,
                    include_archived: true,
                    include_history: false,
                },
            )
            .await
        })
    }
}

#[derive(Clone)]
pub(crate) struct TurnRequestProcessor {
    auth_manager: Arc<AuthManager>,
    turn_runtime: Arc<dyn TurnProcessorRuntime>,
    live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime>,
    live_thread_status: Arc<dyn AppServerLiveThreadStatusRuntime>,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    live_thread_turn: Arc<dyn AppServerLiveThreadTurnRuntime>,
    live_thread_skill_watch: Arc<dyn AppServerLiveThreadSkillWatchRuntime>,
    live_thread_listener: Arc<dyn AppServerLiveThreadListenerRuntime>,
    memory_startup_host: Arc<dyn MemoryServiceHost>,
    model_service: SharedModelServiceApi,
    outgoing: Arc<OutgoingMessageSender>,
    analytics_events_client: AnalyticsEventsClient,
    arg0_paths: Arg0DispatchPaths,
    config: Arc<Config>,
    config_manager: ConfigManager,
    pending_thread_unloads: Arc<Mutex<HashSet<ThreadId>>>,
    thread_state_manager: ThreadStateManager,
    thread_watch_manager: ThreadWatchManager,
    thread_list_state_permit: Arc<Semaphore>,
    state_db: Option<StateDbHandle>,
    skills_watcher: Arc<SkillsWatcher>,
}

fn resolve_runtime_workspace_roots(
    workspace_roots: Vec<PathBuf>,
    base_cwd: &AbsolutePathBuf,
) -> Vec<AbsolutePathBuf> {
    let mut resolved_roots = Vec::new();
    for path in workspace_roots {
        let root = AbsolutePathBuf::resolve_path_against_base(path, base_cwd.as_path());
        if !resolved_roots.iter().any(|existing| existing == &root) {
            resolved_roots.push(root);
        }
    }
    resolved_roots
}

impl TurnRequestProcessor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        auth_manager: Arc<AuthManager>,
        thread_service: Arc<ThreadService>,
        model_service: SharedModelServiceApi,
        outgoing: Arc<OutgoingMessageSender>,
        analytics_events_client: AnalyticsEventsClient,
        arg0_paths: Arg0DispatchPaths,
        config: Arc<Config>,
        config_manager: ConfigManager,
        pending_thread_unloads: Arc<Mutex<HashSet<ThreadId>>>,
        thread_state_manager: ThreadStateManager,
        thread_watch_manager: ThreadWatchManager,
        thread_list_state_permit: Arc<Semaphore>,
        state_db: Option<StateDbHandle>,
        skills_watcher: Arc<SkillsWatcher>,
    ) -> Self {
        Self {
            auth_manager,
            turn_runtime: thread_service.clone(),
            live_thread_inspection: thread_service.clone(),
            live_thread_status: thread_service.clone(),
            live_thread_command: thread_service.clone(),
            live_thread_turn: thread_service.clone(),
            live_thread_skill_watch: thread_service.clone(),
            live_thread_listener: thread_service.clone(),
            memory_startup_host: thread_service,
            model_service,
            outgoing,
            analytics_events_client,
            arg0_paths,
            config,
            config_manager,
            pending_thread_unloads,
            thread_state_manager,
            thread_watch_manager,
            thread_list_state_permit,
            state_db,
            skills_watcher,
        }
    }

    pub(crate) async fn turn_start(
        &self,
        request_id: ConnectionRequestId,
        params: TurnStartParams,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.turn_start_inner(
            request_id,
            params,
            app_server_client_name,
            app_server_client_version,
        )
        .await
        .map(|response| Some(response.into()))
    }

    pub(crate) async fn thread_inject_items(
        &self,
        params: ThreadInjectItemsParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_inject_items_response_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn turn_steer(
        &self,
        request_id: &ConnectionRequestId,
        params: TurnSteerParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.turn_steer_inner(request_id, params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn turn_interrupt(
        &self,
        request_id: &ConnectionRequestId,
        params: TurnInterruptParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.turn_interrupt_inner(request_id, params)
            .await
            .map(|response| response.map(Into::into))
    }

    pub(crate) async fn thread_realtime_start(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRealtimeStartParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_realtime_start_inner(request_id, params)
            .await
            .map(|response| response.map(Into::into))
    }

    pub(crate) async fn thread_realtime_append_audio(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRealtimeAppendAudioParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_realtime_append_audio_inner(request_id, params)
            .await
            .map(|response| response.map(Into::into))
    }

    pub(crate) async fn thread_realtime_append_text(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRealtimeAppendTextParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_realtime_append_text_inner(request_id, params)
            .await
            .map(|response| response.map(Into::into))
    }

    pub(crate) async fn thread_realtime_stop(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRealtimeStopParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_realtime_stop_inner(request_id, params)
            .await
            .map(|response| response.map(Into::into))
    }

    pub(crate) async fn thread_realtime_list_voices(
        &self,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        Ok(Some(
            ThreadRealtimeListVoicesResponse {
                voices: RealtimeVoicesList::builtin(),
            }
            .into(),
        ))
    }

    pub(crate) async fn review_start(
        &self,
        request_id: &ConnectionRequestId,
        params: ReviewStartParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.review_start_inner(request_id, params)
            .await
            .map(|()| None)
    }

    fn track_error_response(
        &self,
        request_id: &ConnectionRequestId,
        error: &JSONRPCErrorError,
        error_type: Option<AnalyticsJsonRpcError>,
    ) {
        self.analytics_events_client.track_error_response(
            request_id.connection_id.0,
            request_id.request_id.clone(),
            error.clone(),
            error_type,
        );
    }

    fn normalize_turn_start_collaboration_mode(
        &self,
        mut collaboration_mode: CollaborationMode,
    ) -> CollaborationMode {
        if collaboration_mode.settings.developer_instructions.is_none()
            && let Some(instructions) = builtin_collaboration_mode_presets()
                .into_iter()
                .find(|preset| preset.mode == Some(collaboration_mode.mode))
                .and_then(|preset| preset.developer_instructions.flatten())
                .filter(|instructions| !instructions.is_empty())
        {
            collaboration_mode.settings.developer_instructions = Some(instructions);
        }

        collaboration_mode
    }

    fn review_request_from_target(
        target: ApiReviewTarget,
    ) -> Result<(ReviewRequest, String), JSONRPCErrorError> {
        let cleaned_target = match target {
            ApiReviewTarget::UncommittedChanges => ApiReviewTarget::UncommittedChanges,
            ApiReviewTarget::BaseBranch { branch } => {
                let branch = branch.trim().to_string();
                if branch.is_empty() {
                    return Err(invalid_request("branch must not be empty".to_string()));
                }
                ApiReviewTarget::BaseBranch { branch }
            }
            ApiReviewTarget::Commit { sha, title } => {
                let sha = sha.trim().to_string();
                if sha.is_empty() {
                    return Err(invalid_request("sha must not be empty".to_string()));
                }
                let title = title
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty());
                ApiReviewTarget::Commit { sha, title }
            }
            ApiReviewTarget::Custom { instructions } => {
                let trimmed = instructions.trim().to_string();
                if trimmed.is_empty() {
                    return Err(invalid_request(
                        "instructions must not be empty".to_string(),
                    ));
                }
                ApiReviewTarget::Custom {
                    instructions: trimmed,
                }
            }
        };

        let core_target = match cleaned_target {
            ApiReviewTarget::UncommittedChanges => CoreReviewTarget::UncommittedChanges,
            ApiReviewTarget::BaseBranch { branch } => CoreReviewTarget::BaseBranch { branch },
            ApiReviewTarget::Commit { sha, title } => CoreReviewTarget::Commit { sha, title },
            ApiReviewTarget::Custom { instructions } => CoreReviewTarget::Custom { instructions },
        };

        let hint = thread_service::review_prompts::user_facing_hint(&core_target);
        let review_request = ReviewRequest {
            target: core_target,
            user_facing_hint: Some(hint.clone()),
        };

        Ok((review_request, hint))
    }

    fn parse_environment_selections(
        &self,
        environments: Option<Vec<TurnEnvironmentParams>>,
    ) -> Result<Option<Vec<TurnEnvironmentSelection>>, JSONRPCErrorError> {
        let environment_selections = environments.map(|environments| {
            environments
                .into_iter()
                .map(|environment| TurnEnvironmentSelection {
                    environment_id: environment.environment_id,
                    cwd: environment.cwd,
                })
                .collect::<Vec<_>>()
        });
        if let Some(environment_selections) = environment_selections.as_ref() {
            self.turn_runtime
                .validate_environment_selections(environment_selections)
                .map_err(|err| invalid_request(environment_selection_error_message(err)))?;
        }
        Ok(environment_selections)
    }

    async fn request_trace_context(
        &self,
        request_id: &ConnectionRequestId,
    ) -> Option<protocol::protocol::W3cTraceContext> {
        self.outgoing.request_trace_context(request_id).await
    }

    fn input_too_large_error(actual_chars: usize) -> JSONRPCErrorError {
        let mut error = invalid_params(format!(
            "Input exceeds the maximum length of {MAX_USER_INPUT_TEXT_CHARS} characters."
        ));
        error.data = Some(serde_json::json!({
            "input_error_code": INPUT_TOO_LARGE_ERROR_CODE,
            "max_chars": MAX_USER_INPUT_TEXT_CHARS,
            "actual_chars": actual_chars,
        }));
        error
    }

    fn validate_v2_input_limit(items: &[V2UserInput]) -> Result<(), JSONRPCErrorError> {
        let actual_chars: usize = items.iter().map(V2UserInput::text_char_count).sum();
        if actual_chars > MAX_USER_INPUT_TEXT_CHARS {
            return Err(Self::input_too_large_error(actual_chars));
        }
        Ok(())
    }

    async fn turn_start_inner(
        &self,
        request_id: ConnectionRequestId,
        params: TurnStartParams,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
    ) -> Result<TurnStartResponse, JSONRPCErrorError> {
        if let Err(error) = Self::validate_v2_input_limit(&params.input) {
            self.track_error_response(
                &request_id,
                &error,
                Some(AnalyticsJsonRpcError::Input(InputError::TooLarge)),
            );
            return Err(error);
        }
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))
            .inspect_err(|error| {
                self.track_error_response(&request_id, error, /*error_type*/ None);
            })?;
        self.set_app_server_client_info(
            thread_id,
            app_server_client_name,
            app_server_client_version,
        )
        .await
        .inspect_err(|error| {
            self.track_error_response(&request_id, error, /*error_type*/ None);
        })?;

        let collaboration_mode = params
            .collaboration_mode
            .map(|mode| self.normalize_turn_start_collaboration_mode(mode));
        let environment_selections = self.parse_environment_selections(params.environments)?;

        // Map protocol input items to core input items.
        let mapped_items: Vec<CoreInputItem> = params
            .input
            .into_iter()
            .map(V2UserInput::into_core)
            .collect();
        let turn_has_input = !mapped_items.is_empty();
        let runtime_workspace_roots_request = params.runtime_workspace_roots.clone();
        let snapshot = if params.permissions.is_some() || runtime_workspace_roots_request.is_some()
        {
            Some(
                self.live_thread_inspection
                    .live_thread_snapshot(thread_id)
                    .await
                    .map_err(|err| match err {
                        CodexErr::ThreadNotFound(thread_id) => {
                            invalid_request(format!("thread not found: {thread_id}"))
                        }
                        err => internal_error(format!("failed to load thread snapshot: {err}")),
                    })?
                    .config_snapshot,
            )
        } else {
            None
        };

        let has_any_overrides = params.cwd.is_some()
            || runtime_workspace_roots_request.is_some()
            || params.approval_policy.is_some()
            || params.approvals_reviewer.is_some()
            || params.sandbox_policy.is_some()
            || params.permissions.is_some()
            || params.model.is_some()
            || params.model_provider.is_some()
            || params.service_tier.is_some()
            || params.effort.is_some()
            || params.summary.is_some()
            || collaboration_mode.is_some()
            || params.personality.is_some();

        if params.sandbox_policy.is_some() && params.permissions.is_some() {
            return Err(invalid_request(
                "`permissions` cannot be combined with `sandboxPolicy`",
            ));
        }

        let cwd = params.cwd;
        let runtime_workspace_roots = if let Some(workspace_roots) =
            runtime_workspace_roots_request.clone()
        {
            let Some(snapshot) = snapshot.as_ref() else {
                return Err(internal_error(
                    "turn/start runtime workspace roots missing thread snapshot",
                ));
            };
            let base_cwd = cwd
                .as_ref()
                .map(|cwd| AbsolutePathBuf::resolve_path_against_base(cwd, snapshot.cwd.as_path()))
                .unwrap_or_else(|| snapshot.cwd.clone());
            Some(resolve_runtime_workspace_roots(workspace_roots, &base_cwd))
        } else {
            None
        };
        let approval_policy = params.approval_policy.map(AskForApproval::to_core);
        let approvals_reviewer = params
            .approvals_reviewer
            .map(app_server_protocol::ApprovalsReviewer::to_core);
        let sandbox_policy = params.sandbox_policy.map(|p| p.to_core());
        let (permission_profile, active_permission_profile, profile_workspace_roots) =
            if let Some(permissions) = params.permissions {
                let Some(snapshot) = snapshot.as_ref() else {
                    return Err(internal_error(
                        "turn/start permission selection missing thread snapshot",
                    ));
                };
                let mut overrides = ConfigOverrides {
                    cwd: cwd.clone(),
                    workspace_roots: Some(runtime_workspace_roots_request.clone().unwrap_or_else(
                        || {
                            snapshot
                                .workspace_roots
                                .iter()
                                .map(AbsolutePathBuf::to_path_buf)
                                .collect()
                        },
                    )),
                    codex_linux_sandbox_exe: self.arg0_paths.codex_linux_sandbox_exe.clone(),
                    main_execve_wrapper_exe: self.arg0_paths.main_execve_wrapper_exe.clone(),
                    ..Default::default()
                };
                apply_permission_profile_selection_to_config_overrides(
                    &mut overrides,
                    Some(permissions),
                );
                let config = self
                    .config_manager
                    .load_for_cwd(
                        /*request_overrides*/ None,
                        overrides,
                        Some(snapshot.cwd.to_path_buf()),
                    )
                    .await
                    .map_err(|err| config_load_error(&err))?;
                // Startup config is allowed to fall back when requirements
                // disallow a configured profile. An explicit turn request
                // is different: reject it before accepting user input.
                if let Some(warning) = config.startup_warnings.iter().find(|warning| {
                    warning.contains("Configured value for `permission_profile` is disallowed")
                }) {
                    return Err(invalid_request(format!(
                        "invalid turn context override: {warning}"
                    )));
                }
                (
                    Some(config.permissions.permission_profile().clone()),
                    config.permissions.active_permission_profile(),
                    Some(config.permissions.profile_workspace_roots().to_vec()),
                )
            } else {
                (None, None, None)
            };
        let model = params.model;
        let model_provider = params.model_provider;
        let effort = params.effort.map(Some);
        let summary = params.summary;
        let service_tier = params.service_tier;
        let personality = params.personality;

        // If any overrides are provided, validate them synchronously so the
        // request can fail before accepting user input. The actual update is
        // still queued together with the input below to preserve submission order.
        if has_any_overrides {
            self.live_thread_turn
                .validate_live_thread_turn_context_overrides(
                    thread_id,
                    CodexThreadTurnContextOverrides {
                        cwd: cwd.clone(),
                        workspace_roots: runtime_workspace_roots.clone(),
                        approval_policy,
                        approvals_reviewer,
                        sandbox_policy: sandbox_policy.clone(),
                        permission_profile: permission_profile.clone(),
                        active_permission_profile: active_permission_profile.clone(),
                        profile_workspace_roots: profile_workspace_roots.clone(),
                        windows_sandbox_level: None,
                        model_provider: model_provider.clone(),
                        model: model.clone(),
                        effort,
                        summary,
                        service_tier: service_tier.clone(),
                        collaboration_mode: collaboration_mode.clone(),
                        personality,
                    },
                )
                .await
                .map_err(|err| match err {
                    CodexErr::ThreadNotFound(thread_id) => {
                        invalid_request(format!("thread not found: {thread_id}"))
                    }
                    CodexErr::InvalidRequest(message) => invalid_request(message),
                    err => {
                        internal_error(format!("failed to validate turn context override: {err}"))
                    }
                })?;
        }

        // Start the turn by submitting the user input. Return its submission id as turn_id.
        let turn_op = if has_any_overrides {
            Op::UserInputWithTurnContext {
                items: mapped_items,
                environments: environment_selections,
                final_output_json_schema: params.output_schema,
                responsesapi_client_metadata: params.responsesapi_client_metadata,
                cwd,
                workspace_roots: runtime_workspace_roots,
                profile_workspace_roots,
                approval_policy,
                approvals_reviewer,
                sandbox_policy,
                permission_profile,
                active_permission_profile,
                windows_sandbox_level: None,
                model,
                model_provider,
                effort,
                summary,
                service_tier,
                collaboration_mode,
                personality,
            }
        } else {
            Op::UserInput {
                items: mapped_items,
                environments: environment_selections,
                final_output_json_schema: params.output_schema,
                responsesapi_client_metadata: params.responsesapi_client_metadata,
            }
        };
        let turn_id = self
            .live_thread_command
            .submit_live_thread_op_with_trace(
                thread_id,
                turn_op,
                self.request_trace_context(&request_id).await,
            )
            .await
            .map_err(|err| {
                let error = match err {
                    CodexErr::ThreadNotFound(thread_id) => {
                        invalid_request(format!("thread not found: {thread_id}"))
                    }
                    err => internal_error(format!("failed to start turn: {err}")),
                };
                self.track_error_response(&request_id, &error, /*error_type*/ None);
                error
            })?;

        if turn_has_input {
            let config_snapshot = self
                .live_thread_inspection
                .live_thread_snapshot(thread_id)
                .await
                .map_err(|err| match err {
                    CodexErr::ThreadNotFound(thread_id) => {
                        invalid_request(format!("thread not found: {thread_id}"))
                    }
                    err => internal_error(format!("failed to load thread snapshot: {err}")),
                })?
                .config_snapshot;
            let thread_config = self
                .turn_runtime
                .live_thread_config(thread_id)
                .await
                .map_err(|err| match err {
                    CodexErr::ThreadNotFound(thread_id) => {
                        invalid_request(format!("thread not found: {thread_id}"))
                    }
                    err => internal_error(format!("failed to load thread config: {err}")),
                })?;
            let runtime = Arc::new(
                crate::memory_service_wiring::AppServerMemoryStartupAdapter::new(
                    self.memory_startup_host.clone(),
                    self.model_service.clone(),
                    Arc::clone(&self.auth_manager),
                    thread_id,
                    config_snapshot.clone(),
                    Arc::clone(&thread_config),
                    self.state_db.clone(),
                ),
            );
            let settings = crate::memory_service_wiring::build_memory_startup_settings(
                thread_config.as_ref(),
                config_snapshot.session_source,
            );
            memory_service::start_memories_startup_task(
                runtime,
                Arc::clone(&self.auth_manager),
                thread_id,
                settings,
            );
        }

        self.outgoing
            .record_request_turn_id(&request_id, &turn_id)
            .await;
        let turn = Turn {
            id: turn_id,
            items: vec![],
            items_view: TurnItemsView::NotLoaded,
            error: None,
            status: TurnStatus::InProgress,
            started_at: None,
            completed_at: None,
            duration_ms: None,
        };

        Ok(TurnStartResponse { turn })
    }

    async fn thread_inject_items_response_inner(
        &self,
        params: ThreadInjectItemsParams,
    ) -> Result<ThreadInjectItemsResponse, JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        let items = params
            .items
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                serde_json::from_value::<ResponseItem>(value)
                    .map_err(|err| format!("items[{index}] is not a valid response item: {err}"))
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(invalid_request)?;

        self.turn_runtime
            .inject_thread_conversation_items(thread_id, items)
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(thread_id) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                CodexErr::InvalidRequest(message) => invalid_request(message),
                err => internal_error(format!("failed to inject response items: {err}")),
            })?;
        Ok(ThreadInjectItemsResponse {})
    }

    async fn set_app_server_client_info(
        &self,
        thread_id: ThreadId,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
    ) -> Result<(), JSONRPCErrorError> {
        let mcp_elicitations_auto_deny = xcode_26_4_mcp_elicitations_auto_deny(
            app_server_client_name.as_deref(),
            app_server_client_version.as_deref(),
        );
        self.live_thread_command
            .set_live_thread_app_server_client_info(
                thread_id,
                AppServerClientInfo {
                    app_server_client_name,
                    app_server_client_version,
                    mcp_elicitations_auto_deny,
                },
            )
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(thread_id) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                err => internal_error(format!("failed to set app server client info: {err}")),
            })
    }

    async fn turn_steer_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: TurnSteerParams,
    ) -> Result<TurnSteerResponse, JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))
            .inspect_err(|error| {
                self.track_error_response(request_id, error, /*error_type*/ None);
            })?;

        if params.expected_turn_id.is_empty() {
            return Err(invalid_request("expectedTurnId must not be empty"));
        }
        self.outgoing
            .record_request_turn_id(request_id, &params.expected_turn_id)
            .await;
        if let Err(error) = Self::validate_v2_input_limit(&params.input) {
            self.track_error_response(
                request_id,
                &error,
                Some(AnalyticsJsonRpcError::Input(InputError::TooLarge)),
            );
            return Err(error);
        }

        let mapped_items: Vec<CoreInputItem> = params
            .input
            .into_iter()
            .map(V2UserInput::into_core)
            .collect();

        let turn_id = self
            .turn_runtime
            .steer_thread_input(
                thread_id,
                mapped_items,
                Some(params.expected_turn_id.clone()),
                params.responsesapi_client_metadata,
            )
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(thread_id) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                err => internal_error(format!("failed to steer turn: {err}")),
            })?
            .map_err(|err| {
                let (message, data, error_type) = match err {
                    SteerInputError::NoActiveTurn(_) => (
                        "no active turn to steer".to_string(),
                        None,
                        Some(AnalyticsJsonRpcError::TurnSteer(
                            TurnSteerRequestError::NoActiveTurn,
                        )),
                    ),
                    SteerInputError::ExpectedTurnMismatch { expected, actual } => (
                        format!("expected active turn id `{expected}` but found `{actual}`"),
                        None,
                        Some(AnalyticsJsonRpcError::TurnSteer(
                            TurnSteerRequestError::ExpectedTurnMismatch,
                        )),
                    ),
                    SteerInputError::ActiveTurnNotSteerable { turn_kind } => {
                        let (message, turn_steer_error) = match turn_kind {
                            protocol::protocol::NonSteerableTurnKind::Review => (
                                "cannot steer a review turn".to_string(),
                                TurnSteerRequestError::NonSteerableReview,
                            ),
                            protocol::protocol::NonSteerableTurnKind::Compact => (
                                "cannot steer a compact turn".to_string(),
                                TurnSteerRequestError::NonSteerableCompact,
                            ),
                        };
                        let error = TurnError {
                            message: message.clone(),
                            codex_error_info: Some(CodexErrorInfo::ActiveTurnNotSteerable {
                                turn_kind: turn_kind.into(),
                            }),
                            additional_details: None,
                        };
                        let data = match serde_json::to_value(error) {
                            Ok(data) => Some(data),
                            Err(error) => {
                                tracing::error!(
                                    ?error,
                                    "failed to serialize active-turn-not-steerable turn error"
                                );
                                None
                            }
                        };
                        (
                            message,
                            data,
                            Some(AnalyticsJsonRpcError::TurnSteer(turn_steer_error)),
                        )
                    }
                    SteerInputError::EmptyInput => (
                        "input must not be empty".to_string(),
                        None,
                        Some(AnalyticsJsonRpcError::Input(InputError::Empty)),
                    ),
                };
                let mut error = invalid_request(message);
                error.data = data;
                self.track_error_response(request_id, &error, error_type);
                error
            })?;
        Ok(TurnSteerResponse { turn_id })
    }

    async fn prepare_realtime_conversation_thread(
        &self,
        request_id: &ConnectionRequestId,
        thread_id: &str,
    ) -> Result<Option<ThreadId>, JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        match self
            .ensure_conversation_listener(thread_id, request_id.connection_id)
            .await
        {
            Ok(EnsureConversationListenerResult::Attached) => {}
            Ok(EnsureConversationListenerResult::ConnectionClosed) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }

        let realtime_enabled = self
            .live_thread_inspection
            .live_thread_feature_enabled(thread_id, Feature::RealtimeConversation)
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(thread_id) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                err => internal_error(format!("failed to read thread feature state: {err}")),
            })?;
        if !realtime_enabled {
            return Err(invalid_request(format!(
                "thread {thread_id} does not support realtime conversation"
            )));
        }

        Ok(Some(thread_id))
    }

    async fn thread_realtime_start_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRealtimeStartParams,
    ) -> Result<Option<ThreadRealtimeStartResponse>, JSONRPCErrorError> {
        let Some(thread_id) = self
            .prepare_realtime_conversation_thread(request_id, &params.thread_id)
            .await?
        else {
            return Ok(None);
        };
        self.live_thread_command
            .submit_live_thread_op_with_trace(
                thread_id,
                Op::RealtimeConversationStart(ConversationStartParams {
                    output_modality: params.output_modality,
                    prompt: params.prompt,
                    realtime_session_id: params.realtime_session_id,
                    transport: params.transport.map(|transport| match transport {
                        ThreadRealtimeStartTransport::Websocket => {
                            ConversationStartTransport::Websocket
                        }
                        ThreadRealtimeStartTransport::Webrtc { sdp } => {
                            ConversationStartTransport::Webrtc { sdp }
                        }
                    }),
                    voice: params.voice,
                }),
                self.request_trace_context(request_id).await,
            )
            .await
            .map_err(|err| {
                internal_error(format!("failed to start realtime conversation: {err}"))
            })?;
        Ok(Some(ThreadRealtimeStartResponse::default()))
    }

    async fn thread_realtime_append_audio_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRealtimeAppendAudioParams,
    ) -> Result<Option<ThreadRealtimeAppendAudioResponse>, JSONRPCErrorError> {
        let Some(thread_id) = self
            .prepare_realtime_conversation_thread(request_id, &params.thread_id)
            .await?
        else {
            return Ok(None);
        };
        self.live_thread_command
            .submit_live_thread_op_with_trace(
                thread_id,
                Op::RealtimeConversationAudio(ConversationAudioParams {
                    frame: params.audio.into(),
                }),
                self.request_trace_context(request_id).await,
            )
            .await
            .map_err(|err| {
                internal_error(format!(
                    "failed to append realtime conversation audio: {err}"
                ))
            })?;
        Ok(Some(ThreadRealtimeAppendAudioResponse::default()))
    }

    async fn thread_realtime_append_text_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRealtimeAppendTextParams,
    ) -> Result<Option<ThreadRealtimeAppendTextResponse>, JSONRPCErrorError> {
        let Some(thread_id) = self
            .prepare_realtime_conversation_thread(request_id, &params.thread_id)
            .await?
        else {
            return Ok(None);
        };
        self.live_thread_command
            .submit_live_thread_op_with_trace(
                thread_id,
                Op::RealtimeConversationText(ConversationTextParams { text: params.text }),
                self.request_trace_context(request_id).await,
            )
            .await
            .map_err(|err| {
                internal_error(format!(
                    "failed to append realtime conversation text: {err}"
                ))
            })?;
        Ok(Some(ThreadRealtimeAppendTextResponse::default()))
    }

    async fn thread_realtime_stop_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRealtimeStopParams,
    ) -> Result<Option<ThreadRealtimeStopResponse>, JSONRPCErrorError> {
        let Some(thread_id) = self
            .prepare_realtime_conversation_thread(request_id, &params.thread_id)
            .await?
        else {
            return Ok(None);
        };
        self.live_thread_command
            .submit_live_thread_op_with_trace(
                thread_id,
                Op::RealtimeConversationClose,
                self.request_trace_context(request_id).await,
            )
            .await
            .map_err(|err| {
                internal_error(format!("failed to stop realtime conversation: {err}"))
            })?;
        Ok(Some(ThreadRealtimeStopResponse::default()))
    }

    fn build_review_turn(turn_id: String, display_text: &str) -> Turn {
        let items = if display_text.is_empty() {
            Vec::new()
        } else {
            vec![ThreadItem::UserMessage {
                id: turn_id.clone(),
                content: vec![V2UserInput::Text {
                    text: display_text.to_string(),
                    // Review prompt display text is synthesized; no UI element ranges to preserve.
                    text_elements: Vec::new(),
                }],
            }]
        };

        Turn {
            id: turn_id,
            items,
            items_view: TurnItemsView::NotLoaded,
            error: None,
            status: TurnStatus::InProgress,
            started_at: None,
            completed_at: None,
            duration_ms: None,
        }
    }

    async fn emit_review_started(
        &self,
        request_id: &ConnectionRequestId,
        turn: Turn,
        review_thread_id: String,
    ) {
        let response = ReviewStartResponse {
            turn,
            review_thread_id,
        };
        self.outgoing
            .send_response(request_id.clone(), response)
            .await;
    }

    async fn start_inline_review(
        &self,
        request_id: &ConnectionRequestId,
        parent_thread_id: ThreadId,
        review_request: ReviewRequest,
        display_text: &str,
    ) -> std::result::Result<(), JSONRPCErrorError> {
        let turn_id = self
            .live_thread_command
            .submit_live_thread_op_with_trace(
                parent_thread_id,
                Op::Review { review_request },
                self.request_trace_context(request_id).await,
            )
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(thread_id) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                err => internal_error(format!("failed to start review: {err}")),
            })?;
        let turn = Self::build_review_turn(turn_id, display_text);
        self.emit_review_started(request_id, turn, parent_thread_id.to_string())
            .await;
        Ok(())
    }

    async fn start_detached_review(
        &self,
        request_id: &ConnectionRequestId,
        parent_thread_id: ThreadId,
        review_request: ReviewRequest,
        display_text: &str,
    ) -> std::result::Result<(), JSONRPCErrorError> {
        let mut config = self.config.as_ref().clone();
        if let Some(review_model) = &config.review_model {
            config.model = Some(review_model.clone());
        }

        let DetachedReviewThread { thread_id } = self
            .turn_runtime
            .fork_detached_review_thread(
                parent_thread_id,
                config,
                self.request_trace_context(request_id).await,
            )
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(thread_id) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                err => internal_error(format!("error creating detached review thread: {err}")),
            })?;

        log_listener_attach_result(
            self.ensure_conversation_listener(thread_id, request_id.connection_id)
                .await,
            thread_id,
            request_id.connection_id,
            "review thread",
        );

        let fallback_provider = self.config.model_provider_id.as_str();
        match self
            .turn_runtime
            .read_detached_review_thread(thread_id)
            .await
        {
            Ok(stored_thread) => {
                let (mut thread, _) =
                    thread_from_stored_thread(stored_thread, fallback_provider, &self.config.cwd);
                thread.session_id = self
                    .live_thread_inspection
                    .live_thread_info(thread_id)
                    .await
                    .map_err(|err| {
                        invalid_request(format!(
                            "failed to read review thread live info: {err}"
                        ))
                    })?
                    .session_id
                    .to_string();
                self.thread_watch_manager
                    .upsert_thread_silently(thread.clone())
                    .await;
                thread.lifecycle_status = resolve_thread_status(
                    self.thread_watch_manager
                        .loaded_status_for_thread(&thread.id)
                        .await,
                    /*has_in_progress_turn*/ false,
                );
                let notif = thread_started_notification(thread);
                self.outgoing
                    .send_server_notification(ServerNotification::ThreadStarted(notif))
                    .await;
            }
            Err(err) => {
                tracing::warn!("failed to load summary for review thread {thread_id}: {err}");
            }
        }

        let turn_id = self
            .live_thread_command
            .submit_live_thread_op_with_trace(
                thread_id,
                Op::Review { review_request },
                self.request_trace_context(request_id).await,
            )
            .await
            .map_err(|err| {
                internal_error(format!("failed to start detached review turn: {err}"))
            })?;

        let turn = Self::build_review_turn(turn_id, display_text);
        let review_thread_id = thread_id.to_string();
        self.emit_review_started(request_id, turn, review_thread_id)
            .await;

        Ok(())
    }

    async fn review_start_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ReviewStartParams,
    ) -> Result<(), JSONRPCErrorError> {
        let ReviewStartParams {
            thread_id,
            target,
            delivery,
        } = params;

        let parent_thread_id = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;
        let (review_request, display_text) = Self::review_request_from_target(target)?;
        match delivery.unwrap_or(ApiReviewDelivery::Inline).to_core() {
            CoreReviewDelivery::Inline => {
                self.start_inline_review(
                    request_id,
                    parent_thread_id,
                    review_request,
                    &display_text,
                )
                .await?;
            }
            CoreReviewDelivery::Detached => {
                self.start_detached_review(
                    request_id,
                    parent_thread_id,
                    review_request,
                    &display_text,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn turn_interrupt_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: TurnInterruptParams,
    ) -> Result<Option<TurnInterruptResponse>, JSONRPCErrorError> {
        let TurnInterruptParams { thread_id, turn_id } = params;
        let is_startup_interrupt = turn_id.is_empty();

        let thread_uuid = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        // Record turn interrupts so we can reply when TurnAborted arrives. Startup
        // interrupts do not have a turn and are acknowledged after submission.
        if !is_startup_interrupt {
            let thread_state = self.thread_state_manager.thread_state(thread_uuid).await;
            let is_running = matches!(
                self.live_thread_status
                    .live_thread_agent_status(thread_uuid)
                    .await
                    .map_err(|err| match err {
                        CodexErr::ThreadNotFound(thread_id) => {
                            invalid_request(format!("thread not found: {thread_id}"))
                        }
                        err => internal_error(format!("failed to read thread status: {err}")),
                    })?,
                AgentStatus::Running
            );
            {
                let mut thread_state = thread_state.lock().await;
                if let Some(active_turn) = thread_state.active_turn_snapshot() {
                    if active_turn.id != turn_id {
                        return Err(invalid_request(format!(
                            "expected active turn id {turn_id} but found {}",
                            active_turn.id
                        )));
                    }
                } else if thread_state.last_terminal_turn_id.as_deref() == Some(turn_id.as_str())
                    || !is_running
                {
                    return Err(invalid_request("no active turn to interrupt"));
                }
                thread_state.pending_interrupts.push(request_id.clone());
            }

            self.outgoing
                .record_request_turn_id(request_id, &turn_id)
                .await;
        }

        // Submit the interrupt. Turn interrupts respond upon TurnAborted; startup
        // interrupts respond here because startup cancellation has no turn event.
        match self
            .live_thread_command
            .submit_live_thread_op_with_trace(
                thread_uuid,
                Op::Interrupt,
                self.request_trace_context(request_id).await,
            )
            .await
        {
            Ok(_) if is_startup_interrupt => Ok(Some(TurnInterruptResponse {})),
            Ok(_) => Ok(None),
            Err(CodexErr::ThreadNotFound(thread_id)) => {
                if !is_startup_interrupt {
                    let thread_state = self.thread_state_manager.thread_state(thread_uuid).await;
                    let mut thread_state = thread_state.lock().await;
                    thread_state
                        .pending_interrupts
                        .retain(|pending_request_id| pending_request_id != request_id);
                }
                Err(invalid_request(format!("thread not found: {thread_id}")))
            }
            Err(err) => {
                if !is_startup_interrupt {
                    let thread_state = self.thread_state_manager.thread_state(thread_uuid).await;
                    let mut thread_state = thread_state.lock().await;
                    thread_state
                        .pending_interrupts
                        .retain(|pending_request_id| pending_request_id != request_id);
                }
                let interrupt_target = if is_startup_interrupt {
                    "startup"
                } else {
                    "turn"
                };
                Err(internal_error(format!(
                    "failed to interrupt {interrupt_target}: {err}"
                )))
            }
        }
    }

    fn listener_task_context(&self) -> ListenerTaskContext {
        ListenerTaskContext {
            live_thread_listener: self.live_thread_listener.clone(),
            live_thread_inspection: self.live_thread_inspection.clone(),
            live_thread_command: self.live_thread_command.clone(),
            live_thread_skill_watch: self.live_thread_skill_watch.clone(),
            thread_state_manager: self.thread_state_manager.clone(),
            outgoing: Arc::clone(&self.outgoing),
            pending_thread_unloads: Arc::clone(&self.pending_thread_unloads),
            thread_watch_manager: self.thread_watch_manager.clone(),
            thread_list_state_permit: self.thread_list_state_permit.clone(),
            fallback_model_provider: self.config.model_provider_id.clone(),
            codex_home: self.config.codex_home.to_path_buf(),
            skills_watcher: Arc::clone(&self.skills_watcher),
        }
    }

    async fn ensure_conversation_listener(
        &self,
        conversation_id: ThreadId,
        connection_id: ConnectionId,
    ) -> Result<EnsureConversationListenerResult, JSONRPCErrorError> {
        super::thread_lifecycle::ensure_conversation_listener(
            self.listener_task_context(),
            conversation_id,
            connection_id,
        )
        .await
    }
}

fn xcode_26_4_mcp_elicitations_auto_deny(
    client_name: Option<&str>,
    client_version: Option<&str>,
) -> bool {
    // Xcode 26.4 shipped before app-server MCP elicitation requests were
    // client-visible. Keep elicitations auto-denied for that client line.
    // TODO: Remove this compatibility hack once Xcode 26.4 ages out.
    client_name == Some("Xcode")
        && client_version.is_some_and(|version| version.starts_with("26.4"))
}
