use super::*;
use crate::models::ModelCatalogRuntime;
use app_server_protocol::Model;
use codex_agent_roles::DEFAULT_ROLE_NAME;
use codex_agent_roles::built_in_configs;
use futures::StreamExt;
use protocol::config_types::CollaborationModeMask;
use skill_service_api::SharedSkillServiceApi;
use skill_service_api::SkillError;
use skill_service_api::SkillMetadata;
use skill_service_api::SkillsLoadInput;

pub(crate) trait CatalogRuntime:
    ModelCatalogRuntime + thread_service_api::ThreadProviderCatalogRuntime + Send + Sync
{
    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask>;
}

impl CatalogRuntime for ThreadService {
    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask> {
        ThreadService::list_collaboration_modes(self)
    }
}

#[derive(Clone)]
pub(crate) struct CatalogRequestProcessor {
    pub(super) auth_manager: Arc<AuthManager>,
    pub(super) catalog_runtime: Arc<dyn CatalogRuntime>,
    pub(super) skill_service: SharedSkillServiceApi,
    pub(super) plugins_manager: Arc<PluginsManager>,
    pub(super) config: Arc<Config>,
    pub(super) config_manager: ConfigManager,
    pub(super) environment_manager: Arc<EnvironmentManager>,
    pub(super) workspace_settings_cache: Arc<workspace_settings::WorkspaceSettingsCache>,
}

const SKILLS_LIST_CWD_CONCURRENCY: usize = 5;

struct ModelCatalogSnapshot {
    models: Vec<Model>,
}

impl ModelCatalogSnapshot {
    fn new(models: Vec<Model>) -> Self {
        Self { models }
    }

    fn model_provider_ids(&self) -> Vec<String> {
        let mut model_providers = self
            .models
            .iter()
            .filter_map(|model| model.model_provider.clone())
            .collect::<Vec<_>>();
        model_providers.sort();
        model_providers.dedup();
        model_providers
    }
}

#[derive(Debug)]
struct CatalogPage<T> {
    data: Vec<T>,
    next_cursor: Option<String>,
}

struct CatalogPagination {
    limit: Option<u32>,
    cursor: Option<String>,
    total: usize,
    item_name: &'static str,
}

impl CatalogPagination {
    fn models(params: ModelListParams, total: usize) -> Self {
        let ModelListParams {
            limit,
            cursor,
            include_hidden: _,
        } = params;
        Self {
            limit,
            cursor,
            total,
            item_name: "models",
        }
    }

    fn feature_flags(params: ExperimentalFeatureListParams, total: usize) -> Self {
        let ExperimentalFeatureListParams { cursor, limit } = params;
        Self {
            limit,
            cursor,
            total,
            item_name: "feature flags",
        }
    }

    fn page<T: Clone>(&self, items: &[T]) -> Result<CatalogPage<T>, JSONRPCErrorError> {
        debug_assert_eq!(self.total, items.len());
        if self.total == 0 {
            return Ok(CatalogPage {
                data: Vec::new(),
                next_cursor: None,
            });
        }

        // Clamp to 1 so limit=0 cannot return a non-advancing page.
        let effective_limit = self.limit.unwrap_or(self.total as u32).max(1) as usize;
        let effective_limit = effective_limit.min(self.total);
        let start = match &self.cursor {
            Some(cursor) => cursor
                .parse::<usize>()
                .map_err(|_| invalid_request(format!("invalid cursor: {cursor}")))?,
            None => 0,
        };

        if start > self.total {
            return Err(invalid_request(format!(
                "cursor {start} exceeds total {} {}",
                self.item_name, self.total
            )));
        }

        let end = start.saturating_add(effective_limit).min(self.total);
        let data = items[start..end].to_vec();
        let next_cursor = if end < self.total {
            Some(end.to_string())
        } else {
            None
        };

        Ok(CatalogPage { data, next_cursor })
    }
}

