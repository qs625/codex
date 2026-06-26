use super::*;
use futures::future::BoxFuture;

pub(crate) trait AppsRuntime: Send + Sync {
    fn thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> BoxFuture<'_, CodexResult<bool>>;

    fn plugin_runtime(&self) -> codex_core_plugins_api::SharedPluginRuntime;
}

impl AppsRuntime for ThreadService {
    fn thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> BoxFuture<'_, CodexResult<bool>> {
        Box::pin(LiveThreadRegistry::thread_feature_enabled(
            self, thread_id, feature,
        ))
    }

    fn plugin_runtime(&self) -> codex_core_plugins_api::SharedPluginRuntime {
        ThreadService::plugin_runtime(self)
    }
}

#[derive(Clone)]
pub(crate) struct AppsRequestProcessor {
    auth_manager: Arc<AuthManager>,
    apps_runtime: Arc<dyn AppsRuntime>,
    outgoing: Arc<OutgoingMessageSender>,
    config_manager: ConfigManager,
    environment_manager: Arc<EnvironmentManager>,
    workspace_settings_cache: Arc<workspace_settings::WorkspaceSettingsCache>,
}

impl AppsRequestProcessor {
    pub(crate) fn new<R>(
        auth_manager: Arc<AuthManager>,
        apps_runtime: Arc<R>,
        outgoing: Arc<OutgoingMessageSender>,
        config_manager: ConfigManager,
        environment_manager: Arc<EnvironmentManager>,
        workspace_settings_cache: Arc<workspace_settings::WorkspaceSettingsCache>,
    ) -> Self
    where
        R: AppsRuntime + 'static,
    {
        let apps_runtime: Arc<dyn AppsRuntime> = apps_runtime;
        Self {
            auth_manager,
            apps_runtime,
            outgoing,
            config_manager,
            environment_manager,
            workspace_settings_cache,
        }
    }

    pub(crate) async fn apps_list(
        &self,
        request_id: &ConnectionRequestId,
        params: AppsListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.apps_list_inner(request_id, params)
            .await
            .map(|response| response.map(Into::into))
    }

