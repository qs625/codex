use super::*;
use app_server_protocol::ItemCompletedNotification;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadLifecycleActiveFlag;
use app_server_protocol::ThreadLifecycleFinalStatus;
use app_server_protocol::ThreadStatusChangedNotification;
use app_server_protocol::Turn;
use app_server_protocol::TurnCompletedNotification;
use app_server_protocol::TurnItemsView;
use app_server_protocol::TurnStartedNotification;
use app_server_protocol::TurnStatus;
use app_server_protocol::UserInput;
use app_server_protocol::is_legacy_structured_assistant_message_text;
use app_server_protocol::item_event_to_server_notification;
use codex_agent_runtime::AgentMetadata;
use protocol::AgentPath;
use protocol::protocol::AgentStatus;
use protocol::protocol::EventMsg;
use thread_service_api::ExternalRootThreadInputRoute;

pub(in crate::request_processors) fn unsupported_external_root_active_op(
    method: &str,
    provider: &str,
) -> JSONRPCErrorError {
    invalid_request(format!(
        "thread provider '{provider}' does not support {method}; external root threads do not support this native-only operation yet"
    ))
}

#[derive(Debug, Clone)]
pub(super) struct ThreadStartAgent {
    pub(super) agent_path: Option<AgentPath>,
    pub(super) agent_role: Option<String>,
}

pub(super) fn parse_thread_start_agent(
    task_name: Option<String>,
    agent_type: Option<String>,
) -> Result<Option<ThreadStartAgent>, JSONRPCErrorError> {
    let task_name = task_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let agent_role = agent_type
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(task_name) = task_name {
        let agent_path = if task_name.starts_with('/') {
            AgentPath::try_from(task_name.as_str())
        } else {
            AgentPath::derive(None, task_name.as_str())
        }
        .map_err(|err| invalid_request(format!("invalid taskName `{task_name}`: {err}")))?;
        return Ok(Some(ThreadStartAgent {
            agent_path: Some(agent_path),
            agent_role,
        }));
    }

    Ok(agent_role.map(|agent_role| ThreadStartAgent {
        agent_path: None,
        agent_role: Some(agent_role),
    }))
}

fn external_root_thread_start_request(
    config: Config,
    provider: thread_service_api::ExternalRootThreadProvider,
    agent_metadata: Option<thread_service_api::ExternalRootAgentMetadata>,
) -> thread_service_api::ExternalRootThreadStartRequest {
    thread_service_api::ExternalRootThreadStartRequest {
        startup_config: thread_service_api::ExternalRootThreadStartupConfig {
            cwd: config.cwd,
            workspace_roots: config.workspace_roots,
            agent_max_threads: config.agent_max_threads,
            agent_roles: config.agent_roles,
            model: config.model.unwrap_or_default(),
            model_provider_id: config.model_provider_id,
            service_tier: config.service_tier,
            approval_policy: config.permissions.approval_policy.value(),
            approvals_reviewer: config.approvals_reviewer,
            permission_profile: config.permissions.effective_permission_profile(),
            active_permission_profile: config.permissions.active_permission_profile(),
            reasoning_effort: config.model_reasoning_effort,
            personality: config.personality,
            features: config.features.get().clone(),
            generate_memories: config.memories.generate_memories,
            default_wait_timeout_ms: config.multi_agent_v2.default_wait_timeout_ms,
            max_wait_timeout_ms: config.multi_agent_v2.max_wait_timeout_ms,
        },
        provider,
        agent_metadata,
    }
}

fn external_root_agent_metadata(
    thread_start_agent: Option<ThreadStartAgent>,
    provider: thread_service_api::ExternalRootThreadProvider,
) -> Option<thread_service_api::ExternalRootAgentMetadata> {
    thread_start_agent.and_then(|agent| {
        agent.agent_path.map(|agent_path| {
            thread_service_api::ExternalRootAgentMetadata {
                agent_path,
                agent_nickname: Some(provider.provider_id().to_string()),
                agent_role: Some(provider.provider_id().to_string()),
            }
        })
    })
}

pub(super) fn thread_lifecycle_status_from_agent_status(
    status: &AgentStatus,
) -> ThreadLifecycleStatus {
    match status {
        AgentStatus::PendingInit => ThreadLifecycleStatus::Initializing,
        AgentStatus::Running => ThreadLifecycleStatus::Active {
            active_flags: vec![ThreadLifecycleActiveFlag::Running],
        },
        AgentStatus::Completed(message) => ThreadLifecycleStatus::completed(message.clone()),
        AgentStatus::Errored(message) => ThreadLifecycleStatus::errored(Some(message.clone())),
        AgentStatus::Interrupted => ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Interrupted,
        },
        AgentStatus::Shutdown => ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Shutdown,
        },
        AgentStatus::NotFound => ThreadLifecycleStatus::NotLoaded,
    }
}

fn thread_status_changed_lifecycle_status(
    authoritative_status: Option<&AgentStatus>,
    live_agent_status: Option<&AgentStatus>,
    watch_status: ThreadLifecycleStatus,
) -> ThreadLifecycleStatus {
    authoritative_status
        .or(live_agent_status)
        .map(thread_lifecycle_status_from_agent_status)
        .unwrap_or_else(|| resolve_thread_status(watch_status, /*has_in_progress_turn*/ false))
}

impl ThreadRequestProcessor {
    pub(super) async fn instruction_sources_from_config(config: &Config) -> Vec<AbsolutePathBuf> {
        thread_service::AgentsMdManager::new(config)
            .instruction_sources(LOCAL_FS.as_ref())
            .await
    }

