use super::*;
use crate::request_processors::thread_processor::ops::parse_thread_start_agent;

impl ThreadRequestProcessor {
    pub(super) async fn thread_start_inner(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadStartParams,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
        request_context: RequestContext,
    ) -> Result<(), JSONRPCErrorError> {
        let ThreadStartParams {
            model,
            model_provider,
            reasoning_effort,
            service_tier,
            cwd,
            task_name,
            agent_type,
            runtime_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox,
            permissions,
            config: mut request_overrides,
            service_name,
            base_instructions,
            developer_instructions,
            dynamic_tools,
            mock_experimental_field: _mock_experimental_field,
            personality,
            ephemeral,
            session_start_source,
            thread_source,
            environments,
            persist_extended_history,
        } = params;
        if sandbox.is_some() && permissions.is_some() {
            return Err(invalid_request(
                "`permissions` cannot be combined with `sandbox`",
            ));
        }
        if persist_extended_history {
            self.send_persist_extended_history_deprecation_notice(request_id.connection_id)
                .await;
        }
        let environment_selections = self.parse_environment_selections(environments)?;
        let thread_start_agent = parse_thread_start_agent(task_name, agent_type)?;
        if let Some(reasoning_effort) = reasoning_effort {
            request_overrides
                .get_or_insert_with(std::collections::HashMap::new)
                .insert(
                    "model_reasoning_effort".to_string(),
                    serde_json::Value::String(reasoning_effort.to_string()),
                );
        }
        let mut typesafe_overrides = self.build_thread_config_overrides(
            model,
            model_provider,
            service_tier,
            cwd,
            runtime_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox,
            permissions,
            base_instructions,
            developer_instructions,
            personality,
        );
        typesafe_overrides.ephemeral = ephemeral;
        let listener_task_context = ListenerTaskContext {
            live_threads: Arc::clone(&self.live_threads),
            thread_state_manager: self.thread_state_manager.clone(),
            outgoing: Arc::clone(&self.outgoing),
            pending_thread_unloads: Arc::clone(&self.pending_thread_unloads),
            thread_watch_manager: self.thread_watch_manager.clone(),
            thread_list_state_permit: self.thread_list_state_permit.clone(),
            fallback_model_provider: self.config.model_provider_id.clone(),
            codex_home: self.config.codex_home.to_path_buf(),
            skills_watcher: Arc::clone(&self.skills_watcher),
        };
        let request_trace = request_context.request_trace();
        let config_manager = self.config_manager.clone();
        let thread_runtime = Arc::clone(&self.thread_runtime);
        let live_threads = Arc::clone(&self.live_threads);
        let thread_store = Arc::clone(&self.thread_store);
        let outgoing = Arc::clone(&listener_task_context.outgoing);
        let error_request_id = request_id.clone();
        let thread_start_task = async move {
            if let Err(error) = Self::thread_start_task(
                listener_task_context,
                thread_runtime,
                live_threads,
                thread_store,
                config_manager,
                request_id,
                app_server_client_name,
                app_server_client_version,
                request_overrides,
                typesafe_overrides,
                dynamic_tools,
                thread_start_agent,
                session_start_source,
                thread_source.map(Into::into),
                environment_selections,
                service_name,
                request_trace,
            )
            .await
            {
                outgoing.send_error(error_request_id, error).await;
            }
        };
        self.background_tasks
            .spawn(thread_start_task.instrument(request_context.span()));
        Ok(())
    }