    async fn apps_list_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: AppsListParams,
    ) -> Result<Option<AppsListResponse>, JSONRPCErrorError> {
        let mut config = self.load_latest_config(/*fallback_cwd*/ None).await?;

        if let Some(thread_id) = params.thread_id.as_deref() {
            let thread_id = ThreadId::from_string(thread_id)
                .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;
            let apps_enabled = self
                .apps_runtime
                .thread_feature_enabled(thread_id, Feature::Apps)
                .await
                .map_err(|_| invalid_request(format!("thread not found: {thread_id}")))?;

            let _ = config.features.set_enabled(Feature::Apps, apps_enabled);
        }

        let auth = self.auth_manager.auth().await;
        let auth_snapshot = auth.as_ref().map(CodexAuth::request_auth_snapshot);
        if !config
            .features
            .apps_enabled_for_auth(auth.as_ref().is_some_and(CodexAuth::uses_codex_backend))
        {
            return Ok(Some(AppsListResponse {
                data: Vec::new(),
                next_cursor: None,
            }));
        }

        if !self
            .workspace_codex_plugins_enabled(&config, auth.as_ref())
            .await
        {
            return Ok(Some(AppsListResponse {
                data: Vec::new(),
                next_cursor: None,
            }));
        }

        let request = request_id.clone();
        let outgoing = Arc::clone(&self.outgoing);
        let environment_manager = Arc::clone(&self.environment_manager);
        let plugin_runtime = self.apps_runtime.plugin_runtime();
        tokio::spawn(async move {
            Self::apps_list_task(
                outgoing,
                request,
                params,
                config,
                auth_snapshot,
                plugin_runtime,
                environment_manager,
            )
            .await;
        });
        Ok(None)
    }

    async fn apps_list_task(
        outgoing: Arc<OutgoingMessageSender>,
        request_id: ConnectionRequestId,
        params: AppsListParams,
        config: Config,
        auth_snapshot: Option<RequestAuthSnapshot>,
        plugin_runtime: codex_core_plugins_api::SharedPluginRuntime,
        environment_manager: Arc<EnvironmentManager>,
    ) {
        let result = Self::apps_list_response(
            &outgoing,
            params,
            config,
            auth_snapshot,
            plugin_runtime,
            environment_manager,
        )
        .await;
        outgoing.send_result(request_id, result).await;
    }

    async fn apps_list_response(
        outgoing: &Arc<OutgoingMessageSender>,
        params: AppsListParams,
        config: Config,
        auth_snapshot: Option<RequestAuthSnapshot>,
        plugin_runtime: codex_core_plugins_api::SharedPluginRuntime,
        environment_manager: Arc<EnvironmentManager>,
    ) -> Result<AppsListResponse, JSONRPCErrorError> {
        let AppsListParams {
            cursor,
            limit,
            thread_id: _,
            force_refetch,
        } = params;
        let start = match cursor {
            Some(cursor) => match cursor.parse::<usize>() {
                Ok(idx) => idx,
                Err(_) => return Err(invalid_request(format!("invalid cursor: {cursor}"))),
            },
            None => 0,
        };

        let chatgpt_config = chatgpt_config_from_core(&config);
        let (mut accessible_connectors, mut all_connectors) = tokio::join!(
            core_connectors::list_cached_accessible_connectors_from_mcp_tools(
                &config,
                auth_snapshot.as_ref()
            ),
            chatgpt_connectors::list_cached_all_connectors(&chatgpt_config)
        );
        let cached_all_connectors = all_connectors.clone();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let accessible_config = config.clone();
        let accessible_tx = tx.clone();
        tokio::spawn(async move {
            let mcp_auth_runtime = codex_mcp::DefaultMcpAuthRuntime;
            let mcp_connection_runtime_factory = codex_mcp::DefaultMcpConnectionRuntimeFactory;
            let result = core_connectors::list_accessible_connectors_from_mcp_tools_with_environment_provider(
                &accessible_config,
                auth_snapshot.as_ref(),
                force_refetch,
                plugin_runtime.as_ref(),
                &environment_manager,
                &mcp_auth_runtime,
                &mcp_connection_runtime_factory,
            )
            .await
            .map(|status| status.connectors)
            .map_err(|err| format!("failed to load accessible apps: {err}"));
            let _ = accessible_tx.send(AppListLoadResult::Accessible(result));
        });

        let all_config = chatgpt_config.clone();
        tokio::spawn(async move {
            let result =
                chatgpt_connectors::list_all_connectors_with_options(&all_config, force_refetch)
                    .await
                    .map_err(|err| format!("failed to list apps: {err}"));
            let _ = tx.send(AppListLoadResult::Directory(result));
        });

        let app_list_deadline = tokio::time::Instant::now() + APP_LIST_LOAD_TIMEOUT;
        let mut accessible_loaded = false;
        let mut all_loaded = false;
        let mut last_notified_apps = None;

        if accessible_connectors.is_some() || all_connectors.is_some() {
            let merged = core_connectors::with_app_enabled_state(
                merge_loaded_apps(all_connectors.as_deref(), accessible_connectors.as_deref()),
                &config,
            );
            if should_send_app_list_updated_notification(
                merged.as_slice(),
                accessible_loaded,
                all_loaded,
            ) {
                send_app_list_updated_notification(outgoing, merged.clone()).await;
                last_notified_apps = Some(merged);
            }
        }

        loop {
            let result = match tokio::time::timeout_at(app_list_deadline, rx.recv()).await {
                Ok(Some(result)) => result,
                Ok(None) => {
                    return Err(internal_error("failed to load app lists"));
                }
                Err(_) => {
                    let timeout_seconds = APP_LIST_LOAD_TIMEOUT.as_secs();
                    return Err(internal_error(format!(
                        "timed out waiting for app lists after {timeout_seconds} seconds"
                    )));
                }
            };

            match result {
                AppListLoadResult::Accessible(Ok(connectors)) => {
                    accessible_connectors = Some(connectors);
                    accessible_loaded = true;
                }
                AppListLoadResult::Accessible(Err(err)) => {
                    return Err(internal_error(err));
                }
                AppListLoadResult::Directory(Ok(connectors)) => {
                    all_connectors = Some(connectors);
                    all_loaded = true;
                }
                AppListLoadResult::Directory(Err(err)) => {
                    return Err(internal_error(err));
                }
            }

            let showing_interim_force_refetch = force_refetch && !(accessible_loaded && all_loaded);
            let all_connectors_for_update =
                if showing_interim_force_refetch && cached_all_connectors.is_some() {
                    cached_all_connectors.as_deref()
                } else {
                    all_connectors.as_deref()
                };
            let accessible_connectors_for_update =
                if showing_interim_force_refetch && !accessible_loaded {
                    None
                } else {
                    accessible_connectors.as_deref()
                };
            let merged = core_connectors::with_app_enabled_state(
                merge_loaded_apps(all_connectors_for_update, accessible_connectors_for_update),
                &config,
            );
            if should_send_app_list_updated_notification(
                merged.as_slice(),
                accessible_loaded,
                all_loaded,
            ) && last_notified_apps.as_ref() != Some(&merged)
            {
                send_app_list_updated_notification(outgoing, merged.clone()).await;
                last_notified_apps = Some(merged.clone());
            }

            if accessible_loaded && all_loaded {
                return paginate_apps(merged.as_slice(), start, limit);
            }
        }
    }

    async fn load_latest_config(
        &self,
        fallback_cwd: Option<PathBuf>,
    ) -> Result<Config, JSONRPCErrorError> {
        self.config_manager
            .load_latest_config(fallback_cwd)
            .await
            .map_err(|err| internal_error(format!("failed to reload config: {err}")))
    }

    async fn workspace_codex_plugins_enabled(
        &self,
        config: &Config,
        auth: Option<&CodexAuth>,
    ) -> bool {
        match workspace_settings::codex_plugins_enabled_for_workspace(
            &chatgpt_config_from_core(config),
            auth,
            Some(&self.workspace_settings_cache),
        )
        .await
        {
            Ok(enabled) => enabled,
            Err(err) => {
                warn!(
                    "failed to fetch workspace Codex plugins setting; allowing Codex plugins: {err:#}"
                );
                true
            }
        }
    }
}