struct FeatureCatalogSnapshot {
    features: Vec<ApiExperimentalFeature>,
}

impl FeatureCatalogSnapshot {
    fn from_config(config: &Config, workspace_codex_plugins_enabled: bool) -> Self {
        let features = FEATURES
            .iter()
            .map(|spec| feature_to_api(spec, config, workspace_codex_plugins_enabled))
            .collect();
        Self { features }
    }
}

fn feature_to_api(
    spec: &codex_features::FeatureSpec,
    config: &Config,
    workspace_codex_plugins_enabled: bool,
) -> ApiExperimentalFeature {
    let (stage, display_name, description, announcement) = match spec.stage {
        Stage::Experimental {
            name,
            menu_description,
            announcement,
        } => (
            ApiExperimentalFeatureStage::Beta,
            Some(name.to_string()),
            Some(menu_description.to_string()),
            Some(announcement.to_string()),
        ),
        Stage::UnderDevelopment => (
            ApiExperimentalFeatureStage::UnderDevelopment,
            None,
            None,
            None,
        ),
        Stage::Stable => (ApiExperimentalFeatureStage::Stable, None, None, None),
        Stage::Deprecated => (ApiExperimentalFeatureStage::Deprecated, None, None, None),
        Stage::Removed => (ApiExperimentalFeatureStage::Removed, None, None, None),
    };

    ApiExperimentalFeature {
        name: spec.key.to_string(),
        stage,
        display_name,
        description,
        announcement,
        enabled: config.features.enabled(spec.id)
            && workspace_allows_feature(spec.id, workspace_codex_plugins_enabled),
        default_enabled: spec.default_enabled,
    }
}

fn workspace_allows_feature(feature: Feature, workspace_codex_plugins_enabled: bool) -> bool {
    workspace_codex_plugins_enabled || !matches!(feature, Feature::Apps | Feature::Plugins)
}

struct ThreadProviderCatalogSnapshot {
    runtime_providers: Vec<thread_service_api::ThreadProviderRuntimeDescriptor>,
    native_agent_types: Vec<AgentType>,
    native_model_providers: Vec<String>,
}

impl ThreadProviderCatalogSnapshot {
    fn into_response(self) -> ThreadProviderListResponse {
        let external_model_selection = ThreadProviderModelSelection {
            mode: ThreadProviderModelSelectionMode::ProviderDefault,
            model_providers: Vec::new(),
        };
        let native_model_selection = ThreadProviderModelSelection {
            mode: ThreadProviderModelSelectionMode::Catalog,
            model_providers: self.native_model_providers,
        };
        let data = self
            .runtime_providers
            .into_iter()
            .map(|provider| {
                let is_native =
                    provider.kind == thread_service_api::ThreadProviderRuntimeKind::Native;
                ThreadProviderDescriptor {
                    id: provider.id,
                    display_name: provider.display_name,
                    kind: provider_kind_to_api(provider.kind),
                    description: provider.description,
                    agent_types: if is_native {
                        self.native_agent_types.clone()
                    } else {
                        Vec::new()
                    },
                    model_selection: if is_native {
                        native_model_selection.clone()
                    } else {
                        external_model_selection.clone()
                    },
                    capabilities: provider_capabilities_to_api(provider.capabilities),
                }
            })
            .collect();
        ThreadProviderListResponse { data }
    }
}

struct CatalogCwdTargets {
    cwds: Vec<PathBuf>,
}

impl CatalogCwdTargets {
    fn from_requested(cwds: Vec<PathBuf>, fallback_cwd: &Path) -> Self {
        let cwds = if cwds.is_empty() {
            vec![fallback_cwd.to_path_buf()]
        } else {
            cwds
        };
        Self { cwds }
    }

    fn into_vec(self) -> Vec<PathBuf> {
        self.cwds
    }
}