    pub(super) async fn thread_resume_inner(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadResumeParams,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
    ) -> Result<(), JSONRPCErrorError> {
        if let Ok(thread_id) = ThreadId::from_string(&params.thread_id)
            && self
                .pending_thread_unloads
                .lock()
                .await
                .contains(&thread_id)
        {
            self.outgoing
                .send_error(
                    request_id,
                    invalid_request(format!(
                        "thread {thread_id} is closing; retry thread/resume after the thread is closed"
                    )),
                )
                .await;
            return Ok(());
        }

        if params.sandbox.is_some() && params.permissions.is_some() {
            self.outgoing
                .send_error(
                    request_id,
                    invalid_request("`permissions` cannot be combined with `sandbox`"),
                )
                .await;
            return Ok(());
        }
        if params.persist_extended_history {
            self.send_persist_extended_history_deprecation_notice(request_id.connection_id)
                .await;
        }
        let redact_resume_payloads =
            should_redact_thread_resume_payloads(app_server_client_name.as_deref());

        let _thread_list_state_permit = match self.acquire_thread_list_state_permit().await {
            Ok(permit) => permit,
            Err(error) => {
                self.outgoing.send_error(request_id, error).await;
                return Ok(());
            }
        };
        match self
            .resume_running_thread(
                &request_id,
                &params,
                app_server_client_name.clone(),
                app_server_client_version.clone(),
            )
            .await
        {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => {
                self.outgoing.send_error(request_id, error).await;
                return Ok(());
            }
        }

        let ThreadResumeParams {
            thread_id,
            history,
            path,
            model,
            model_provider,
            service_tier,
            cwd,
            runtime_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox,
            permissions,
            config: mut request_overrides,
            base_instructions,
            developer_instructions,
            personality,
            exclude_turns,
            persist_extended_history: _persist_extended_history,
        } = params;
        let include_turns = !exclude_turns;

        let (thread_history, resume_source_thread) = match if let Some(history) = history {
            self.resume_thread_from_history(history.as_slice())
                .await
                .map(|thread_history| (thread_history, None))
        } else {
            self.resume_thread_from_rollout(&thread_id, path.as_ref())
                .await
                .map(|(thread_history, stored_thread)| (thread_history, Some(stored_thread)))
        } {
            Ok(value) => value,
            Err(error) => {
                self.outgoing.send_error(request_id, error).await;
                return Ok(());
            }
        };
        let resume_session_source = resume_source_thread
            .as_ref()
            .map(stored_thread_session_source_with_agent_metadata);
        let resume_agent_metadata = resume_source_thread
            .as_ref()
            .and_then(stored_thread_root_agent_metadata);
        let resume_agent_role = native_agent_role_for_resume(resume_session_source.as_ref());

        let history_cwd = thread_history.session_cwd();
        let mut typesafe_overrides = self.build_thread_config_overrides(
            model,
            model_provider,
            service_tier,
            cwd,
            runtime_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox,
            permissions,
            base_instructions,
            developer_instructions,
            personality,
        );
        self.load_and_apply_persisted_resume_metadata(
            &thread_history,
            &mut request_overrides,
            &mut typesafe_overrides,
        )
        .await;

        // Derive a Config using the same logic as new conversation, honoring overrides if provided.
        let mut config = match self
            .config_manager
            .load_for_cwd(request_overrides, typesafe_overrides, history_cwd)
            .await
        {
            Ok(config) => config,
            Err(err) => {
                let error = config_load_error(&err);
                self.outgoing.send_error(request_id, error).await;
                return Ok(());
            }
        };
        if let Some(agent_role) = resume_agent_role
            && let Err(err) =
                codex_agent_runtime::apply_role_to_config(&mut config, Some(agent_role)).await
        {
            self.outgoing.send_error(request_id, invalid_request(err)).await;
            return Ok(());
        }

        let instruction_sources = Self::instruction_sources_from_config(&config).await;
        let response_history = thread_history.clone();
        let parent_trace = self.request_trace_context(&request_id).await;

        let resume_result = if let Some(session_source) = resume_session_source {
            self.thread_runtime
                .resume_thread_with_history_and_source(
                    config.clone(),
                    thread_history,
                    session_source,
                    resume_agent_metadata,
                    parent_trace,
                )
                .await
        } else {
            self.thread_runtime
                .resume_thread_with_history(
                    config.clone(),
                    thread_history,
                    /*persist_extended_history*/ false,
                    parent_trace,
                )
                .await
        };

        match resume_result {
            Ok(ThreadProcessorNewThread {
                thread_id,
                thread: codex_thread,
                session_configured,
                ..
            }) => {
                if let Err(err) = self
                    .set_app_server_client_info(
                        thread_id,
                        app_server_client_name,
                        app_server_client_version,
                    )
                    .await
                {
                    self.outgoing.send_error(request_id, err).await;
                    return Ok(());
                }
                let SessionConfiguredEvent { rollout_path, .. } = session_configured;
                let Some(rollout_path) = rollout_path else {
                    let error =
                        internal_error(format!("rollout path missing for thread {thread_id}"));
                    self.outgoing.send_error(request_id, error).await;
                    return Ok(());
                };
                // Auto-attach a thread listener when resuming a thread.
                log_listener_attach_result(
                    self.ensure_conversation_listener(thread_id, request_id.connection_id)
                        .await,
                    thread_id,
                    request_id.connection_id,
                    "thread",
                );

                let mut thread = match self
                    .load_thread_from_resume_source_or_send_internal(
                        thread_id,
                        codex_thread.as_ref(),
                        &response_history,
                        rollout_path.as_path(),
                        resume_source_thread,
                        include_turns,
                    )
                    .await
                {
                    Ok(thread) => thread,
                    Err(message) => {
                        self.outgoing
                            .send_error(request_id, internal_error(message))
                            .await;
                        return Ok(());
                    }
                };
                thread.thread_source = codex_thread
                    .config_snapshot()
                    .await
                    .thread_source
                    .map(Into::into);

                self.thread_watch_manager
                    .upsert_thread(thread.clone())
                    .await;

                let thread_status = self
                    .thread_watch_manager
                    .loaded_status_for_thread(&thread.id)
                    .await;

                set_thread_status_and_interrupt_stale_turns(
                    &mut thread,
                    thread_status,
                    /*has_live_in_progress_turn*/ false,
                );
                let config_snapshot = codex_thread.config_snapshot().await;
                let sandbox = thread_response_sandbox_policy(
                    &config_snapshot.permission_profile,
                    config_snapshot.cwd.as_path(),
                );
                let active_permission_profile = thread_response_active_permission_profile(
                    config_snapshot.active_permission_profile,
                );
                let token_usage_thread = include_turns.then(|| thread.clone());
                if redact_resume_payloads {
                    redact_thread_resume_payloads(&mut thread);
                }

                let response = ThreadResumeResponse {
                    thread,
                    model: session_configured.model,
                    model_provider: session_configured.model_provider_id,
                    service_tier: session_configured.service_tier,
                    cwd: session_configured.cwd,
                    runtime_workspace_roots: config_snapshot.workspace_roots,
                    instruction_sources,
                    approval_policy: session_configured.approval_policy.into(),
                    approvals_reviewer: session_configured.approvals_reviewer.into(),
                    sandbox,
                    permission_profile: Some(config_snapshot.permission_profile.into()),
                    active_permission_profile,
                    reasoning_effort: session_configured.reasoning_effort,
                };

                let connection_id = request_id.connection_id;
                self.outgoing.send_response(request_id, response).await;
                // `excludeTurns` is explicitly the cheap resume path, so avoid
                // rebuilding history only to attribute a replayed usage update.
                if let Some(token_usage_thread) = token_usage_thread {
                    let token_usage_turn_id = latest_token_usage_turn_id_from_rollout_items(
                        &response_history.get_rollout_items(),
                        token_usage_thread.turns.as_slice(),
                    );
                    // The client needs restored usage before it starts another turn.
                    // Sending after the response preserves JSON-RPC request ordering while
                    // still filling the status line before the next turn lifecycle begins.
                    send_thread_token_usage_update_to_connection(
                        &self.outgoing,
                        connection_id,
                        thread_id,
                        &token_usage_thread,
                        codex_thread.as_ref(),
                        token_usage_turn_id,
                    )
                    .await;
                    send_thread_context_usage_update_to_connection(
                        &self.outgoing,
                        connection_id,
                        thread_id,
                        &token_usage_thread,
                        codex_thread.as_ref(),
                        response_history.get_rollout_items().as_slice(),
                    )
                    .await;
                }
                self.thread_goal_processor
                    .emit_resume_goal_snapshot_and_continue(thread_id, codex_thread.as_ref())
                    .await;
            }
            Err(err) => {
                let error = internal_error(format!("error resuming thread: {err}"));
                self.outgoing.send_error(request_id, error).await;
            }
        }
        Ok(())
    }