    pub(super) async fn acquire_thread_list_state_permit(
        &self,
    ) -> Result<SemaphorePermit<'_>, JSONRPCErrorError> {
        self.thread_list_state_permit
            .acquire()
            .await
            .map_err(|err| {
                internal_error(format!("failed to acquire thread list state permit: {err}"))
            })
    }

    pub(super) async fn set_app_server_client_info(
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

    pub(super) async fn finalize_thread_teardown(&self, thread_id: ThreadId) {
        self.pending_thread_unloads.lock().await.remove(&thread_id);
        self.outgoing
            .cancel_requests_for_thread(thread_id, /*error*/ None)
            .await;
        self.thread_state_manager
            .remove_thread_state(thread_id)
            .await;
        self.thread_watch_manager
            .remove_thread(&thread_id.to_string())
            .await;
    }

    pub(super) async fn thread_unsubscribe_response_inner(
        &self,
        params: ThreadUnsubscribeParams,
        connection_id: ConnectionId,
    ) -> Result<ThreadUnsubscribeResponse, JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        if !self
            .live_thread_inspection
            .is_live_thread_loaded(thread_id)
            .await
        {
            self.finalize_thread_teardown(thread_id).await;
            return Ok(ThreadUnsubscribeResponse {
                status: ThreadUnsubscribeStatus::NotLoaded,
            });
        };

        let was_subscribed = self
            .thread_state_manager
            .unsubscribe_connection_from_thread(thread_id, connection_id)
            .await;

        let status = if was_subscribed {
            ThreadUnsubscribeStatus::Unsubscribed
        } else {
            ThreadUnsubscribeStatus::NotSubscribed
        };
        Ok(ThreadUnsubscribeResponse { status })
    }

    pub(super) async fn prepare_thread_for_archive(&self, thread_id: ThreadId) {
        if self
            .live_thread_inspection
            .is_live_thread_loaded(thread_id)
            .await
        {
            info!("thread {thread_id} was active; shutting down");
            match tokio::time::timeout(
                Duration::from_secs(10),
                self.thread_lifecycle_runtime
                    .shutdown_live_thread(thread_id),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(_)) => error!(
                    "failed to submit Shutdown to thread {thread_id}; proceeding with archive"
                ),
                Err(_) => warn!("thread {thread_id} shutdown timed out; proceeding with archive"),
            }
            self.thread_lifecycle_runtime
                .remove_live_thread(thread_id)
                .await;
        }
        self.finalize_thread_teardown(thread_id).await;
    }

    pub(super) fn listener_task_context(&self) -> ListenerTaskContext {
        ListenerTaskContext {
            live_thread_listener: Arc::clone(&self.live_thread_listener),
            live_thread_inspection: Arc::clone(&self.live_thread_inspection),
            live_thread_history: Arc::clone(&self.live_thread_history),
            thread_lifecycle_runtime: Arc::clone(&self.thread_lifecycle_runtime),
            live_thread_command: Arc::clone(&self.live_thread_command),
            live_thread_usage: Arc::clone(&self.live_thread_usage),
            live_thread_goal: Arc::clone(&self.live_thread_goal),
            live_thread_skill_watch: Arc::clone(&self.live_thread_skill_watch),
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

    pub(super) async fn ensure_conversation_listener(
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

    pub(crate) async fn drain_background_tasks(&self) {
        self.background_tasks.close();
        if tokio::time::timeout(Duration::from_secs(10), self.background_tasks.wait())
            .await
            .is_err()
        {
            warn!("timed out waiting for background tasks to shut down; proceeding");
        }
    }

    pub(crate) async fn clear_all_thread_listeners(&self) {
        self.thread_state_manager.clear_all_listeners().await;
    }

    pub(crate) async fn shutdown_threads(&self) {
        let report = self
            .thread_lifecycle_runtime
            .shutdown_all_threads_for_runtime_teardown_bounded(Duration::from_secs(10))
            .await;
        for thread_id in report.submit_failed {
            warn!("failed to shut down thread {thread_id}");
        }
        for thread_id in report.timed_out {
            warn!("timed out waiting for thread {thread_id} to shut down");
        }
    }

    pub(super) async fn request_trace_context(
        &self,
        request_id: &ConnectionRequestId,
    ) -> Option<protocol::protocol::W3cTraceContext> {
        self.outgoing.request_trace_context(request_id).await
    }

    pub(super) async fn send_persist_extended_history_deprecation_notice(
        &self,
        connection_id: ConnectionId,
    ) {
        self.outgoing
            .send_server_notification_to_connections(
                &[connection_id],
                ServerNotification::DeprecationNotice(DeprecationNoticeNotification {
                    summary: PERSIST_EXTENDED_HISTORY_DEPRECATION_SUMMARY.to_string(),
                    details: Some(PERSIST_EXTENDED_HISTORY_DEPRECATION_DETAILS.to_string()),
                }),
            )
            .await;
    }

    pub(crate) async fn emit_thread_started_notification_to_connections(
        &self,
        thread_id: ThreadId,
        connection_ids: &[ConnectionId],
    ) {
        if connection_ids.is_empty() {
            return;
        }
        let Ok(live_snapshot) = self
            .live_thread_inspection
            .live_thread_snapshot(thread_id)
            .await
        else {
            return;
        };
        let mut loaded_thread = build_thread_from_live_snapshot(thread_id, &live_snapshot);
        let watch_status = self
            .thread_watch_manager
            .loaded_status_for_thread(&loaded_thread.id)
            .await;
        let agent_status = self
            .thread_lifecycle_runtime
            .live_thread_agent_status(thread_id)
            .await
            .ok();
        loaded_thread.lifecycle_status = agent_status
            .as_ref()
            .map(thread_lifecycle_status_from_agent_status)
            .unwrap_or_else(|| {
                resolve_thread_status(watch_status, /*has_in_progress_turn*/ false)
            });
        let notif = thread_started_notification(loaded_thread);
        self.outgoing
            .send_server_notification_to_connections(
                connection_ids,
                ServerNotification::ThreadStarted(notif),
            )
            .await;
    }

    pub(crate) async fn emit_thread_live_event_notification_to_connections(
        &self,
        thread_id: ThreadId,
        turn_id: String,
        event: EventMsg,
        connection_ids: &[ConnectionId],
    ) {
        if connection_ids.is_empty() {
            return;
        }
        let thread_id_string = thread_id.to_string();
        let notification = match event {
            EventMsg::TurnStarted(payload) => {
                ServerNotification::TurnStarted(TurnStartedNotification {
                    thread_id: thread_id_string,
                    turn: Turn {
                        id: payload.turn_id,
                        items: Vec::new(),
                        items_view: TurnItemsView::NotLoaded,
                        status: TurnStatus::InProgress,
                        error: None,
                        started_at: payload.started_at,
                        completed_at: None,
                        duration_ms: None,
                    },
                })
            }
            EventMsg::TurnComplete(payload) => {
                ServerNotification::TurnCompleted(TurnCompletedNotification {
                    thread_id: thread_id_string,
                    turn: Turn {
                        id: payload.turn_id,
                        items: Vec::new(),
                        items_view: TurnItemsView::NotLoaded,
                        status: TurnStatus::Completed,
                        error: None,
                        started_at: None,
                        completed_at: payload.completed_at,
                        duration_ms: payload.duration_ms,
                    },
                })
            }
            EventMsg::TurnAborted(payload) => {
                let event_turn_id = payload.turn_id.unwrap_or(turn_id);
                ServerNotification::TurnCompleted(TurnCompletedNotification {
                    thread_id: thread_id_string,
                    turn: Turn {
                        id: event_turn_id,
                        items: Vec::new(),
                        items_view: TurnItemsView::NotLoaded,
                        status: TurnStatus::Interrupted,
                        error: None,
                        started_at: None,
                        completed_at: payload.completed_at,
                        duration_ms: payload.duration_ms,
                    },
                })
            }
            EventMsg::ExternalTerminalStatus(payload) => {
                let (status, error) = match payload.status {
                    protocol::protocol::ExternalTerminalStatus::Errored => (
                        TurnStatus::Failed,
                        Some(TurnError {
                            message: payload.message.unwrap_or_default(),
                            codex_error_info: None,
                            additional_details: None,
                        }),
                    ),
                    protocol::protocol::ExternalTerminalStatus::Shutdown => {
                        (TurnStatus::Completed, None)
                    }
                };
                ServerNotification::TurnCompleted(TurnCompletedNotification {
                    thread_id: thread_id_string,
                    turn: Turn {
                        id: payload.turn_id,
                        items: Vec::new(),
                        items_view: TurnItemsView::NotLoaded,
                        status,
                        error,
                        started_at: None,
                        completed_at: Some(payload.terminal_at_ms / 1000),
                        duration_ms: None,
                    },
                })
            }
            EventMsg::UserMessage(payload) => {
                let mut content = Vec::new();
                for skill in payload.skills {
                    content.push(UserInput::Skill {
                        name: skill.name,
                        path: skill.path,
                    });
                }
                if !payload.message.trim().is_empty() {
                    content.push(UserInput::Text {
                        text: payload.message,
                        text_elements: payload
                            .text_elements
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                    });
                }
                if let Some(images) = payload.images {
                    for image in images {
                        content.push(UserInput::Image { url: image });
                    }
                }
                for path in payload.local_images {
                    content.push(UserInput::LocalImage { path });
                }
                ServerNotification::ItemCompleted(ItemCompletedNotification {
                    thread_id: thread_id_string,
                    turn_id,
                    item: ThreadItem::UserMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        content,
                    },
                    completed_at_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            EventMsg::AgentMessage(payload) => {
                if !should_display_agent_message_event(&payload.message) {
                    return;
                }
                ServerNotification::ItemCompleted(ItemCompletedNotification {
                    thread_id: thread_id_string,
                    turn_id,
                    item: ThreadItem::AgentMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        text: payload.message,
                        phase: payload.phase,
                        memory_citation: payload.memory_citation.map(Into::into),
                    },
                    completed_at_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            EventMsg::AgentReasoning(payload) => {
                ServerNotification::ItemCompleted(ItemCompletedNotification {
                    thread_id: thread_id_string,
                    turn_id,
                    item: ThreadItem::Reasoning {
                        id: uuid::Uuid::new_v4().to_string(),
                        summary: vec![payload.text],
                        content: Vec::new(),
                    },
                    completed_at_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            EventMsg::AgentReasoningRawContent(payload) => {
                ServerNotification::ItemCompleted(ItemCompletedNotification {
                    thread_id: thread_id_string,
                    turn_id,
                    item: ThreadItem::Reasoning {
                        id: uuid::Uuid::new_v4().to_string(),
                        summary: Vec::new(),
                        content: vec![payload.text],
                    },
                    completed_at_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            EventMsg::Error(_) => {
                return;
            }
            other => {
                let Some(notification) =
                    item_event_to_server_notification(other, &thread_id_string, &turn_id)
                else {
                    return;
                };
                notification
            }
        };
        self.outgoing
            .send_server_notification_to_connections(connection_ids, notification)
            .await;
    }

    pub(crate) async fn emit_thread_status_changed_notification_to_connections(
        &self,
        thread_id: ThreadId,
        authoritative_status: Option<AgentStatus>,
        connection_ids: &[ConnectionId],
    ) {
        if connection_ids.is_empty() {
            return;
        }
        let thread_id_string = thread_id.to_string();
        let lifecycle_status = if let Some(authoritative_status) = authoritative_status.as_ref() {
            thread_lifecycle_status_from_agent_status(authoritative_status)
        } else {
            let watch_status = self
                .thread_watch_manager
                .loaded_status_for_thread(&thread_id_string)
                .await;
            let live_agent_status = self
                .thread_lifecycle_runtime
                .live_thread_agent_status(thread_id)
                .await
                .ok();
            thread_status_changed_lifecycle_status(None, live_agent_status.as_ref(), watch_status)
        };
        self.outgoing
            .send_server_notification_to_connections(
                connection_ids,
                ServerNotification::ThreadStatusChanged(ThreadStatusChangedNotification {
                    thread_id: thread_id_string,
                    lifecycle_status,
                }),
            )
            .await;
    }

    pub(super) async fn submit_core_op(
        &self,
        request_id: &ConnectionRequestId,
        thread_id: ThreadId,
        method: &'static str,
        op: Op,
        failure_message: &str,
    ) -> Result<String, JSONRPCErrorError> {
        match self
            .external_root_thread_runtime
            .external_root_thread_input_route(thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to inspect thread provider: {err}")))?
        {
            ExternalRootThreadInputRoute::LiveExternalRoot { provider, .. } => {
                return Err(unsupported_external_root_active_op(
                    method,
                    provider.provider_id(),
                ));
            }
            ExternalRootThreadInputRoute::UnsupportedPersistedExternalRoot {
                provider_id, ..
            } => {
                return Err(unsupported_external_root_active_op(
                    method,
                    provider_id.as_str(),
                ));
            }
            ExternalRootThreadInputRoute::NativeRequired => {}
        }

        if !self
            .live_thread_inspection
            .is_live_thread_loaded(thread_id)
            .await
        {
            return Err(invalid_request(format!("thread not found: {thread_id}")));
        }
        self.live_thread_command
            .submit_live_thread_op_with_trace(
                thread_id,
                op,
                self.request_trace_context(request_id).await,
            )
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(_) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                err => internal_error(format!("{failure_message}: {err}")),
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn thread_start_task(
        listener_task_context: ListenerTaskContext,
        external_root_provider: Option<thread_service_api::ExternalRootThreadProvider>,
        native_thread_creation: Arc<dyn NativeThreadCreationRuntime>,
        external_root_thread_runtime: Arc<dyn thread_service_api::ExternalRootThreadRuntime>,
        environment_runtime: Arc<dyn NativeThreadEnvironmentRuntime>,
        live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
        thread_store: Arc<dyn ThreadStore>,
        config_manager: ConfigManager,
        request_id: ConnectionRequestId,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
        config_overrides: Option<HashMap<String, serde_json::Value>>,
        typesafe_overrides: ConfigOverrides,
        dynamic_tools: Option<Vec<ApiDynamicToolSpec>>,
        thread_start_agent: Option<ThreadStartAgent>,
        session_start_source: Option<app_server_protocol::ThreadStartSource>,
        thread_source: Option<protocol::protocol::ThreadSource>,
        environments: Option<Vec<TurnEnvironmentSelection>>,
        service_name: Option<String>,
        request_trace: Option<W3cTraceContext>,
    ) -> Result<(), JSONRPCErrorError> {
        let thread_start_started_at = std::time::Instant::now();
        let requested_cwd = typesafe_overrides.cwd.clone();
        let mut config = config_manager
            .load_with_overrides(config_overrides.clone(), typesafe_overrides.clone())
            .await
            .map_err(|err| config_load_error(&err))?;

        // The user may have requested WorkspaceWrite or DangerFullAccess via
        // the command line, though in the process of deriving the Config, it
        // could be downgraded to ReadOnly (perhaps there is no sandbox
        // available on Windows or the enterprise config disallows it). The cwd
        // should still be considered "trusted" in this case.
        let requested_permissions_trust_project =
            requested_permissions_trust_project(&typesafe_overrides, config.cwd.as_path());
        let effective_permissions_trust_project = permission_profile_trusts_project(
            &config.permissions.effective_permission_profile(),
            config.cwd.as_path(),
        );

        if requested_cwd.is_some()
            && config.active_project.trust_level.is_none()
            && (requested_permissions_trust_project || effective_permissions_trust_project)
        {
            let trust_target = resolve_root_git_project_for_trust(LOCAL_FS.as_ref(), &config.cwd)
                .await
                .unwrap_or_else(|| config.cwd.clone());
            let current_cli_overrides = config_manager.current_cli_overrides();
            let cli_overrides_with_trust;
            let cli_overrides_for_reload = if let Err(err) =
                thread_service::config::set_project_trust_level(
                    &listener_task_context.codex_home,
                    trust_target.as_path(),
                    TrustLevel::Trusted,
                ) {
                warn!(
                    "failed to persist trusted project state for {}; continuing with in-memory trust for this thread: {err}",
                    trust_target.display()
                );
                let mut project = toml::map::Map::new();
                project.insert(
                    "trust_level".to_string(),
                    TomlValue::String("trusted".to_string()),
                );
                let mut projects = toml::map::Map::new();
                projects.insert(
                    project_trust_key(trust_target.as_path()),
                    TomlValue::Table(project),
                );
                cli_overrides_with_trust = current_cli_overrides
                    .iter()
                    .cloned()
                    .chain(std::iter::once((
                        "projects".to_string(),
                        TomlValue::Table(projects),
                    )))
                    .collect::<Vec<_>>();
                cli_overrides_with_trust.as_slice()
            } else {
                current_cli_overrides.as_slice()
            };

            config = config_manager
                .load_with_cli_overrides(
                    cli_overrides_for_reload,
                    config_overrides,
                    typesafe_overrides,
                    /*fallback_cwd*/ None,
                )
                .await
                .map_err(|err| config_load_error(&err))?;
        }
        if let Some(agent_role) = thread_start_agent
            .as_ref()
            .and_then(|agent| agent.agent_role.as_deref())
        {
            codex_agent_runtime::apply_role_to_config(&mut config, Some(agent_role))
                .await
                .map_err(invalid_request)?;
        }

        let instruction_sources = Self::instruction_sources_from_config(&config).await;
        if let Some(provider) = external_root_provider {
            let agent_metadata = external_root_agent_metadata(thread_start_agent, provider);
            return Self::external_root_thread_start_task(
                listener_task_context,
                external_root_thread_runtime,
                thread_store,
                request_id,
                instruction_sources,
                config,
                provider,
                agent_metadata,
            )
            .await;
        }
        let environments = environments
            .unwrap_or_else(|| environment_runtime.default_environment_selections(&config.cwd));
        let dynamic_tools = dynamic_tools.unwrap_or_default();
        let core_dynamic_tools = if dynamic_tools.is_empty() {
            Vec::new()
        } else {
            validate_dynamic_tools(&dynamic_tools).map_err(invalid_request)?;
            dynamic_tools
                .into_iter()
                .map(|tool| CoreDynamicToolSpec {
                    namespace: tool.namespace,
                    name: tool.name,
                    description: tool.description,
                    input_schema: tool.input_schema,
                    defer_loading: tool.defer_loading,
                })
                .collect()
        };
        let core_dynamic_tool_count = core_dynamic_tools.len();
        let create_thread_started_at = std::time::Instant::now();
        let new_thread = native_thread_creation
            .start_thread_with_options(StartThreadOptions {
                config,
                initial_history: match session_start_source
                    .unwrap_or(app_server_protocol::ThreadStartSource::Startup)
                {
                    app_server_protocol::ThreadStartSource::Startup => InitialHistory::New,
                    app_server_protocol::ThreadStartSource::Clear => InitialHistory::Cleared,
                },
                session_source: None,
                agent_metadata: thread_start_agent
                    .clone()
                    .map(|agent| AgentMetadata {
                        agent_path: agent.agent_path,
                        agent_role: agent.agent_role,
                        ..Default::default()
                    })
                    .filter(|metadata| {
                        metadata.agent_path.is_some() || metadata.agent_role.is_some()
                    }),
                thread_source,
                dynamic_tools: core_dynamic_tools,
                persist_extended_history: false,
                metrics_service_name: service_name,
                parent_trace: request_trace,
                environments,
            })
            .instrument(tracing::info_span!(
                "app_server.thread_start.create_thread",
                otel.name = "app_server.thread_start.create_thread",
                thread_start.dynamic_tool_count = core_dynamic_tool_count,
                thread_start.persist_extended_history = false,
            ))
            .await
            .map_err(thread_start_create_error)?;
        let ThreadProcessorNewThread {
            thread_id,
            thread: created_thread,
            session_configured,
            ..
        } = thread_processor_new_thread(new_thread);
        created_thread.record_startup_phase(
            "thread_start_create_thread",
            create_thread_started_at.elapsed(),
            Some("ready"),
        );

        let mcp_elicitations_auto_deny = xcode_26_4_mcp_elicitations_auto_deny(
            app_server_client_name.as_deref(),
            app_server_client_version.as_deref(),
        );
        live_thread_command
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
            })?;

        let config_snapshot = listener_task_context
            .live_thread_inspection
            .live_thread_config_snapshot(thread_id)
            .instrument(tracing::info_span!(
                "app_server.thread_start.config_snapshot",
                otel.name = "app_server.thread_start.config_snapshot",
            ))
            .await
            .map_err(thread_start_create_error)?;
        let mut thread = build_thread_from_snapshot(
            thread_id,
            session_configured.session_id.to_string(),
            &config_snapshot,
            session_configured.rollout_path.clone(),
        );
        if let Some(agent) = thread_start_agent.as_ref() {
            if agent.agent_role.is_some() {
                thread.agent_role = agent.agent_role.clone();
            }
            if let Some(agent_path) = agent.agent_path.as_ref() {
                thread.agent_path = Some(agent_path.to_string());
            }
        }

        // Auto-attach a thread listener when starting a thread.
        log_listener_attach_result(
            super::thread_lifecycle::ensure_conversation_listener(
                listener_task_context.clone(),
                thread_id,
                request_id.connection_id,
            )
            .instrument(tracing::info_span!(
                "app_server.thread_start.attach_listener",
                otel.name = "app_server.thread_start.attach_listener",
            ))
            .await,
            thread_id,
            request_id.connection_id,
            "thread",
        );
        listener_task_context
            .thread_watch_manager
            .upsert_thread_silently(thread.clone())
            .instrument(tracing::info_span!(
                "app_server.thread_start.upsert_thread",
                otel.name = "app_server.thread_start.upsert_thread",
            ))
            .await;

        thread.lifecycle_status = resolve_thread_status(
            listener_task_context
                .thread_watch_manager
                .loaded_status_for_thread(&thread.id)
                .instrument(tracing::info_span!(
                    "app_server.thread_start.resolve_status",
                    otel.name = "app_server.thread_start.resolve_status",
                ))
                .await,
            /*has_in_progress_turn*/ false,
        );
        if !config_snapshot.ephemeral {
            let history_items = read_thread_history_items(thread_store.as_ref(), thread_id)
                .instrument(tracing::info_span!(
                    "app_server.thread_start.read_initial_history",
                    otel.name = "app_server.thread_start.read_initial_history",
                ))
                .await
                .map_err(|err| {
                    internal_error(format!(
                        "failed to read initial thread history for thread id {thread_id}: {err}"
                    ))
                })?;
            thread.turns = build_api_turns_from_rollout_items(&history_items);
        }

        let sandbox = thread_response_sandbox_policy(
            &config_snapshot.permission_profile,
            config_snapshot.cwd.as_path(),
        );
        let active_permission_profile =
            thread_response_active_permission_profile(config_snapshot.active_permission_profile);

        let response = ThreadStartResponse {
            thread: thread.clone(),
            model: config_snapshot.model,
            model_provider: config_snapshot.model_provider_id,
            service_tier: config_snapshot.service_tier,
            cwd: config_snapshot.cwd,
            runtime_workspace_roots: config_snapshot.workspace_roots,
            instruction_sources,
            approval_policy: config_snapshot.approval_policy.into(),
            approvals_reviewer: config_snapshot.approvals_reviewer.into(),
            sandbox,
            permission_profile: Some(config_snapshot.permission_profile.into()),
            active_permission_profile,
            reasoning_effort: config_snapshot.reasoning_effort,
        };
        let notif = thread_started_notification_with_turns(thread.clone());
        listener_task_context
            .outgoing
            .send_response(request_id, response)
            .instrument(tracing::info_span!(
                "app_server.thread_start.send_response",
                otel.name = "app_server.thread_start.send_response",
            ))
            .await;

        listener_task_context
            .outgoing
            .send_server_notification(ServerNotification::ThreadStarted(notif))
            .instrument(tracing::info_span!(
                "app_server.thread_start.notify_started",
                otel.name = "app_server.thread_start.notify_started",
            ))
            .await;
        created_thread.record_startup_phase(
            "thread_start_total",
            thread_start_started_at.elapsed(),
            Some("ready"),
        );
        Ok(())
    }

    async fn external_root_thread_start_task(
        listener_task_context: ListenerTaskContext,
        external_root_thread_runtime: Arc<dyn thread_service_api::ExternalRootThreadRuntime>,
        thread_store: Arc<dyn ThreadStore>,
        request_id: ConnectionRequestId,
        instruction_sources: Vec<AbsolutePathBuf>,
        config: Config,
        provider: thread_service_api::ExternalRootThreadProvider,
        agent_metadata: Option<thread_service_api::ExternalRootAgentMetadata>,
    ) -> Result<(), JSONRPCErrorError> {
        let new_thread = external_root_thread_runtime
            .start_external_root_thread(external_root_thread_start_request(
                config,
                provider,
                agent_metadata,
            ))
            .instrument(tracing::info_span!(
                "app_server.thread_start.create_external_root_thread",
                otel.name = "app_server.thread_start.create_external_root_thread",
            ))
            .await
            .map_err(thread_start_create_error)?;
        let thread_service_api::ExternalRootThreadStartResult {
            thread_id,
            session_configured,
        } = new_thread;
        let result = Self::send_external_root_thread_start_response(
            listener_task_context.clone(),
            thread_store,
            request_id,
            instruction_sources,
            thread_id,
            session_configured,
        )
        .await;
        if result.is_err() {
            let _ = listener_task_context
                .thread_lifecycle_runtime
                .shutdown_live_thread(thread_id)
                .await;
        }
        result
    }

    async fn send_external_root_thread_start_response(
        listener_task_context: ListenerTaskContext,
        thread_store: Arc<dyn ThreadStore>,
        request_id: ConnectionRequestId,
        instruction_sources: Vec<AbsolutePathBuf>,
        thread_id: ThreadId,
        session_configured: SessionConfiguredEvent,
    ) -> Result<(), JSONRPCErrorError> {
        let config_snapshot = listener_task_context
            .live_thread_inspection
            .live_thread_config_snapshot(thread_id)
            .instrument(tracing::info_span!(
                "app_server.thread_start.external_config_snapshot",
                otel.name = "app_server.thread_start.external_config_snapshot",
            ))
            .await
            .map_err(thread_start_create_error)?;
        let mut thread = build_thread_from_snapshot(
            thread_id,
            session_configured.session_id.to_string(),
            &config_snapshot,
            session_configured.rollout_path.clone(),
        );
        listener_task_context
            .thread_watch_manager
            .upsert_thread_silently(thread.clone())
            .instrument(tracing::info_span!(
                "app_server.thread_start.upsert_external_thread",
                otel.name = "app_server.thread_start.upsert_external_thread",
            ))
            .await;

        log_listener_attach_result(
            super::thread_lifecycle::ensure_external_root_unload_watcher(
                listener_task_context.clone(),
                thread_id,
                request_id.connection_id,
            )
            .instrument(tracing::info_span!(
                "app_server.thread_start.attach_external_listener",
                otel.name = "app_server.thread_start.attach_external_listener",
            ))
            .await,
            thread_id,
            request_id.connection_id,
            "external root thread",
        );

        let watch_status = listener_task_context
            .thread_watch_manager
            .loaded_status_for_thread(&thread.id)
            .await;
        let agent_status = listener_task_context
            .thread_lifecycle_runtime
            .live_thread_agent_status(thread_id)
            .await
            .ok();
        thread.lifecycle_status = agent_status
            .as_ref()
            .map(thread_lifecycle_status_from_agent_status)
            .unwrap_or_else(|| {
                resolve_thread_status(watch_status, /*has_in_progress_turn*/ false)
            });
        if !config_snapshot.ephemeral {
            let history_items = match read_thread_history_items(thread_store.as_ref(), thread_id)
                .instrument(tracing::info_span!(
                    "app_server.thread_start.read_external_initial_history",
                    otel.name = "app_server.thread_start.read_external_initial_history",
                ))
                .await
            {
                Ok(history_items) => history_items,
                Err(ThreadStoreError::ThreadNotFound { .. }) => Vec::new(),
                Err(ThreadStoreError::InvalidRequest { message })
                    if message.contains("no rollout found for thread id") =>
                {
                    Vec::new()
                }
                Err(err) => {
                    return Err(internal_error(format!(
                        "failed to read initial thread history for thread id {thread_id}: {err}"
                    )));
                }
            };
            thread.turns = build_api_turns_from_rollout_items(&history_items);
        }

        let sandbox = thread_response_sandbox_policy(
            &config_snapshot.permission_profile,
            config_snapshot.cwd.as_path(),
        );
        let active_permission_profile =
            thread_response_active_permission_profile(config_snapshot.active_permission_profile);
        let response = ThreadStartResponse {
            thread: thread.clone(),
            model: config_snapshot.model,
            model_provider: config_snapshot.model_provider_id,
            service_tier: config_snapshot.service_tier,
            cwd: config_snapshot.cwd,
            runtime_workspace_roots: config_snapshot.workspace_roots,
            instruction_sources,
            approval_policy: config_snapshot.approval_policy.into(),
            approvals_reviewer: config_snapshot.approvals_reviewer.into(),
            sandbox,
            permission_profile: Some(config_snapshot.permission_profile.into()),
            active_permission_profile,
            reasoning_effort: config_snapshot.reasoning_effort,
        };
        let notif = thread_started_notification_with_turns(thread);
        listener_task_context
            .outgoing
            .send_response(request_id, response)
            .instrument(tracing::info_span!(
                "app_server.thread_start.send_external_response",
                otel.name = "app_server.thread_start.send_external_response",
            ))
            .await;
        listener_task_context
            .outgoing
            .send_server_notification(ServerNotification::ThreadStarted(notif))
            .instrument(tracing::info_span!(
                "app_server.thread_start.notify_external_started",
                otel.name = "app_server.thread_start.notify_external_started",
            ))
            .await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_thread_config_overrides(
        &self,
        model: Option<String>,
        model_provider: Option<String>,
        service_tier: Option<Option<String>>,
        cwd: Option<String>,
        runtime_workspace_roots: Option<Vec<PathBuf>>,
        approval_policy: Option<app_server_protocol::AskForApproval>,
        approvals_reviewer: Option<app_server_protocol::ApprovalsReviewer>,
        sandbox: Option<SandboxMode>,
        permissions: Option<PermissionProfileSelectionParams>,
        base_instructions: Option<String>,
        developer_instructions: Option<String>,
        personality: Option<Personality>,
    ) -> ConfigOverrides {
        let mut overrides = ConfigOverrides {
            model,
            model_provider,
            service_tier,
            cwd: cwd.map(PathBuf::from),
            workspace_roots: runtime_workspace_roots,
            approval_policy: approval_policy.map(app_server_protocol::AskForApproval::to_core),
            approvals_reviewer: approvals_reviewer
                .map(app_server_protocol::ApprovalsReviewer::to_core),
            sandbox_mode: sandbox.map(SandboxMode::to_core),
            codex_linux_sandbox_exe: self.arg0_paths.codex_linux_sandbox_exe.clone(),
            main_execve_wrapper_exe: self.arg0_paths.main_execve_wrapper_exe.clone(),
            base_instructions,
            developer_instructions,
            personality,
            ..Default::default()
        };
        apply_permission_profile_selection_to_config_overrides(&mut overrides, permissions);
        overrides
    }

    pub(super) fn parse_environment_selections(
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
            self.environment_runtime
                .validate_environment_selections(environment_selections)
                .map_err(|err| invalid_request(environment_selection_error_message(err)))?;
        }
        Ok(environment_selections)
    }

    pub(super) async fn thread_archive_inner(
        &self,
        params: ThreadArchiveParams,
    ) -> Result<(ThreadArchiveResponse, Vec<String>), JSONRPCErrorError> {
        let _thread_list_state_permit = self.acquire_thread_list_state_permit().await?;
        self.thread_archive_response(params).await
    }

    pub(super) async fn thread_archive_response(
        &self,
        params: ThreadArchiveParams,
    ) -> Result<(ThreadArchiveResponse, Vec<String>), JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        let subtree_thread_ids = self
            .thread_agent_directory_runtime
            .list_agent_subtree_thread_ids(thread_id)
            .await
            .map_err(|err| {
                internal_error(format!(
                    "failed to list agent subtree for thread id {thread_id}: {err}"
                ))
            })?;
        let mut thread_ids = vec![thread_id];
        let mut seen = HashSet::from([thread_id]);
        for subtree_thread_id in subtree_thread_ids {
            if seen.insert(subtree_thread_id) {
                thread_ids.push(subtree_thread_id);
            }
        }

        let mut archive_thread_ids = Vec::new();
        match self
            .thread_store
            .read_thread(StoreReadThreadParams {
                thread_id,
                include_archived: false,
                include_history: false,
            })
            .await
        {
            Ok(thread) => {
                if thread.archived_at.is_none() {
                    if let Some(rollout_path) = thread.rollout_path.as_ref() {
                        match tokio::fs::try_exists(rollout_path).await {
                            Ok(true) => {}
                            Ok(false) => {
                                return Ok((ThreadArchiveResponse {}, vec![params.thread_id]));
                            }
                            Err(err) => {
                                return Err(internal_error(format!(
                                    "failed to inspect rollout path for thread id {thread_id}: {err}"
                                )));
                            }
                        }
                    }
                    archive_thread_ids.push(thread_id);
                }
            }
            Err(err) if Self::is_missing_rollout_for_thread(&err, thread_id) => {
                return Ok((ThreadArchiveResponse {}, vec![params.thread_id]));
            }
            Err(err) => return Err(thread_store_archive_error("archive", err)),
        }
        for descendant_thread_id in thread_ids.into_iter().skip(1) {
            match self
                .thread_store
                .read_thread(StoreReadThreadParams {
                    thread_id: descendant_thread_id,
                    include_archived: true,
                    include_history: false,
                })
                .await
            {
                Ok(thread) => {
                    if thread.archived_at.is_none() {
                        archive_thread_ids.push(descendant_thread_id);
                    }
                }
                Err(err) => {
                    warn!(
                        "failed to read spawned descendant thread {descendant_thread_id} while archiving {thread_id}: {err}"
                    );
                }
            }
        }

        let mut archived_thread_ids = Vec::new();
        let Some((parent_thread_id, descendant_thread_ids)) = archive_thread_ids.split_first()
        else {
            return Ok((ThreadArchiveResponse {}, archived_thread_ids));
        };

        self.prepare_thread_for_archive(*parent_thread_id).await;
        match self
            .thread_store
            .archive_thread(StoreArchiveThreadParams {
                thread_id: *parent_thread_id,
            })
            .await
        {
            Ok(()) => {
                archived_thread_ids.push(parent_thread_id.to_string());
            }
            Err(err) => return Err(thread_store_archive_error("archive", err)),
        }

        for descendant_thread_id in descendant_thread_ids.iter().rev().copied() {
            self.prepare_thread_for_archive(descendant_thread_id).await;
            match self
                .thread_store
                .archive_thread(StoreArchiveThreadParams {
                    thread_id: descendant_thread_id,
                })
                .await
            {
                Ok(()) => {
                    archived_thread_ids.push(descendant_thread_id.to_string());
                }
                Err(err) => {
                    warn!(
                        "failed to archive spawned descendant thread {descendant_thread_id} while archiving {thread_id}: {err}"
                    );
                }
            }
        }

        Ok((ThreadArchiveResponse {}, archived_thread_ids))
    }

    fn thread_archive_missing_rollout_message(thread_id: ThreadId) -> String {
        format!("no rollout found for thread id {thread_id}")
    }

    fn is_missing_rollout_for_thread(err: &ThreadStoreError, thread_id: ThreadId) -> bool {
        matches!(
            err,
            ThreadStoreError::InvalidRequest { message }
                if message == &Self::thread_archive_missing_rollout_message(thread_id)
        )
    }

    pub(super) async fn thread_increment_elicitation_inner(
        &self,
        params: ThreadIncrementElicitationParams,
    ) -> Result<ThreadIncrementElicitationResponse, JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;
        let count = self
            .live_thread_elicitation
            .increment_thread_out_of_band_elicitation_count(thread_id)
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(_) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                err => internal_error(format!(
                    "failed to increment out-of-band elicitation counter: {err}"
                )),
            })?;
        Ok(ThreadIncrementElicitationResponse {
            count,
            paused: count > 0,
        })
    }

    pub(super) async fn thread_decrement_elicitation_inner(
        &self,
        params: ThreadDecrementElicitationParams,
    ) -> Result<ThreadDecrementElicitationResponse, JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;
        let count = self
            .live_thread_elicitation
            .decrement_thread_out_of_band_elicitation_count(thread_id)
            .await
            .map_err(|err| match err {
                CodexErr::ThreadNotFound(_) => {
                    invalid_request(format!("thread not found: {thread_id}"))
                }
                CodexErr::InvalidRequest(message) => invalid_request(message),
                err => internal_error(format!(
                    "failed to decrement out-of-band elicitation counter: {err}"
                )),
            })?;
        Ok(ThreadDecrementElicitationResponse {
            count,
            paused: count > 0,
        })
    }

    pub(super) async fn thread_set_name_response_inner(
        &self,
        params: ThreadSetNameParams,
    ) -> Result<(ThreadSetNameResponse, Option<ThreadNameUpdatedNotification>), JSONRPCErrorError>
    {
        let ThreadSetNameParams { thread_id, name } = params;
        let thread_id = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;
        let Some(name) = thread_service::util::normalize_thread_name(&name) else {
            return Err(invalid_request("thread name must not be empty"));
        };

        let _thread_list_state_permit = self.acquire_thread_list_state_permit().await?;
        self.thread_metadata_runtime
            .update_thread_metadata(
                thread_id,
                StoreThreadMetadataPatch {
                    name: Some(Some(name.clone())),
                    ..Default::default()
                },
                /*include_archived*/ false,
            )
            .await
            .map_err(|err| core_thread_write_error("set thread name", err))?;

        Ok((
            ThreadSetNameResponse {},
            Some(ThreadNameUpdatedNotification {
                thread_id: thread_id.to_string(),
                thread_name: Some(name),
            }),
        ))
    }

    pub(super) async fn thread_memory_mode_set_response_inner(
        &self,
        params: ThreadMemoryModeSetParams,
    ) -> Result<ThreadMemoryModeSetResponse, JSONRPCErrorError> {
        let ThreadMemoryModeSetParams { thread_id, mode } = params;
        let thread_id = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        self.thread_metadata_runtime
            .update_thread_metadata(
                thread_id,
                StoreThreadMetadataPatch {
                    memory_mode: Some(mode.to_core()),
                    ..Default::default()
                },
                /*include_archived*/ false,
            )
            .await
            .map_err(|err| core_thread_write_error("set thread memory mode", err))?;

        Ok(ThreadMemoryModeSetResponse {})
    }

    pub(super) async fn memory_reset_response_inner(
        &self,
    ) -> Result<MemoryResetResponse, JSONRPCErrorError> {
        let state_db = self
            .state_db
            .clone()
            .ok_or_else(|| internal_error("sqlite state db unavailable for memory reset"))?;

        state_db.clear_memory_data().await.map_err(|err| {
            internal_error(format!("failed to clear memory rows in state db: {err}"))
        })?;

        clear_memory_roots_contents(&self.config.codex_home)
            .await
            .map_err(|err| {
                internal_error(format!(
                    "failed to clear memory directories under {}: {err}",
                    self.config.codex_home.display()
                ))
            })?;

        Ok(MemoryResetResponse {})
    }

    pub(super) async fn thread_metadata_update_response_inner(
        &self,
        params: ThreadMetadataUpdateParams,
    ) -> Result<ThreadMetadataUpdateResponse, JSONRPCErrorError> {
        let ThreadMetadataUpdateParams {
            thread_id,
            git_info,
        } = params;

        let thread_uuid = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        let Some(ThreadMetadataGitInfoUpdateParams {
            sha,
            branch,
            origin_url,
        }) = git_info
        else {
            return Err(invalid_request("gitInfo must include at least one field"));
        };

        if sha.is_none() && branch.is_none() && origin_url.is_none() {
            return Err(invalid_request("gitInfo must include at least one field"));
        }

        let git_sha = Self::normalize_thread_metadata_git_field(sha, "gitInfo.sha")?;
        let git_branch = Self::normalize_thread_metadata_git_field(branch, "gitInfo.branch")?;
        let git_origin_url =
            Self::normalize_thread_metadata_git_field(origin_url, "gitInfo.originUrl")?;

        let patch = StoreThreadMetadataPatch {
            git_info: Some(StoreGitInfoPatch {
                sha: git_sha,
                branch: git_branch,
                origin_url: git_origin_url,
            }),
            ..Default::default()
        };

        let updated_thread = {
            let _thread_list_state_permit = self.acquire_thread_list_state_permit().await?;
            self.thread_metadata_runtime
                .update_thread_metadata(thread_uuid, patch, /*include_archived*/ true)
                .await
                .map_err(|err| core_thread_write_error("update thread metadata", err))?
        };
        let (mut thread, _) = thread_from_stored_thread(
            updated_thread,
            self.config.model_provider_id.as_str(),
            &self.config.cwd,
        );
        if let Ok(live_info) = self
            .live_thread_inspection
            .live_thread_info(thread_uuid)
            .await
        {
            thread.session_id = live_info.session_id.to_string();
        }
        self.attach_thread_name(thread_uuid, &mut thread).await;
        thread.lifecycle_status = resolve_thread_status(
            self.thread_watch_manager
                .loaded_status_for_thread(&thread.id)
                .await,
            /*has_in_progress_turn*/ false,
        );

        Ok(ThreadMetadataUpdateResponse { thread })
    }

    pub(super) fn normalize_thread_metadata_git_field(
        value: Option<Option<String>>,
        name: &str,
    ) -> Result<Option<Option<String>>, JSONRPCErrorError> {
        match value {
            Some(Some(value)) => {
                let value = value.trim().to_string();
                if value.is_empty() {
                    return Err(invalid_request(format!("{name} must not be empty")));
                }
                Ok(Some(Some(value)))
            }
            Some(None) => Ok(Some(None)),
            None => Ok(None),
        }
    }

    pub(super) async fn thread_unarchive_inner(
        &self,
        params: ThreadUnarchiveParams,
    ) -> Result<(ThreadUnarchiveResponse, ThreadUnarchivedNotification), JSONRPCErrorError> {
        let _thread_list_state_permit = self.acquire_thread_list_state_permit().await?;
        let (response, thread_id) = self.thread_unarchive_response(params).await?;
        Ok((response, ThreadUnarchivedNotification { thread_id }))
    }

    pub(super) async fn thread_unarchive_response(
        &self,
        params: ThreadUnarchiveParams,
    ) -> Result<(ThreadUnarchiveResponse, String), JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        let fallback_provider = self.config.model_provider_id.clone();
        let stored_thread = self
            .thread_store
            .unarchive_thread(StoreArchiveThreadParams { thread_id })
            .await
            .map_err(|err| thread_store_archive_error("unarchive", err))?;
        let (mut thread, _) =
            thread_from_stored_thread(stored_thread, fallback_provider.as_str(), &self.config.cwd);

        thread.lifecycle_status = resolve_thread_status(
            self.thread_watch_manager
                .loaded_status_for_thread(&thread.id)
                .await,
            /*has_in_progress_turn*/ false,
        );
        self.attach_thread_name(thread_id, &mut thread).await;
        let thread_id = thread.id.clone();
        Ok((ThreadUnarchiveResponse { thread }, thread_id))
    }

    pub(super) async fn thread_rollback_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRollbackParams,
    ) -> Result<(), JSONRPCErrorError> {
        self.thread_rollback_start(request_id, params).await
    }

    pub(super) async fn thread_rollback_start(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadRollbackParams,
    ) -> Result<(), JSONRPCErrorError> {
        let ThreadRollbackParams {
            thread_id,
            num_turns,
        } = params;

        if num_turns == 0 {
            return Err(invalid_request("numTurns must be >= 1"));
        }

        let thread_id = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        let request = request_id.clone();

        let rollback_already_in_progress = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let mut thread_state = thread_state.lock().await;
            if thread_state.pending_rollbacks.is_some() {
                true
            } else {
                thread_state.pending_rollbacks = Some(request.clone());
                false
            }
        };
        if rollback_already_in_progress {
            return Err(invalid_request(
                "rollback already in progress for this thread",
            ));
        }

        if let Err(err) = self
            .submit_core_op(
                request_id,
                thread_id,
                "thread/rollback",
                Op::ThreadRollback { num_turns },
                "failed to start rollback",
            )
            .await
        {
            // No ThreadRollback event will arrive if an error occurs.
            // Clean up and reply immediately.
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            thread_state.lock().await.pending_rollbacks = None;
            return Err(err);
        }
        Ok(())
    }

    pub(super) async fn thread_compact_start_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadCompactStartParams,
    ) -> Result<ThreadCompactStartResponse, JSONRPCErrorError> {
        let ThreadCompactStartParams { thread_id } = params;
        let thread_id = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        self.submit_core_op(
            request_id,
            thread_id,
            "thread/compact/start",
            Op::Compact,
            "failed to start compaction",
        )
        .await?;
        Ok(ThreadCompactStartResponse {})
    }

    pub(super) async fn thread_background_terminals_clean_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadBackgroundTerminalsCleanParams,
    ) -> Result<ThreadBackgroundTerminalsCleanResponse, JSONRPCErrorError> {
        let ThreadBackgroundTerminalsCleanParams { thread_id } = params;
        let thread_id = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        self.submit_core_op(
            request_id,
            thread_id,
            "thread/backgroundTerminals/clean",
            Op::CleanBackgroundTerminals,
            "failed to clean background terminals",
        )
        .await?;
        Ok(ThreadBackgroundTerminalsCleanResponse {})
    }

    pub(super) async fn thread_shell_command_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadShellCommandParams,
    ) -> Result<ThreadShellCommandResponse, JSONRPCErrorError> {
        let ThreadShellCommandParams { thread_id, command } = params;
        let command = command.trim().to_string();
        if command.is_empty() {
            return Err(invalid_request("command must not be empty"));
        }
        let thread_id = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        self.submit_core_op(
            request_id,
            thread_id,
            "thread/shellCommand",
            Op::RunUserShellCommand { command },
            "failed to start shell command",
        )
        .await?;
        Ok(ThreadShellCommandResponse {})
    }

    pub(super) async fn thread_approve_guardian_denied_action_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: ThreadApproveGuardianDeniedActionParams,
    ) -> Result<ThreadApproveGuardianDeniedActionResponse, JSONRPCErrorError> {
        let ThreadApproveGuardianDeniedActionParams { thread_id, event } = params;
        let event = serde_json::from_value(event)
            .map_err(|err| invalid_request(format!("invalid Guardian denial event: {err}")))?;
        let thread_id = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        self.submit_core_op(
            request_id,
            thread_id,
            "thread/approveGuardianDeniedAction",
            Op::ApproveGuardianDeniedAction { event },
            "failed to approve Guardian denial",
        )
        .await?;
        Ok(ThreadApproveGuardianDeniedActionResponse {})
    }
}