fn skills_to_info(
    skills: &[SkillMetadata],
    disabled_paths: &HashSet<AbsolutePathBuf>,
) -> Vec<app_server_protocol::SkillMetadata> {
    skills
        .iter()
        .map(|skill| {
            let enabled = !disabled_paths.contains(&skill.path_to_skills_md);
            app_server_protocol::SkillMetadata {
                name: skill.name.clone(),
                description: skill.description.clone(),
                short_description: skill.short_description.clone(),
                interface: skill.interface.clone().map(|interface| {
                    app_server_protocol::SkillInterface {
                        display_name: interface.display_name,
                        short_description: interface.short_description,
                        icon_small: interface.icon_small,
                        icon_large: interface.icon_large,
                        brand_color: interface.brand_color,
                        default_prompt: interface.default_prompt,
                    }
                }),
                dependencies: skill.dependencies.clone().map(|dependencies| {
                    app_server_protocol::SkillDependencies {
                        tools: dependencies
                            .tools
                            .into_iter()
                            .map(|tool| app_server_protocol::SkillToolDependency {
                                r#type: tool.r#type,
                                value: tool.value,
                                description: tool.description,
                                transport: tool.transport,
                                command: tool.command,
                                url: tool.url,
                            })
                            .collect(),
                    }
                }),
                path: skill.path_to_skills_md.clone(),
                scope: skill.scope.into(),
                enabled,
            }
        })
        .collect()
}

fn hooks_to_info(hooks: &[hooks::HookListEntry]) -> Vec<HookMetadata> {
    hooks
        .iter()
        .map(|hook| HookMetadata {
            key: hook.key.clone(),
            event_name: hook.event_name.into(),
            handler_type: hook.handler_type.into(),
            matcher: hook.matcher.clone(),
            command: hook.command.clone(),
            timeout_sec: hook.timeout_sec,
            status_message: hook.status_message.clone(),
            source_path: hook.source_path.clone(),
            source: hook.source.into(),
            plugin_id: hook.plugin_id.clone(),
            display_order: hook.display_order,
            enabled: hook.enabled,
            is_managed: hook.is_managed,
            current_hash: hook.current_hash.clone(),
            trust_status: hook.trust_status.into(),
        })
        .collect()
}

fn errors_to_info(errors: &[SkillError]) -> Vec<app_server_protocol::SkillErrorInfo> {
    errors
        .iter()
        .map(|err| app_server_protocol::SkillErrorInfo {
            path: err.path.to_path_buf(),
            message: err.message.clone(),
        })
        .collect()
}

fn native_agent_types(config: &Config) -> Vec<AgentType> {
    let mut items = built_in_configs()
        .iter()
        .map(|(name, role)| AgentType {
            name: name.clone(),
            description: role.description.clone(),
            built_in: true,
        })
        .collect::<Vec<_>>();

    for (name, role) in &config.agent_roles {
        if let Some(item) = items.iter_mut().find(|item| item.name == *name) {
            item.description = role.description.clone();
            item.built_in = false;
        } else {
            items.push(AgentType {
                name: name.clone(),
                description: role.description.clone(),
                built_in: false,
            });
        }
    }

    items.sort_by(|left, right| {
        if left.name == DEFAULT_ROLE_NAME && right.name != DEFAULT_ROLE_NAME {
            return std::cmp::Ordering::Less;
        }
        if right.name == DEFAULT_ROLE_NAME && left.name != DEFAULT_ROLE_NAME {
            return std::cmp::Ordering::Greater;
        }
        left.name.cmp(&right.name)
    });

    items
}

fn provider_kind_to_api(kind: thread_service_api::ThreadProviderRuntimeKind) -> ThreadProviderKind {
    match kind {
        thread_service_api::ThreadProviderRuntimeKind::Native => ThreadProviderKind::Native,
        thread_service_api::ThreadProviderRuntimeKind::ExternalCli => {
            ThreadProviderKind::ExternalCli
        }
    }
}