    pub(super) async fn load_and_apply_persisted_resume_metadata(
        &self,
        thread_history: &InitialHistory,
        request_overrides: &mut Option<HashMap<String, serde_json::Value>>,
        typesafe_overrides: &mut ConfigOverrides,
    ) -> Option<ThreadMetadata> {
        let InitialHistory::Resumed(resumed_history) = thread_history else {
            return None;
        };
        let state_db_ctx = self.state_db.clone()?;
        let persisted_metadata = state_db_ctx
            .get_thread(resumed_history.conversation_id)
            .await
            .ok()
            .flatten()?;
        merge_persisted_resume_metadata(request_overrides, typesafe_overrides, &persisted_metadata);
        Some(persisted_metadata)
    }

    pub(super) async fn resume_running_thread(
        &self,
        request_id: &ConnectionRequestId,
        params: &ThreadResumeParams,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
    ) -> Result<bool, JSONRPCErrorError> {
        let running_thread = if params.history.is_some() {
            if let Ok(existing_thread_id) = ThreadId::from_string(&params.thread_id)
                && self.live_threads.is_thread_loaded(existing_thread_id).await
            {
                return Err(invalid_request(format!(
                    "cannot resume thread {existing_thread_id} with history while it is already running"
                )));
            }
            None
        } else if params.path.is_some() {
            let source_thread = self
                .read_stored_thread_for_resume(
                    &params.thread_id,
                    params.path.as_ref(),
                    /*include_history*/ true,
                )
                .await?;
            let existing_thread_id = source_thread.thread_id;
            if let Ok(live_info) = self.live_threads.live_thread_info(existing_thread_id).await {
                if let (Some(requested_path), Some(active_path)) =
                    (params.path.as_ref(), live_info.rollout_path.as_ref())
                    && requested_path != active_path
                {
                    return Err(invalid_request(format!(
                        "cannot resume running thread {existing_thread_id} with stale path: requested `{}`, active `{}`",
                        requested_path.display(),
                        active_path.display()
                    )));
                }
                Some((existing_thread_id, source_thread))
            } else {
                None
            }
        } else if let Ok(existing_thread_id) = ThreadId::from_string(&params.thread_id)
            && self.live_threads.is_thread_loaded(existing_thread_id).await
        {
            let source_thread = self
                .read_stored_thread_for_resume(
                    &params.thread_id,
                    /*path*/ None,
                    /*include_history*/ true,
                )
                .await?;
            if source_thread.thread_id != existing_thread_id {
                return Err(invalid_request(format!(
                    "cannot resume running thread {existing_thread_id} from source thread {}",
                    source_thread.thread_id
                )));
            }
            Some((existing_thread_id, source_thread))
        } else {
            None
        };

        if let Some((existing_thread_id, source_thread)) = running_thread {
            match self
                .ensure_conversation_listener(existing_thread_id, request_id.connection_id)
                .await?
            {
                EnsureConversationListenerResult::Attached => {}
                EnsureConversationListenerResult::ConnectionClosed => {
                    return Ok(true);
                }
            }
            let live_snapshot = self
                .live_threads
                .live_thread_snapshot(existing_thread_id)
                .await
                .map_err(|err| match err {
                    CodexErr::ThreadNotFound(thread_id) => {
                        invalid_request(format!("thread not found: {thread_id}"))
                    }
                    err => internal_error(format!(
                        "failed to load running thread snapshot {existing_thread_id}: {err}"
                    )),
                })?;
            let redact_resume_payloads =
                should_redact_thread_resume_payloads(app_server_client_name.as_deref());
            let history_items = source_thread
                .history
                .as_ref()
                .map(|history| history.items.clone())
                .ok_or_else(|| {
                    internal_error(format!(
                        "thread {existing_thread_id} did not include persisted history"
                    ))
                })?;

            let thread_state = self
                .thread_state_manager
                .thread_state(existing_thread_id)
                .await;
            self.set_app_server_client_info(
                existing_thread_id,
                app_server_client_name,
                app_server_client_version,
            )
            .await?;

            let config_snapshot = live_snapshot.config_snapshot;
            let mismatch_details = collect_resume_override_mismatches(params, &config_snapshot);
            if !mismatch_details.is_empty() {
                tracing::warn!(
                    "thread/resume overrides ignored for running thread {}: {}",
                    existing_thread_id,
                    mismatch_details.join("; ")
                );
            }
            let mut summary_source_thread = source_thread;
            summary_source_thread.history = None;
            let mut thread_summary = self.stored_thread_to_api_thread(
                summary_source_thread,
                config_snapshot.model_provider_id.as_str(),
                /*include_turns*/ false,
            );
            thread_summary.session_id = live_snapshot.info.session_id.to_string();
            let mut config_for_instruction_sources = self.config.as_ref().clone();
            config_for_instruction_sources.cwd = config_snapshot.cwd.clone();
            let instruction_sources =
                Self::instruction_sources_from_config(&config_for_instruction_sources).await;

            let listener_command_tx = {
                let thread_state = thread_state.lock().await;
                thread_state.listener_command_tx()
            };
            let Some(listener_command_tx) = listener_command_tx else {
                return Err(internal_error(format!(
                    "failed to enqueue running thread resume for thread {existing_thread_id}: thread listener is not running"
                )));
            };

            let (emit_thread_goal_update, thread_goal_state_db) =
                self.thread_goal_processor.pending_resume_goal_state().await;

            let command = crate::thread_state::ThreadListenerCommand::SendThreadResumeResponse(
                Box::new(crate::thread_state::PendingThreadResumeRequest {
                    request_id: request_id.clone(),
                    history_items,
                    config_snapshot,
                    instruction_sources,
                    thread_summary,
                    emit_thread_goal_update,
                    thread_goal_state_db,
                    include_turns: !params.exclude_turns,
                    redact_resume_payloads,
                }),
            );
            if listener_command_tx.send(command).is_err() {
                return Err(internal_error(format!(
                    "failed to enqueue running thread resume for thread {existing_thread_id}: thread listener command channel is closed"
                )));
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) async fn resume_thread_from_history(
        &self,
        history: &[ResponseItem],
    ) -> Result<InitialHistory, JSONRPCErrorError> {
        if history.is_empty() {
            return Err(invalid_request("history must not be empty"));
        }
        Ok(InitialHistory::Forked(
            history
                .iter()
                .cloned()
                .map(RolloutItem::ResponseItem)
                .collect(),
        ))
    }

    pub(super) async fn resume_thread_from_rollout(
        &self,
        thread_id: &str,
        path: Option<&PathBuf>,
    ) -> Result<(InitialHistory, StoredThread), JSONRPCErrorError> {
        let stored_thread = self
            .read_stored_thread_for_resume(thread_id, path, /*include_history*/ true)
            .await?;
        let history = self
            .stored_thread_to_initial_history(&stored_thread)
            .await?;
        Ok((history, stored_thread))
    }

    pub(super) async fn read_stored_thread_for_resume(
        &self,
        thread_id: &str,
        path: Option<&PathBuf>,
        include_history: bool,
    ) -> Result<StoredThread, JSONRPCErrorError> {
        let result = if let Some(path) = path {
            self.thread_store
                .read_thread_by_rollout_path(StoreReadThreadByRolloutPathParams {
                    rollout_path: path.clone(),
                    include_archived: true,
                    include_history,
                })
                .await
        } else {
            let existing_thread_id = match ThreadId::from_string(thread_id) {
                Ok(id) => id,
                Err(err) => {
                    return Err(invalid_request(format!("invalid thread id: {err}")));
                }
            };
            let params = StoreReadThreadParams {
                thread_id: existing_thread_id,
                include_archived: true,
                include_history,
            };
            self.thread_store.read_thread(params).await
        };

        result.map_err(thread_store_resume_read_error)
    }

    pub(super) async fn stored_thread_to_initial_history(
        &self,
        stored_thread: &StoredThread,
    ) -> Result<InitialHistory, JSONRPCErrorError> {
        let thread_id = stored_thread.thread_id;
        let history = stored_thread
            .history
            .as_ref()
            .map(|history| history.items.clone())
            .ok_or_else(|| {
                internal_error(format!(
                    "thread {thread_id} did not include persisted history"
                ))
            })?;
        Ok(InitialHistory::Resumed(ResumedHistory {
            conversation_id: thread_id,
            history,
            rollout_path: stored_thread.rollout_path.clone(),
        }))
    }

    pub(super) fn stored_thread_to_api_thread(
        &self,
        stored_thread: StoredThread,
        fallback_provider: &str,
        include_turns: bool,
    ) -> Thread {
        let (mut thread, history) =
            thread_from_stored_thread(stored_thread, fallback_provider, &self.config.cwd);
        if include_turns && let Some(history) = history {
            populate_thread_turns_from_history(
                &mut thread,
                &history.items,
                /*active_turn*/ None,
            );
        }
        thread
    }

    pub(super) async fn read_stored_thread_for_new_fork(
        &self,
        thread_id: ThreadId,
        include_history: bool,
    ) -> Result<StoredThread, JSONRPCErrorError> {
        self.thread_store
            .read_thread(StoreReadThreadParams {
                thread_id,
                include_archived: true,
                include_history,
            })
            .await
            .map_err(thread_store_resume_read_error)
    }

    pub(super) async fn load_thread_from_resume_source_or_send_internal(
        &self,
        thread_id: ThreadId,
        codex_thread: &dyn AppServerLiveThreadHandle,
        thread_history: &InitialHistory,
        rollout_path: &Path,
        resume_source_thread: Option<StoredThread>,
        include_turns: bool,
    ) -> std::result::Result<Thread, String> {
        let config_snapshot = codex_thread.config_snapshot().await;
        let session_id = codex_thread.session_configured().session_id.to_string();
        let thread = match thread_history {
            InitialHistory::Resumed(resumed) => {
                let fallback_provider = config_snapshot.model_provider_id.as_str();
                if let Some(stored_thread) = resume_source_thread {
                    Ok(thread_from_stored_thread(
                        StoredThread {
                            history: None,
                            ..stored_thread
                        },
                        fallback_provider,
                        &self.config.cwd,
                    )
                    .0)
                } else {
                    match self
                        .thread_store
                        .read_thread(StoreReadThreadParams {
                            thread_id: resumed.conversation_id,
                            include_archived: true,
                            include_history: false,
                        })
                        .await
                    {
                        Ok(stored_thread) => Ok(thread_from_stored_thread(
                            stored_thread,
                            fallback_provider,
                            &self.config.cwd,
                        )
                        .0),
                        Err(read_err) => {
                            Err(format!("failed to read thread from store: {read_err}"))
                        }
                    }
                }
            }
            InitialHistory::Forked(items) => {
                let mut thread = build_thread_from_snapshot(
                    thread_id,
                    session_id.clone(),
                    &config_snapshot,
                    Some(rollout_path.into()),
                );
                thread.preview = preview_from_rollout_items(items);
                Ok(thread)
            }
            InitialHistory::New | InitialHistory::Cleared => Err(format!(
                "failed to build resume response for thread {thread_id}: initial history missing"
            )),
        };
        let mut thread = thread?;
        thread.id = thread_id.to_string();
        thread.session_id = session_id;
        thread.path = Some(rollout_path.to_path_buf());
        let history_items = thread_history.get_rollout_items();
        apply_thread_usage_from_rollout_items(&mut thread, history_items.as_slice());
        if let Some(token_usage) = codex_thread.token_usage_info().await.map(Into::into) {
            thread.token_usage = Some(token_usage);
        }
        if thread.context_usage.is_none() {
            thread.context_usage = Some(
                super::context_usage_replay::thread_context_usage_from_rollout_or_conversation(
                    codex_thread,
                    history_items.as_slice(),
                )
                .await
                .into(),
            );
        }
        if include_turns {
            populate_thread_turns_from_history(
                &mut thread,
                history_items.as_slice(),
                /*active_turn*/ None,
            );
        }
        self.attach_thread_name(thread_id, &mut thread).await;
        Ok(thread)
    }

    pub(super) async fn attach_thread_name(&self, thread_id: ThreadId, thread: &mut Thread) {
        if let Ok(stored_thread) = self
            .thread_store
            .read_thread(StoreReadThreadParams {
                thread_id,
                include_archived: true,
                include_history: false,
            })
            .await
            && let Some(title) = stored_thread.name.as_deref().map(str::trim)
            && !title.is_empty()
            && stored_thread.preview.trim() != title
        {
            set_thread_name_from_title(thread, title.to_string());
        }
    }

    pub(super) async fn thread_fork_inner(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadForkParams,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
    ) -> Result<(), JSONRPCErrorError> {
        let ThreadForkParams {
            thread_id,
            path,
            model,
            model_provider,
            service_tier,
            cwd,
            runtime_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox,
            permissions,
            config: cli_overrides,
            base_instructions,
            developer_instructions,
            ephemeral,
            thread_source,
            exclude_turns,
            persist_extended_history,
        } = params;
        let include_turns = !exclude_turns;
        if sandbox.is_some() && permissions.is_some() {
            return Err(invalid_request(
                "`permissions` cannot be combined with `sandbox`",
            ));
        }
        if persist_extended_history {
            self.send_persist_extended_history_deprecation_notice(request_id.connection_id)
                .await;
        }

        let source_thread = self
            .read_stored_thread_for_resume(&thread_id, path.as_ref(), /*include_history*/ true)
            .await?;
        let source_thread_id = source_thread.thread_id;
        let history_items = source_thread
            .history
            .as_ref()
            .map(|history| history.items.clone())
            .ok_or_else(|| {
                internal_error(format!(
                    "thread {source_thread_id} did not include persisted history"
                ))
            })?;
        let history_cwd = Some(source_thread.cwd.clone());

        // Persist Windows sandbox mode.
        let mut cli_overrides = cli_overrides.unwrap_or_default();
        if cfg!(windows) {
            match WindowsSandboxLevel::from_config(&self.config) {
                WindowsSandboxLevel::Elevated => {
                    cli_overrides
                        .insert("windows.sandbox".to_string(), serde_json::json!("elevated"));
                }
                WindowsSandboxLevel::RestrictedToken => {
                    cli_overrides.insert(
                        "windows.sandbox".to_string(),
                        serde_json::json!("unelevated"),
                    );
                }
                WindowsSandboxLevel::Disabled => {}
            }
        }
        let request_overrides = if cli_overrides.is_empty() {
            None
        } else {
            Some(cli_overrides)
        };
        let mut typesafe_overrides = self.build_thread_config_overrides(
            model,
            model_provider,
            service_tier,
            cwd,
            runtime_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox,
            permissions,
            base_instructions,
            developer_instructions,
            /*personality*/ None,
        );
        typesafe_overrides.ephemeral = ephemeral.then_some(true);
        // Derive a Config using the same logic as new conversation, honoring overrides if provided.
        let config = self
            .config_manager
            .load_for_cwd(request_overrides, typesafe_overrides, history_cwd)
            .await
            .map_err(|err| config_load_error(&err))?;

        let fallback_model_provider = config.model_provider_id.clone();
        let instruction_sources = Self::instruction_sources_from_config(&config).await;

        let ThreadProcessorNewThread {
            thread_id,
            thread: forked_thread,
            session_configured,
            ..
        } = self
            .thread_runtime
            .fork_thread_from_history(
                ForkSnapshot::Interrupted,
                config,
                InitialHistory::Resumed(ResumedHistory {
                    conversation_id: source_thread_id,
                    history: history_items.clone(),
                    rollout_path: source_thread.rollout_path.clone(),
                }),
                thread_source.map(Into::into),
                /*persist_extended_history*/ false,
                self.request_trace_context(&request_id).await,
            )
            .await
            .map_err(|err| match err {
                CodexErr::Io(_) | CodexErr::Json(_) => {
                    invalid_request(format!("failed to load thread {source_thread_id}: {err}"))
                }
                CodexErr::InvalidRequest(message) => invalid_request(message),
                err => internal_error(format!("error forking thread: {err}")),
            })?;

        self.set_app_server_client_info(
            thread_id,
            app_server_client_name,
            app_server_client_version,
        )
        .await?;

        // Auto-attach a conversation listener when forking a thread.
        log_listener_attach_result(
            self.ensure_conversation_listener(thread_id, request_id.connection_id)
                .await,
            thread_id,
            request_id.connection_id,
            "thread",
        );

        // Persistent forks materialize their own rollout immediately. Ephemeral forks stay
        // pathless, so they rebuild their visible history from the copied source history instead.
        let mut thread = if session_configured.rollout_path.is_some() {
            let stored_thread = self
                .read_stored_thread_for_new_fork(thread_id, include_turns)
                .await?;
            self.stored_thread_to_api_thread(
                stored_thread,
                fallback_model_provider.as_str(),
                include_turns,
            )
        } else {
            let config_snapshot = forked_thread.config_snapshot().await;
            // forked thread names do not inherit the source thread name
            let mut thread = build_thread_from_snapshot(
                thread_id,
                session_configured.session_id.to_string(),
                &config_snapshot,
                /*path*/ None,
            );
            thread.preview = preview_from_rollout_items(&history_items);
            thread.forked_from_id = Some(source_thread_id.to_string());
            if include_turns {
                populate_thread_turns_from_history(
                    &mut thread,
                    &history_items,
                    /*active_turn*/ None,
                );
            }
            thread
        };
        thread.session_id = session_configured.session_id.to_string();
        thread.thread_source = forked_thread
            .config_snapshot()
            .await
            .thread_source
            .map(Into::into);
        if let Some(token_usage) = forked_thread.token_usage_info().await.map(Into::into) {
            thread.token_usage = Some(token_usage);
        }
        if thread.context_usage.is_none() {
            thread.context_usage = Some(
                super::context_usage_replay::thread_context_usage_from_rollout_or_conversation(
                    forked_thread.as_ref(),
                    history_items.as_slice(),
                )
                .await
                .into(),
            );
        }

        self.thread_watch_manager
            .upsert_thread_silently(thread.clone())
            .await;

        thread.lifecycle_status = resolve_thread_status(
            self.thread_watch_manager
                .loaded_status_for_thread(&thread.id)
                .await,
            /*has_in_progress_turn*/ false,
        );
        let config_snapshot = forked_thread.config_snapshot().await;
        let sandbox = thread_response_sandbox_policy(
            &config_snapshot.permission_profile,
            config_snapshot.cwd.as_path(),
        );
        let active_permission_profile =
            thread_response_active_permission_profile(config_snapshot.active_permission_profile);

        let response = ThreadForkResponse {
            thread: thread.clone(),
            model: session_configured.model,
            model_provider: session_configured.model_provider_id,
            service_tier: session_configured.service_tier,
            cwd: session_configured.cwd,
            runtime_workspace_roots: config_snapshot.workspace_roots,
            instruction_sources,
            approval_policy: session_configured.approval_policy.into(),
            approvals_reviewer: session_configured.approvals_reviewer.into(),
            sandbox,
            permission_profile: Some(config_snapshot.permission_profile.into()),
            active_permission_profile,
            reasoning_effort: session_configured.reasoning_effort,
        };

        let notif = thread_started_notification(thread);
        let connection_id = request_id.connection_id;
        let token_usage_thread = include_turns.then(|| response.thread.clone());
        self.outgoing.send_response(request_id, response).await;
        // `excludeTurns` is the cheap fork path, so skip restored usage replay
        // instead of rebuilding history only to attribute a historical update.
        if let Some(token_usage_thread) = token_usage_thread {
            let token_usage_turn_id = latest_token_usage_turn_id_from_rollout_items(
                &history_items,
                token_usage_thread.turns.as_slice(),
            );
            // Mirror the resume contract for forks: the new thread is usable as soon
            // as the response arrives, so restored usage must follow immediately.
            send_thread_token_usage_update_to_connection(
                &self.outgoing,
                connection_id,
                thread_id,
                &token_usage_thread,
                forked_thread.as_ref(),
                token_usage_turn_id,
            )
            .await;
            send_thread_context_usage_update_to_connection(
                &self.outgoing,
                connection_id,
                thread_id,
                &token_usage_thread,
                forked_thread.as_ref(),
                history_items.as_slice(),
            )
            .await;
        }

        self.outgoing
            .send_server_notification(ServerNotification::ThreadStarted(notif))
            .await;
        Ok(())
    }
}