fn should_display_agent_message_event(message: &str) -> bool {
    !is_legacy_structured_assistant_message_text(message)
}

fn thread_start_create_error(err: CodexErr) -> JSONRPCErrorError {
    match err {
        CodexErr::InvalidRequest(message) => invalid_request(message),
        CodexErr::UnsupportedOperation(message)
            if message.contains("agent path") && message.contains("already exists") =>
        {
            invalid_request(message)
        }
        err => internal_error(format!("error creating thread: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_thread_start_agent_derives_path_from_task_name() {
        let agent = parse_thread_start_agent(
            Some("owner_dev".to_string()),
            Some("feature-owner".to_string()),
        )
        .expect("valid agent")
        .expect("agent metadata");

        assert_eq!(
            agent.agent_path.as_ref().map(ToString::to_string),
            Some("/owner_dev".to_string())
        );
        assert_eq!(agent.agent_role.as_deref(), Some("feature-owner"));
    }

    #[test]
    fn parse_thread_start_agent_accepts_root_level_agent_path() {
        let agent = parse_thread_start_agent(
            Some("/owner_dev".to_string()),
            Some("feature-owner".to_string()),
        )
        .expect("valid agent path")
        .expect("agent metadata");

        assert_eq!(
            agent.agent_path.as_ref().map(ToString::to_string),
            Some("/owner_dev".to_string())
        );
        assert_eq!(agent.agent_role.as_deref(), Some("feature-owner"));
    }

    #[test]
    fn parse_thread_start_agent_rejects_invalid_task_name() {
        let error = parse_thread_start_agent(Some("OwnerDev".to_string()), None)
            .expect_err("invalid task name should fail");

        assert!(error.message.contains("invalid taskName"));
    }

    #[test]
    fn parse_thread_start_agent_role_only_does_not_create_root_metadata() {
        let agent = parse_thread_start_agent(None, Some("feature-owner".to_string()))
            .expect("valid role")
            .expect("role should be preserved");

        assert!(agent.agent_path.is_none());
        assert_eq!(agent.agent_role.as_deref(), Some("feature-owner"));
    }

    #[test]
    fn duplicate_agent_path_create_error_is_invalid_request() {
        let error = thread_start_create_error(CodexErr::UnsupportedOperation(
            "agent path `/project` already exists".to_string(),
        ));

        assert_eq!(error.code, -32600);
        assert!(error.message.contains("already exists"));
    }

    #[test]
    fn status_changed_payload_takes_precedence_when_live_status_missing() {
        let lifecycle_status = thread_status_changed_lifecycle_status(
            Some(&AgentStatus::Shutdown),
            None,
            ThreadLifecycleStatus::NotLoaded,
        );

        assert_eq!(
            lifecycle_status,
            ThreadLifecycleStatus::Final {
                result: ThreadLifecycleFinalStatus::Shutdown
            }
        );
    }

    #[test]
    fn direct_live_agent_message_suppresses_raw_subagent_notification() {
        let message = serde_json::json!({
            "author": "/root/worker",
            "recipient": "/root",
            "other_recipients": [],
            "content": concat!(
                "<subagent_notification>\n",
                r#"{"agent_path":"/root/worker","status":{"completed":"done"}}"#,
                "\n</subagent_notification>"
            ),
            "operation": "childCompletion",
        })
        .to_string();

        assert!(!should_display_agent_message_event(&message));
    }

    #[test]
    fn direct_live_agent_message_preserves_ordinary_json() {
        let message = serde_json::json!({
            "content": "ordinary assistant JSON",
            "status": { "completed": "done" },
        })
        .to_string();

        assert!(should_display_agent_message_event(&message));
    }

    #[test]
    fn status_changed_without_payload_prefers_live_status() {
        let lifecycle_status = thread_status_changed_lifecycle_status(
            None,
            Some(&AgentStatus::Completed(Some("done".to_string()))),
            ThreadLifecycleStatus::NotLoaded,
        );

        assert_eq!(
            lifecycle_status,
            ThreadLifecycleStatus::completed(Some("done".to_string()))
        );
    }

    #[test]
    fn status_changed_without_payload_falls_back_to_watch_status() {
        let watch_status = ThreadLifecycleStatus::Active {
            active_flags: vec![ThreadLifecycleActiveFlag::Running],
        };
        let lifecycle_status =
            thread_status_changed_lifecycle_status(None, None, watch_status.clone());

        assert_eq!(lifecycle_status, watch_status);
    }
}