fn provider_capabilities_to_api(
    capabilities: thread_service_api::ThreadProviderRuntimeCapabilities,
) -> ThreadProviderCapabilities {
    ThreadProviderCapabilities {
        start_thread: capabilities.start_thread,
        send_input: capabilities.send_input,
        close_thread: capabilities.close_thread,
        list_children: capabilities.list_children,
        restore_thread: capabilities.restore_thread,
        restore_snapshot: capabilities.restore_snapshot,
        event_stream: capabilities.event_stream,
        spawn_child: capabilities.spawn_child,
        compact: capabilities.compact,
        workflow: capabilities.workflow,
        poll_event: capabilities.poll_event,
        command_session: capabilities.command_session,
        permissions: capabilities.permissions,
        dynamic_tools: capabilities.dynamic_tools,
        fork_thread: capabilities.fork_thread,
    }
}

impl CatalogRequestProcessor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new<R>(
        auth_manager: Arc<AuthManager>,
        catalog_runtime: Arc<R>,
        skill_service: SharedSkillServiceApi,
        plugins_manager: Arc<PluginsManager>,
        config: Arc<Config>,
        config_manager: ConfigManager,
        environment_manager: Arc<EnvironmentManager>,
        workspace_settings_cache: Arc<workspace_settings::WorkspaceSettingsCache>,
    ) -> Self
    where
        R: CatalogRuntime + 'static,
    {
        let catalog_runtime: Arc<dyn CatalogRuntime> = catalog_runtime;
        Self {
            auth_manager,
            catalog_runtime,
            skill_service,
            plugins_manager,
            config,
            config_manager,
            environment_manager,
            workspace_settings_cache,
        }
    }

    pub(crate) async fn skills_list(
        &self,
        params: SkillsListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.skills_list_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn hooks_list(
        &self,
        params: HooksListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.hooks_list_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn skills_config_write(
        &self,
        params: SkillsConfigWriteParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.skills_config_write_response_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn model_list(
        &self,
        params: ModelListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.list_models(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn agent_type_list(
        &self,
        params: AgentTypeListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.list_agent_types(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn thread_provider_list(
        &self,
        params: ThreadProviderListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.list_thread_providers(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn experimental_feature_list(
        &self,
        params: ExperimentalFeatureListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.experimental_feature_list_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn collaboration_mode_list(
        &self,
        params: CollaborationModeListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.list_collaboration_modes(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn mock_experimental_method(
        &self,
        params: MockExperimentalMethodParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.mock_experimental_method_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    async fn resolve_cwd_config(
        &self,
        cwd: &Path,
    ) -> Result<(AbsolutePathBuf, ConfigLayerStack), String> {
        let cwd_abs =
            AbsolutePathBuf::relative_to_current_dir(cwd).map_err(|err| err.to_string())?;
        let config_layer_stack = self
            .config_manager
            .load_config_layers_for_cwd(cwd_abs.clone())
            .await
            .map_err(|err| err.to_string())?;

        Ok((cwd_abs, config_layer_stack))
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

    async fn list_models(
        &self,
        params: ModelListParams,
    ) -> Result<ModelListResponse, JSONRPCErrorError> {
        let include_hidden = params.include_hidden.unwrap_or(false);
        let model_catalog = self
            .collect_model_catalog(/*cwd*/ None, include_hidden)
            .await?;
        let CatalogPage { data, next_cursor } =
            CatalogPagination::models(params, model_catalog.models.len())
                .page(&model_catalog.models)?;
        Ok(ModelListResponse { data, next_cursor })
    }

    async fn list_agent_types(
        &self,
        params: AgentTypeListParams,
    ) -> Result<AgentTypeListResponse, JSONRPCErrorError> {
        let AgentTypeListParams { cwd } = params;
        let config = load_latest_config_for_request(&self.config_manager, cwd).await?;
        Ok(AgentTypeListResponse {
            data: native_agent_types(&config),
        })
    }

    async fn list_thread_providers(
        &self,
        params: ThreadProviderListParams,
    ) -> Result<ThreadProviderListResponse, JSONRPCErrorError> {
        let ThreadProviderListParams { cwd } = params;
        let config = load_latest_config_for_request(&self.config_manager, cwd).await?;
        let model_catalog = self
            .collect_model_catalog_from_config(&config, /*include_hidden*/ false)
            .await;
        let snapshot = ThreadProviderCatalogSnapshot {
            runtime_providers: self.catalog_runtime.list_thread_providers(),
            native_agent_types: native_agent_types(&config),
            native_model_providers: model_catalog.model_provider_ids(),
        };
        Ok(snapshot.into_response())
    }

    async fn collect_model_catalog(
        &self,
        cwd: Option<PathBuf>,
        include_hidden: bool,
    ) -> Result<ModelCatalogSnapshot, JSONRPCErrorError> {
        let config = load_latest_config_for_request(&self.config_manager, cwd).await?;
        Ok(self
            .collect_model_catalog_from_config(&config, include_hidden)
            .await)
    }

    async fn collect_model_catalog_from_config(
        &self,
        config: &Config,
        include_hidden: bool,
    ) -> ModelCatalogSnapshot {
        let mut models =
            supported_models(self.catalog_runtime.as_ref(), config, include_hidden).await;
        add_configured_model(&mut models, config);
        ModelCatalogSnapshot::new(models)
    }

    async fn list_collaboration_modes(
        &self,
        params: CollaborationModeListParams,
    ) -> Result<CollaborationModeListResponse, JSONRPCErrorError> {
        let CollaborationModeListParams {} = params;
        let items = self
            .catalog_runtime
            .list_collaboration_modes()
            .into_iter()
            .map(Into::into)
            .collect();
        let response = CollaborationModeListResponse { data: items };
        Ok(response)
    }

    async fn experimental_feature_list_response(
        &self,
        params: ExperimentalFeatureListParams,
    ) -> Result<ExperimentalFeatureListResponse, JSONRPCErrorError> {
        let config =
            load_latest_config_for_request(&self.config_manager, /*fallback_cwd*/ None).await?;
        let auth = self.auth_manager.auth().await;
        let workspace_codex_plugins_enabled = self
            .workspace_codex_plugins_enabled(&config, auth.as_ref())
            .await;

        let feature_catalog =
            FeatureCatalogSnapshot::from_config(&config, workspace_codex_plugins_enabled);
        let CatalogPage { data, next_cursor } =
            CatalogPagination::feature_flags(params, feature_catalog.features.len())
                .page(&feature_catalog.features)?;

        Ok(ExperimentalFeatureListResponse { data, next_cursor })
    }

    async fn mock_experimental_method_inner(
        &self,
        params: MockExperimentalMethodParams,
    ) -> Result<MockExperimentalMethodResponse, JSONRPCErrorError> {
        let MockExperimentalMethodParams { value } = params;
        let response = MockExperimentalMethodResponse { echoed: value };
        Ok(response)
    }

    async fn skills_list_response(
        &self,
        params: SkillsListParams,
    ) -> Result<SkillsListResponse, JSONRPCErrorError> {
        let SkillsListParams { cwds, force_reload } = params;
        let cwds = CatalogCwdTargets::from_requested(cwds, self.config.cwd.as_path()).into_vec();

        let config =
            load_latest_config_for_request(&self.config_manager, /*fallback_cwd*/ None).await?;
        let auth = self.auth_manager.auth().await;
        let workspace_codex_plugins_enabled = self
            .workspace_codex_plugins_enabled(&config, auth.as_ref())
            .await;
        let skill_service = Arc::clone(&self.skill_service);
        let plugins_manager = Arc::clone(&self.plugins_manager);
        let fs = Some(
            self.environment_manager
                .default_environment()
                .unwrap_or_else(|| self.environment_manager.local_environment())
                .get_filesystem(),
        );
        let mut data = futures::stream::iter(cwds.into_iter().enumerate())
            .map(|(index, cwd)| {
                let config = &config;
                let fs = fs.clone();
                let plugins_manager = &plugins_manager;
                let skill_service = &skill_service;
                async move {
                    let (cwd_abs, config_layer_stack) = match self.resolve_cwd_config(&cwd).await {
                        Ok(resolved) => resolved,
                        Err(message) => {
                            let error_path = cwd.clone();
                            return (
                                index,
                                app_server_protocol::SkillsListEntry {
                                    cwd,
                                    skills: Vec::new(),
                                    errors: vec![app_server_protocol::SkillErrorInfo {
                                        path: error_path,
                                        message,
                                    }],
                                },
                            );
                        }
                    };
                    let effective_skill_roots = if workspace_codex_plugins_enabled {
                        let plugins_input = config.plugins_config_input();
                        let plugin_config_layer_stack =
                            thread_service::config::plugin_config_layer_stack_from_config_layer_stack(
                                &config_layer_stack,
                            );
                        plugins_manager
                            .effective_skill_roots_for_layer_stack(
                                &plugin_config_layer_stack,
                                &plugins_input,
                            )
                            .await
                    } else {
                        Vec::new()
                    };
                    let skills_input = SkillsLoadInput::new(
                        cwd_abs.clone(),
                        effective_skill_roots,
                        thread_service::config::skill_config_layer_stack_from_config_layer_stack(
                            &config_layer_stack,
                        ),
                        config.bundled_skills_enabled(),
                    );
                    let outcome = skill_service
                        .skills_for_cwd(&skills_input, force_reload, fs)
                        .await;
                    let errors = errors_to_info(&outcome.errors);
                    let skills = skills_to_info(&outcome.skills, &outcome.disabled_paths);
                    (
                        index,
                        app_server_protocol::SkillsListEntry {
                            cwd,
                            skills,
                            errors,
                        },
                    )
                }
            })
            .buffer_unordered(SKILLS_LIST_CWD_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        data.sort_unstable_by_key(|(index, _)| *index);
        let data = data.into_iter().map(|(_, entry)| entry).collect();
        Ok(SkillsListResponse { data })
    }

    /// Handle `hooks/list` by resolving hooks for each requested cwd.
    async fn hooks_list_response(
        &self,
        params: HooksListParams,
    ) -> Result<HooksListResponse, JSONRPCErrorError> {
        let HooksListParams { cwds } = params;
        let cwds = CatalogCwdTargets::from_requested(cwds, self.config.cwd.as_path()).into_vec();

        let auth = self.auth_manager.auth().await;
        let plugins_manager = Arc::clone(&self.plugins_manager);
        let mut data = Vec::new();
        for cwd in cwds {
            let config = match self
                .config_manager
                .load_for_cwd(
                    /*request_overrides*/ None,
                    ConfigOverrides::default(),
                    Some(cwd.clone()),
                )
                .await
            {
                Ok(config) => config,
                Err(err) => {
                    let error_path = cwd.clone();
                    data.push(app_server_protocol::HooksListEntry {
                        cwd,
                        hooks: Vec::new(),
                        warnings: Vec::new(),
                        errors: vec![app_server_protocol::HookErrorInfo {
                            path: error_path,
                            message: err.to_string(),
                        }],
                    });
                    continue;
                }
            };
            let workspace_codex_plugins_enabled = self
                .workspace_codex_plugins_enabled(&config, auth.as_ref())
                .await;
            let plugins_enabled =
                config.features.enabled(Feature::Plugins) && workspace_codex_plugins_enabled;
            let plugin_outcome = if plugins_enabled && config.features.enabled(Feature::PluginHooks)
            {
                let plugins_input = config.plugins_config_input();
                let plugin_config_layer_stack =
                    thread_service::config::plugin_config_layer_stack_from_config_layer_stack(
                        &config.config_layer_stack,
                    );
                plugins_manager
                    .plugins_for_layer_stack(
                        &plugin_config_layer_stack,
                        &plugins_input,
                        /*plugin_hooks_feature_enabled*/ true,
                    )
                    .await
            } else {
                PluginLoadOutcome::default()
            };
            let hooks = hooks::list_hooks(hooks::HooksConfig {
                feature_enabled: config.features.enabled(Feature::CodexHooks),
                bypass_hook_trust: config.bypass_hook_trust,
                config_layer_stack: Some(
                    thread_service::config::hook_config_layer_stack_from_config_layer_stack(
                        &config.config_layer_stack,
                    ),
                ),
                plugin_hook_sources: plugin_outcome.effective_plugin_hook_sources(),
                plugin_hook_load_warnings: plugin_outcome.effective_plugin_hook_warnings(),
                ..Default::default()
            });
            data.push(app_server_protocol::HooksListEntry {
                cwd,
                hooks: hooks_to_info(&hooks.hooks),
                warnings: hooks.warnings,
                errors: Vec::new(),
            });
        }
        Ok(HooksListResponse { data })
    }

    async fn skills_config_write_response_inner(
        &self,
        params: SkillsConfigWriteParams,
    ) -> Result<SkillsConfigWriteResponse, JSONRPCErrorError> {
        let SkillsConfigWriteParams {
            path,
            name,
            enabled,
        } = params;
        let edit = match (path, name) {
            (Some(path), None) => ConfigEdit::SetSkillConfig {
                path: path.into_path_buf(),
                enabled,
            },
            (None, Some(name)) if !name.trim().is_empty() => {
                ConfigEdit::SetSkillConfigByName { name, enabled }
            }
            _ => {
                return Err(invalid_params(
                    "skills/config/write requires exactly one of path or name",
                ));
            }
        };
        let edits = vec![edit];
        ConfigEditsBuilder::new(&self.config.codex_home)
            .with_edits(edits)
            .apply()
            .await
            .map(|()| {
                self.plugins_manager.clear_cache();
                self.skill_service.clear_cache();
                SkillsConfigWriteResponse {
                    effective_enabled: enabled,
                }
            })
            .map_err(|err| internal_error(format!("failed to update skill settings: {err}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thread_service::config::ConfigBuilder;

    fn runtime_capabilities() -> thread_service_api::ThreadProviderRuntimeCapabilities {
        thread_service_api::ThreadProviderRuntimeCapabilities {
            start_thread: true,
            send_input: true,
            close_thread: true,
            list_children: true,
            restore_thread: false,
            restore_snapshot: true,
            event_stream: true,
            spawn_child: false,
            compact: false,
            workflow: false,
            poll_event: true,
            command_session: false,
            permissions: false,
            dynamic_tools: false,
            fork_thread: false,
        }
    }

    fn runtime_provider(
        id: &str,
        kind: thread_service_api::ThreadProviderRuntimeKind,
    ) -> thread_service_api::ThreadProviderRuntimeDescriptor {
        thread_service_api::ThreadProviderRuntimeDescriptor {
            id: id.to_string(),
            display_name: format!("{id} display"),
            description: format!("{id} description"),
            kind,
            external_root_provider: None,
            capabilities: runtime_capabilities(),
        }
    }

    #[test]
    fn catalog_pagination_preserves_cursor_and_limit_semantics() {
        let items = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        let page = CatalogPagination::models(
            ModelListParams {
                limit: Some(0),
                cursor: Some("1".to_string()),
                include_hidden: None,
            },
            items.len(),
        )
        .page(&items)
        .expect("valid page");

        assert_eq!(page.data, vec!["beta".to_string()]);
        assert_eq!(page.next_cursor, Some("2".to_string()));

        let page = CatalogPagination::models(
            ModelListParams {
                limit: Some(1),
                cursor: Some("3".to_string()),
                include_hidden: None,
            },
            items.len(),
        )
        .page(&items)
        .expect("cursor at total returns empty terminal page");

        assert!(page.data.is_empty());
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn catalog_pagination_preserves_invalid_cursor_errors() {
        let items = vec!["alpha".to_string(), "beta".to_string()];
        let err = CatalogPagination::feature_flags(
            ExperimentalFeatureListParams {
                limit: Some(1),
                cursor: Some("bad".to_string()),
            },
            items.len(),
        )
        .page(&items)
        .expect_err("invalid cursor is rejected");
        assert_eq!(err.code, crate::error_code::INVALID_REQUEST_ERROR_CODE);
        assert_eq!(err.message, "invalid cursor: bad");

        let err = CatalogPagination::feature_flags(
            ExperimentalFeatureListParams {
                limit: Some(1),
                cursor: Some("3".to_string()),
            },
            items.len(),
        )
        .page(&items)
        .expect_err("cursor beyond total is rejected");
        assert_eq!(err.code, crate::error_code::INVALID_REQUEST_ERROR_CODE);
        assert_eq!(err.message, "cursor 3 exceeds total feature flags 2");
    }

    #[tokio::test]
    async fn feature_catalog_snapshot_preserves_workspace_plugin_policy() -> anyhow::Result<()> {
        let codex_home = tempfile::TempDir::new()?;
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .fallback_cwd(Some(codex_home.path().to_path_buf()))
            .build()
            .await?;

        let enabled_snapshot = FeatureCatalogSnapshot::from_config(&config, true);
        let disabled_snapshot = FeatureCatalogSnapshot::from_config(&config, false);

        let apps_when_enabled = enabled_snapshot
            .features
            .iter()
            .find(|feature| feature.name == "apps")
            .expect("apps feature exists");
        let apps_when_disabled = disabled_snapshot
            .features
            .iter()
            .find(|feature| feature.name == "apps")
            .expect("apps feature exists");
        let shell_when_disabled = disabled_snapshot
            .features
            .iter()
            .find(|feature| feature.name == "shell_tool")
            .expect("shell_tool feature exists");

        assert!(apps_when_enabled.default_enabled);
        assert!(apps_when_enabled.enabled);
        assert!(!apps_when_disabled.enabled);
        assert!(shell_when_disabled.enabled);
        assert_eq!(
            apps_when_disabled.stage,
            ApiExperimentalFeatureStage::Stable
        );
        Ok(())
    }

    #[test]
    fn thread_provider_snapshot_preserves_native_and_external_projection() {
        let native_agent_types = vec![AgentType {
            name: "default".to_string(),
            description: Some("Default agent".to_string()),
            built_in: true,
        }];
        let snapshot = ThreadProviderCatalogSnapshot {
            runtime_providers: vec![
                runtime_provider(
                    "native",
                    thread_service_api::ThreadProviderRuntimeKind::Native,
                ),
                runtime_provider(
                    "external",
                    thread_service_api::ThreadProviderRuntimeKind::ExternalCli,
                ),
            ],
            native_agent_types: native_agent_types.clone(),
            native_model_providers: vec!["corp".to_string(), "openai".to_string()],
        };

        let response = snapshot.into_response();
        let native = response
            .data
            .iter()
            .find(|provider| provider.id == "native")
            .expect("native provider projected");
        assert_eq!(native.kind, ThreadProviderKind::Native);
        assert_eq!(native.agent_types, native_agent_types);
        assert_eq!(
            native.model_selection.mode,
            ThreadProviderModelSelectionMode::Catalog
        );
        assert_eq!(
            native.model_selection.model_providers,
            vec!["corp".to_string(), "openai".to_string()]
        );

        let external = response
            .data
            .iter()
            .find(|provider| provider.id == "external")
            .expect("external provider projected");
        assert_eq!(external.kind, ThreadProviderKind::ExternalCli);
        assert!(external.agent_types.is_empty());
        assert_eq!(
            external.model_selection.mode,
            ThreadProviderModelSelectionMode::ProviderDefault
        );
        assert!(external.model_selection.model_providers.is_empty());
        assert!(external.capabilities.start_thread);
        assert!(!external.capabilities.restore_thread);
    }
}