const APP_LIST_LOAD_TIMEOUT: Duration = Duration::from_secs(90);

enum AppListLoadResult {
    Accessible(Result<Vec<AppInfo>, String>),
    Directory(Result<Vec<AppInfo>, String>),
}

fn merge_loaded_apps(
    all_connectors: Option<&[AppInfo]>,
    accessible_connectors: Option<&[AppInfo]>,
) -> Vec<AppInfo> {
    let all_connectors_loaded = all_connectors.is_some();
    let all = all_connectors.map_or_else(Vec::new, <[AppInfo]>::to_vec);
    let accessible = accessible_connectors.map_or_else(Vec::new, <[AppInfo]>::to_vec);
    chatgpt_connectors::merge_connectors_with_accessible(all, accessible, all_connectors_loaded)
}

fn should_send_app_list_updated_notification(
    connectors: &[AppInfo],
    accessible_loaded: bool,
    all_loaded: bool,
) -> bool {
    connectors.iter().any(|connector| connector.is_accessible) || (accessible_loaded && all_loaded)
}

fn paginate_apps(
    connectors: &[AppInfo],
    start: usize,
    limit: Option<u32>,
) -> Result<AppsListResponse, JSONRPCErrorError> {
    let total = connectors.len();
    if start > total {
        return Err(invalid_request(format!(
            "cursor {start} exceeds total apps {total}"
        )));
    }

    let effective_limit = limit.unwrap_or(total as u32).max(1) as usize;
    let end = start.saturating_add(effective_limit).min(total);
    let data = connectors[start..end].to_vec();
    let next_cursor = if end < total {
        Some(end.to_string())
    } else {
        None
    };

    Ok(AppsListResponse { data, next_cursor })
}

async fn send_app_list_updated_notification(
    outgoing: &Arc<OutgoingMessageSender>,
    data: Vec<AppInfo>,
) {
    outgoing
        .send_server_notification(ServerNotification::AppListUpdated(
            AppListUpdatedNotification { data },
        ))
        .await;
}
