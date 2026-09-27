use super::*;
use crate::live_thread_runtime::AppServerLiveThreadInspectionRuntime;

pub(crate) trait AppsRuntime: Send + Sync {
    fn plugin_runtime(&self) -> plugin_service_api::SharedPluginRuntime;
}

impl AppsRuntime for ThreadService {
    fn plugin_runtime(&self) -> plugin_service_api::SharedPluginRuntime {
        ThreadService::plugin_runtime(self)
    }
}

#[derive(Clone)]
pub(crate) struct AppsRequestProcessor {
    auth_manager: Arc<AuthManager>,
    apps_runtime: Arc<dyn AppsRuntime>,
    live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime>,
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
        R: AppsRuntime + AppServerLiveThreadInspectionRuntime + 'static,
    {
        let live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime> =
            apps_runtime.clone();
        let apps_runtime: Arc<dyn AppsRuntime> = apps_runtime;
        Self {
            auth_manager,
            apps_runtime,
            live_thread_inspection,
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
        let mut config =
            load_latest_config_for_request(&self.config_manager, /*fallback_cwd*/ None).await?;

        if let Some(thread_id) = params.thread_id.as_deref() {
            let thread_id = ThreadId::from_string(thread_id)
                .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;
            let apps_enabled = self
                .live_thread_inspection
                .live_thread_feature_enabled(thread_id, Feature::Apps)
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
        plugin_runtime: plugin_service_api::SharedPluginRuntime,
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
        plugin_runtime: plugin_service_api::SharedPluginRuntime,
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
        let (accessible_connectors, all_connectors) = tokio::join!(
            core_connectors::list_cached_accessible_connectors_from_mcp_tools(
                &config,
                auth_snapshot.as_ref()
            ),
            chatgpt_connectors::list_cached_all_connectors(&chatgpt_config)
        );
        let mut load_state =
            AppListLoadState::new(accessible_connectors, all_connectors, force_refetch);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let accessible_config = config.clone();
        let accessible_tx = tx.clone();
        tokio::spawn(async move {
            let mcp_auth_runtime = mcp_service::DefaultMcpAuthRuntime;
            let mcp_connection_runtime_factory = mcp_service::DefaultMcpConnectionRuntimeFactory;
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
        if let Some(data) = load_state.initial_notification_data(&config) {
            send_app_list_updated_notification(outgoing, data).await;
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

            load_state.apply_load_result(result)?;

            if let Some(data) = load_state.current_notification_data(&config) {
                send_app_list_updated_notification(outgoing, data).await;
            }

            if let Some(response) = load_state.final_response(&config, start, limit)? {
                return Ok(response);
            }
        }
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

struct AppListLoadState {
    accessible_connectors: Option<Vec<AppInfo>>,
    all_connectors: Option<Vec<AppInfo>>,
    cached_all_connectors: Option<Vec<AppInfo>>,
    accessible_loaded: bool,
    all_loaded: bool,
    force_refetch: bool,
    last_notified_apps: Option<Vec<AppInfo>>,
}

impl AppListLoadState {
    fn new(
        accessible_connectors: Option<Vec<AppInfo>>,
        all_connectors: Option<Vec<AppInfo>>,
        force_refetch: bool,
    ) -> Self {
        Self {
            cached_all_connectors: all_connectors.clone(),
            accessible_connectors,
            all_connectors,
            accessible_loaded: false,
            all_loaded: false,
            force_refetch,
            last_notified_apps: None,
        }
    }

    fn apply_load_result(&mut self, result: AppListLoadResult) -> Result<(), JSONRPCErrorError> {
        match result {
            AppListLoadResult::Accessible(Ok(connectors)) => {
                self.accessible_connectors = Some(connectors);
                self.accessible_loaded = true;
                Ok(())
            }
            AppListLoadResult::Accessible(Err(err)) => Err(internal_error(err)),
            AppListLoadResult::Directory(Ok(connectors)) => {
                self.all_connectors = Some(connectors);
                self.all_loaded = true;
                Ok(())
            }
            AppListLoadResult::Directory(Err(err)) => Err(internal_error(err)),
        }
    }

    fn initial_notification_data(&mut self, config: &Config) -> Option<Vec<AppInfo>> {
        if self.accessible_connectors.is_none() && self.all_connectors.is_none() {
            return None;
        }

        let merged = app_list_with_enabled_state(
            self.all_connectors.as_deref(),
            self.accessible_connectors.as_deref(),
            config,
        );
        self.take_notification_data(merged)
    }

    fn current_notification_data(&mut self, config: &Config) -> Option<Vec<AppInfo>> {
        let merged = self.current_merged_apps(config);
        self.take_notification_data(merged)
    }

    fn final_response(
        &self,
        config: &Config,
        start: usize,
        limit: Option<u32>,
    ) -> Result<Option<AppsListResponse>, JSONRPCErrorError> {
        if !self.is_fully_loaded() {
            return Ok(None);
        }

        paginate_apps(self.current_merged_apps(config).as_slice(), start, limit).map(Some)
    }

    fn take_notification_data(&mut self, merged: Vec<AppInfo>) -> Option<Vec<AppInfo>> {
        if !should_send_app_list_updated_notification(
            merged.as_slice(),
            self.accessible_loaded,
            self.all_loaded,
        ) || self.last_notified_apps.as_ref() == Some(&merged)
        {
            return None;
        }

        self.last_notified_apps = Some(merged.clone());
        Some(merged)
    }

    fn current_merged_apps(&self, config: &Config) -> Vec<AppInfo> {
        let (all_connectors, accessible_connectors) = self.current_merge_sources();
        app_list_with_enabled_state(all_connectors, accessible_connectors, config)
    }

    fn current_merge_sources(&self) -> (Option<&[AppInfo]>, Option<&[AppInfo]>) {
        let showing_interim_force_refetch = self.force_refetch && !self.is_fully_loaded();
        let all_connectors =
            if showing_interim_force_refetch && self.cached_all_connectors.is_some() {
                self.cached_all_connectors.as_deref()
            } else {
                self.all_connectors.as_deref()
            };
        let accessible_connectors = if showing_interim_force_refetch && !self.accessible_loaded {
            None
        } else {
            self.accessible_connectors.as_deref()
        };
        (all_connectors, accessible_connectors)
    }

    fn is_fully_loaded(&self) -> bool {
        self.accessible_loaded && self.all_loaded
    }
}

fn app_list_with_enabled_state(
    all_connectors: Option<&[AppInfo]>,
    accessible_connectors: Option<&[AppInfo]>,
    config: &Config,
) -> Vec<AppInfo> {
    core_connectors::with_app_enabled_state(
        merge_loaded_apps(all_connectors, accessible_connectors),
        config,
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, is_accessible: bool) -> AppInfo {
        AppInfo {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            logo_url: None,
            logo_url_dark: None,
            distribution_channel: None,
            branding: None,
            app_metadata: None,
            labels: None,
            install_url: None,
            is_accessible,
            is_enabled: true,
            plugin_display_names: Vec::new(),
        }
    }

    fn app_ids(apps: Option<&[AppInfo]>) -> Vec<&str> {
        apps.unwrap_or_default()
            .iter()
            .map(|app| app.id.as_str())
            .collect()
    }

    #[test]
    fn app_list_load_state_force_refetch_uses_cached_directory_until_fully_loaded() {
        let mut state = AppListLoadState::new(
            Some(vec![app("cached-accessible", true)]),
            Some(vec![app("cached-directory", false)]),
            true,
        );

        state
            .apply_load_result(AppListLoadResult::Accessible(Ok(vec![app(
                "fresh-accessible",
                true,
            )])))
            .expect("accessible load result should apply");
        let (all_connectors, accessible_connectors) = state.current_merge_sources();
        assert_eq!(app_ids(all_connectors), vec!["cached-directory"]);
        assert_eq!(app_ids(accessible_connectors), vec!["fresh-accessible"]);

        state
            .apply_load_result(AppListLoadResult::Directory(Ok(vec![app(
                "fresh-directory",
                false,
            )])))
            .expect("directory load result should apply");
        let (all_connectors, accessible_connectors) = state.current_merge_sources();
        assert_eq!(app_ids(all_connectors), vec!["fresh-directory"]);
        assert_eq!(app_ids(accessible_connectors), vec!["fresh-accessible"]);
    }

    #[test]
    fn app_list_load_state_suppresses_duplicate_notifications() {
        let mut state = AppListLoadState::new(None, None, false);
        let accessible = vec![app("accessible", true)];

        assert!(
            state
                .take_notification_data(vec![app("directory", false)])
                .is_none()
        );

        state.accessible_loaded = true;
        assert_eq!(
            state.take_notification_data(accessible.clone()),
            Some(accessible.clone())
        );
        assert!(state.take_notification_data(accessible.clone()).is_none());

        state.all_loaded = true;
        assert!(state.take_notification_data(accessible).is_none());

        let final_directory_only = vec![app("directory", false)];
        assert_eq!(
            state.take_notification_data(final_directory_only.clone()),
            Some(final_directory_only)
        );
    }
}
