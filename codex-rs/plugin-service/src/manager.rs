use super::PluginLoadOutcome;
use crate::OPENAI_BUNDLED_MARKETPLACE_NAME;
use crate::OPENAI_CURATED_MARKETPLACE_NAME;
use crate::TOOL_SUGGEST_DISCOVERABLE_PLUGIN_ALLOWLIST;
use crate::config_layers::PluginConfigLayerStack;
use crate::installed_marketplaces::installed_marketplace_roots_from_layer_stack;
use crate::loader::curated_plugin_cache_version;
use crate::loader::installed_plugin_telemetry_metadata;
use crate::loader::load_plugin_apps;
use crate::loader::load_plugin_hooks;
use crate::loader::load_plugin_mcp_servers;
use crate::loader::load_plugin_skills;
use crate::loader::load_plugins_from_layer_stack;
use crate::loader::log_plugin_load_errors;
use crate::loader::materialize_marketplace_plugin_source;
use crate::loader::plugin_telemetry_metadata_from_root;
use crate::loader::refresh_non_curated_plugin_cache;
use crate::loader::refresh_non_curated_plugin_cache_force_reinstall;
use crate::loader::remote_installed_plugins_to_config;
use crate::loader::skill_config_layer_stack_from_config_layer_stack;
use crate::manifest::PluginManifestInterface;
use crate::manifest::load_plugin_manifest;
use crate::marketplace::MarketplaceError;
use crate::marketplace::MarketplaceInterface;
use crate::marketplace::MarketplaceListError;
use crate::marketplace::MarketplacePluginAuthPolicy;
use crate::marketplace::MarketplacePluginPolicy;
use crate::marketplace::MarketplacePluginSource;
use crate::marketplace::ResolvedMarketplacePlugin;
use crate::marketplace::find_installable_marketplace_plugin;
use crate::marketplace::find_marketplace_plugin;
use crate::marketplace::list_marketplaces;
use crate::marketplace::plugin_interface_with_marketplace_category;
use crate::marketplace_upgrade::ConfiguredMarketplaceUpgradeError;
use crate::marketplace_upgrade::ConfiguredMarketplaceUpgradeOutcome;
use crate::marketplace_upgrade::configured_git_marketplace_names;
use crate::marketplace_upgrade::upgrade_configured_git_marketplaces;
use crate::remote_state::RemoteInstalledPlugin;
use crate::startup_sync::curated_plugins_repo_path;
use crate::startup_sync::read_curated_plugins_sha;
use crate::store::PluginInstallResult as StorePluginInstallResult;
use crate::store::PluginStore;
use crate::store::PluginStoreError;
use config_service::clear_user_plugin;
use config_service::set_user_plugin_enabled;
use config_service::version_for_toml;
use codex_config_types::PluginConfig;
use codex_utils_absolute_path::AbsolutePathBuf;
use hooks_api::plugin_hook_declarations;
use plugin_service_api::AppConnectorId;
use plugin_service_api::PluginCapabilitySummary;
use plugin_service_api::PluginId;
use plugin_service_api::PluginIdError;
use plugin_service_api::PluginRuntime;
use plugin_service_api::PluginRuntimeFuture;
use plugin_service_api::PluginSkillRoot;
use plugin_service_api::PluginTelemetryMetadata;
use plugin_service_api::PluginsConfigInput;
use plugin_service_api::ToolSuggestDiscoverablePlugin;
use plugin_service_api::prompt_safe_plugin_description;
use protocol::protocol::HookEventName;
use protocol::protocol::Product;
use skill_service::SkillMetadata;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use tracing::warn;

#[derive(Clone, PartialEq, Eq)]
struct NonCuratedCacheRefreshRequest {
    roots: Vec<AbsolutePathBuf>,
    mode: NonCuratedCacheRefreshMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NonCuratedCacheRefreshMode {
    IfVersionChanged,
    ForceReinstall,
}

#[derive(Default)]
struct NonCuratedCacheRefreshState {
    requested: Option<NonCuratedCacheRefreshRequest>,
    last_refreshed: Option<NonCuratedCacheRefreshRequest>,
    in_flight: bool,
}

#[derive(Default)]
struct ConfiguredMarketplaceUpgradeState {
    in_flight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInstallRequest {
    pub plugin_name: String,
    pub marketplace_path: AbsolutePathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginReadRequest {
    pub plugin_name: String,
    pub marketplace_path: AbsolutePathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInstallOutcome {
    pub plugin_id: PluginId,
    pub plugin_version: String,
    pub installed_path: AbsolutePathBuf,
    pub auth_policy: MarketplacePluginAuthPolicy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginReadOutcome {
    pub marketplace_name: String,
    pub marketplace_path: Option<AbsolutePathBuf>,
    pub plugin: PluginDetail,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginDetail {
    pub id: String,
    pub name: String,
    pub local_version: Option<String>,
    pub description: Option<String>,
    pub source: MarketplacePluginSource,
    pub policy: MarketplacePluginPolicy,
    pub interface: Option<PluginManifestInterface>,
    pub keywords: Vec<String>,
    pub installed: bool,
    pub enabled: bool,
    pub skills: Vec<SkillMetadata>,
    pub disabled_skill_paths: HashSet<AbsolutePathBuf>,
    pub hooks: Vec<PluginHookSummary>,
    pub apps: Vec<AppConnectorId>,
    pub mcp_server_names: Vec<String>,
    pub details_unavailable_reason: Option<PluginDetailsUnavailableReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginHookSummary {
    pub key: String,
    pub event_name: HookEventName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginDetailsUnavailableReason {
    InstallRequiredForRemoteSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredMarketplace {
    pub name: String,
    pub path: AbsolutePathBuf,
    pub interface: Option<MarketplaceInterface>,
    pub plugins: Vec<ConfiguredMarketplacePlugin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredMarketplacePlugin {
    pub id: String,
    pub name: String,
    pub local_version: Option<String>,
    pub source: MarketplacePluginSource,
    pub policy: MarketplacePluginPolicy,
    pub interface: Option<PluginManifestInterface>,
    pub keywords: Vec<String>,
    pub installed: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfiguredMarketplaceListOutcome {
    pub marketplaces: Vec<ConfiguredMarketplace>,
    pub errors: Vec<MarketplaceListError>,
}

impl From<PluginDetail> for PluginCapabilitySummary {
    fn from(value: PluginDetail) -> Self {
        let has_skills = value.skills.iter().any(|skill| {
            !value
                .disabled_skill_paths
                .contains(&skill.path_to_skills_md)
        });
        Self {
            config_name: value.id,
            display_name: value.name,
            description: prompt_safe_plugin_description(value.description.as_deref()),
            has_skills,
            mcp_server_names: value.mcp_server_names,
            app_connector_ids: value.apps,
        }
    }
}

pub struct PluginsManager {
    codex_home: PathBuf,
    store: PluginStore,
    configured_marketplace_upgrade_state: RwLock<ConfiguredMarketplaceUpgradeState>,
    non_curated_cache_refresh_state: RwLock<NonCuratedCacheRefreshState>,
    cached_enabled_outcome: RwLock<Option<CachedPluginLoadOutcome>>,
    remote_installed_plugins_cache: RwLock<Option<Vec<RemoteInstalledPlugin>>>,
    restriction_product: Option<Product>,
    analytics_event_sink: RwLock<Option<Arc<dyn PluginAnalyticsEventSink>>>,
}

/// Receives plugin lifecycle analytics events from `PluginsManager`.
pub trait PluginAnalyticsEventSink: Send + Sync {
    fn track_plugin_installed(&self, plugin: PluginTelemetryMetadata);
    fn track_plugin_uninstalled(&self, plugin: PluginTelemetryMetadata);
}

#[derive(Clone)]
struct CachedPluginLoadOutcome {
    config_version: String,
    plugin_hooks_enabled: bool,
    outcome: PluginLoadOutcome,
}

impl PluginsManager {
    pub fn new(codex_home: PathBuf) -> Self {
        Self::new_with_restriction_product(codex_home, Some(Product::Codex))
    }

    pub fn new_with_restriction_product(
        codex_home: PathBuf,
        restriction_product: Option<Product>,
    ) -> Self {
        // Product restrictions are enforced at marketplace admission time for a given MORPHEUS_HOME:
        // listing, install, and curated refresh all consult this restriction context before new
        // plugins enter local config or cache. After admission, runtime plugin loading trusts the
        // contents of that MORPHEUS_HOME and does not re-filter configured plugins by product, so
        // already-admitted plugins may continue exposing MCP servers/tools from shared local state.
        //
        // This assumes a single MORPHEUS_HOME is only used by one product.
        Self {
            codex_home: codex_home.clone(),
            store: PluginStore::new(codex_home),
            configured_marketplace_upgrade_state: RwLock::new(
                ConfiguredMarketplaceUpgradeState::default(),
            ),
            non_curated_cache_refresh_state: RwLock::new(NonCuratedCacheRefreshState::default()),
            cached_enabled_outcome: RwLock::new(None),
            remote_installed_plugins_cache: RwLock::new(None),
            restriction_product,
            analytics_event_sink: RwLock::new(None),
        }
    }

    pub fn set_plugin_analytics_event_sink(
        &self,
        analytics_event_sink: Arc<dyn PluginAnalyticsEventSink>,
    ) {
        let mut stored_client = match self.analytics_event_sink.write() {
            Ok(client_guard) => client_guard,
            Err(err) => err.into_inner(),
        };
        *stored_client = Some(analytics_event_sink);
    }

    pub fn codex_home(&self) -> &Path {
        self.codex_home.as_path()
    }

    pub fn plugin_store(&self) -> PluginStore {
        self.store.clone()
    }

    pub fn restriction_product(&self) -> Option<Product> {
        self.restriction_product
    }

    pub fn restriction_product_matches(&self, products: Option<&[Product]>) -> bool {
        match products {
            None => true,
            Some([]) => false,
            Some(products) => self
                .restriction_product
                .is_some_and(|product| product.matches_product_restriction(products)),
        }
    }

    pub async fn plugins_for_config(&self, config: &PluginsConfigInput) -> PluginLoadOutcome {
        self.plugins_for_config_with_force_reload(config, /*force_reload*/ false)
            .await
    }

    pub(crate) async fn plugins_for_config_with_force_reload(
        &self,
        config: &PluginsConfigInput,
        force_reload: bool,
    ) -> PluginLoadOutcome {
        if !config.plugins_enabled {
            return PluginLoadOutcome::default();
        }

        let plugin_hooks_enabled = config.plugin_hooks_enabled;
        let config_version = version_for_toml(&config.config_layer_stack.effective_config());
        if !force_reload
            && let Some(outcome) =
                self.cached_enabled_outcome(&config_version, plugin_hooks_enabled)
        {
            return outcome;
        }

        let outcome = load_plugins_from_layer_stack(
            &config.config_layer_stack,
            self.remote_installed_plugin_configs(),
            &self.store,
            self.restriction_product,
            plugin_hooks_enabled,
        )
        .await;
        log_plugin_load_errors(&outcome);
        let mut cache = match self.cached_enabled_outcome.write() {
            Ok(cache) => cache,
            Err(err) => err.into_inner(),
        };
        *cache = Some(CachedPluginLoadOutcome {
            config_version,
            plugin_hooks_enabled,
            outcome: outcome.clone(),
        });
        outcome
    }

    pub fn clear_cache(&self) {
        self.clear_enabled_outcome_cache();
    }

    fn clear_enabled_outcome_cache(&self) {
        let mut cached_enabled_outcome = match self.cached_enabled_outcome.write() {
            Ok(cache) => cache,
            Err(err) => err.into_inner(),
        };
        *cached_enabled_outcome = None;
    }

    /// Load plugins for a config layer stack without touching the plugins cache.
    pub async fn plugins_for_layer_stack(
        &self,
        config_layer_stack: &PluginConfigLayerStack,
        config: &PluginsConfigInput,
        plugin_hooks_feature_enabled: bool,
    ) -> PluginLoadOutcome {
        if !config.plugins_enabled {
            return PluginLoadOutcome::default();
        }
        load_plugins_from_layer_stack(
            config_layer_stack,
            self.remote_installed_plugin_configs(),
            &self.store,
            self.restriction_product,
            plugin_hooks_feature_enabled,
        )
        .await
    }

    /// Resolve plugin skill roots for a config layer stack without touching the plugins cache.
    pub async fn effective_skill_roots_for_layer_stack(
        &self,
        config_layer_stack: &PluginConfigLayerStack,
        config: &PluginsConfigInput,
    ) -> Vec<PluginSkillRoot> {
        self.plugins_for_layer_stack(config_layer_stack, config, config.plugin_hooks_enabled)
            .await
            .effective_plugin_skill_roots()
    }

    fn cached_enabled_outcome(
        &self,
        config_version: &str,
        plugin_hooks_enabled: bool,
    ) -> Option<PluginLoadOutcome> {
        match self.cached_enabled_outcome.read() {
            Ok(cache) => cache
                .as_ref()
                .filter(|cached| {
                    cached.config_version == config_version
                        && cached.plugin_hooks_enabled == plugin_hooks_enabled
                })
                .map(|cached| cached.outcome.clone()),
            Err(err) => err
                .into_inner()
                .as_ref()
                .filter(|cached| {
                    cached.config_version == config_version
                        && cached.plugin_hooks_enabled == plugin_hooks_enabled
                })
                .map(|cached| cached.outcome.clone()),
        }
    }

    fn remote_installed_plugin_configs(&self) -> HashMap<String, PluginConfig> {
        let cache = match self.remote_installed_plugins_cache.read() {
            Ok(cache) => cache,
            Err(err) => err.into_inner(),
        };
        let Some(plugins) = cache.as_ref() else {
            return HashMap::new();
        };

        remote_installed_plugins_to_config(plugins, &self.store)
    }

    pub fn replace_remote_installed_plugins_cache(
        &self,
        plugins: Vec<RemoteInstalledPlugin>,
    ) -> bool {
        let mut cache = match self.remote_installed_plugins_cache.write() {
            Ok(cache) => cache,
            Err(err) => err.into_inner(),
        };
        if cache.as_ref().is_some_and(|cache| cache.eq(&plugins)) {
            return false;
        }
        *cache = Some(plugins);
        drop(cache);
        self.clear_enabled_outcome_cache();
        true
    }

    pub fn clear_remote_installed_plugins_cache(&self) -> bool {
        let mut cache = match self.remote_installed_plugins_cache.write() {
            Ok(cache) => cache,
            Err(err) => err.into_inner(),
        };
        if cache.is_none() {
            return false;
        }
        *cache = None;
        drop(cache);
        self.clear_enabled_outcome_cache();
        true
    }

    pub fn maybe_start_plugin_list_background_tasks_for_config(
        self: &Arc<Self>,
        roots: &[AbsolutePathBuf],
    ) {
        self.maybe_start_non_curated_plugin_cache_refresh(roots);
    }

    pub async fn install_plugin(
        &self,
        request: PluginInstallRequest,
    ) -> Result<PluginInstallOutcome, PluginInstallError> {
        let resolved = find_installable_marketplace_plugin(
            &request.marketplace_path,
            &request.plugin_name,
            self.restriction_product,
        )?;
        self.install_resolved_plugin(resolved).await
    }

    async fn install_resolved_plugin(
        &self,
        resolved: ResolvedMarketplacePlugin,
    ) -> Result<PluginInstallOutcome, PluginInstallError> {
        let auth_policy = resolved.policy.authentication;
        let plugin_version =
            if resolved.plugin_id.marketplace_name == OPENAI_CURATED_MARKETPLACE_NAME {
                let curated_plugin_version = read_curated_plugins_sha(self.codex_home.as_path())
                    .ok_or_else(|| {
                        PluginStoreError::Invalid(
                            "local curated marketplace sha is not available".to_string(),
                        )
                    })?;
                Some(curated_plugin_cache_version(&curated_plugin_version))
            } else {
                None
            };
        let store = self.store.clone();
        let codex_home = self.codex_home.clone();
        let result: StorePluginInstallResult = tokio::task::spawn_blocking(move || {
            let materialized =
                materialize_marketplace_plugin_source(codex_home.as_path(), &resolved.source)
                    .map_err(PluginStoreError::Invalid)?;
            let source_path = materialized.path;
            if let Some(plugin_version) = plugin_version {
                store.install_with_version(source_path, resolved.plugin_id, plugin_version)
            } else {
                store.install(source_path, resolved.plugin_id)
            }
        })
        .await
        .map_err(PluginInstallError::join)??;

        set_user_plugin_enabled(
            &self.codex_home,
            result.plugin_id.as_key(),
            /*enabled*/ true,
        )
        .await
        .map_err(anyhow::Error::from)?;

        let analytics_event_sink = match self.analytics_event_sink.read() {
            Ok(client) => client.clone(),
            Err(err) => err.into_inner().clone(),
        };
        if let Some(analytics_event_sink) = analytics_event_sink {
            analytics_event_sink.track_plugin_installed(
                plugin_telemetry_metadata_from_root(&result.plugin_id, &result.installed_path)
                    .await,
            );
        }

        Ok(PluginInstallOutcome {
            plugin_id: result.plugin_id,
            plugin_version: result.plugin_version,
            installed_path: result.installed_path,
            auth_policy,
        })
    }

    pub async fn uninstall_plugin(&self, plugin_id: String) -> Result<(), PluginUninstallError> {
        let plugin_id = PluginId::parse(&plugin_id)?;
        self.uninstall_plugin_id(plugin_id).await
    }

    async fn uninstall_plugin_id(&self, plugin_id: PluginId) -> Result<(), PluginUninstallError> {
        let plugin_telemetry = if self.store.active_plugin_root(&plugin_id).is_some() {
            Some(installed_plugin_telemetry_metadata(self.codex_home.as_path(), &plugin_id).await)
        } else {
            None
        };
        let store = self.store.clone();
        let plugin_id_for_store = plugin_id.clone();
        tokio::task::spawn_blocking(move || store.uninstall(&plugin_id_for_store))
            .await
            .map_err(PluginUninstallError::join)??;

        clear_user_plugin(&self.codex_home, plugin_id.as_key())
            .await
            .map_err(anyhow::Error::from)?;

        let analytics_event_sink = match self.analytics_event_sink.read() {
            Ok(client) => client.clone(),
            Err(err) => err.into_inner().clone(),
        };
        if let Some(plugin_telemetry) = plugin_telemetry
            && let Some(analytics_event_sink) = analytics_event_sink
        {
            analytics_event_sink.track_plugin_uninstalled(plugin_telemetry);
        }

        Ok(())
    }

    pub fn list_marketplaces_for_config(
        &self,
        config: &PluginsConfigInput,
        additional_roots: &[AbsolutePathBuf],
    ) -> Result<ConfiguredMarketplaceListOutcome, MarketplaceError> {
        if !config.plugins_enabled {
            return Ok(ConfiguredMarketplaceListOutcome::default());
        }

        let (installed_plugins, enabled_plugins) = self.configured_plugin_states(config);
        let marketplace_outcome =
            list_marketplaces(&self.marketplace_roots(config, additional_roots))?;
        let mut seen_plugin_keys = HashSet::new();
        let marketplaces = marketplace_outcome
            .marketplaces
            .into_iter()
            .filter_map(|marketplace| {
                let marketplace_name = marketplace.name.clone();
                let plugins = marketplace
                    .plugins
                    .into_iter()
                    .filter_map(|plugin| {
                        let plugin_key = format!("{}@{marketplace_name}", plugin.name);
                        if !seen_plugin_keys.insert(plugin_key.clone()) {
                            return None;
                        }
                        if !self.restriction_product_matches(plugin.policy.products.as_deref()) {
                            return None;
                        }
                        let installed = installed_plugins.contains(&plugin_key);
                        let enabled = enabled_plugins.contains(&plugin_key);
                        let mut interface = plugin.interface;
                        let mut local_version = plugin.local_version;
                        if installed
                            && matches!(&plugin.source, MarketplacePluginSource::Git { .. })
                            && let Ok(plugin_id) =
                                PluginId::new(plugin.name.clone(), marketplace_name.clone())
                            && let Some(plugin_root) = self.store.active_plugin_root(&plugin_id)
                            && let Some(manifest) = load_plugin_manifest(plugin_root.as_path())
                        {
                            local_version = manifest.version.clone();
                            let marketplace_category = interface
                                .as_ref()
                                .and_then(|interface| interface.category.clone());
                            interface = plugin_interface_with_marketplace_category(
                                manifest.interface,
                                marketplace_category,
                            );
                        }

                        Some(ConfiguredMarketplacePlugin {
                            // Enabled state is keyed by `<plugin>@<marketplace>`, so duplicate
                            // plugin entries from duplicate marketplace files intentionally
                            // resolve to the first discovered source.
                            id: plugin_key,
                            installed,
                            enabled,
                            name: plugin.name,
                            local_version,
                            source: plugin.source,
                            policy: plugin.policy,
                            keywords: plugin.keywords,
                            interface,
                        })
                    })
                    .collect::<Vec<_>>();

                (!plugins.is_empty()).then_some(ConfiguredMarketplace {
                    name: marketplace.name,
                    path: marketplace.path,
                    interface: marketplace.interface,
                    plugins,
                })
            })
            .collect();

        Ok(ConfiguredMarketplaceListOutcome {
            marketplaces,
            errors: marketplace_outcome.errors,
        })
    }

    pub async fn read_plugin_for_config(
        &self,
        config: &PluginsConfigInput,
        request: &PluginReadRequest,
    ) -> Result<PluginReadOutcome, MarketplaceError> {
        if !config.plugins_enabled {
            return Err(MarketplaceError::PluginsDisabled);
        }

        let plugin = find_marketplace_plugin(&request.marketplace_path, &request.plugin_name)?;
        if !self.restriction_product_matches(plugin.policy.products.as_deref()) {
            return Err(MarketplaceError::PluginNotFound {
                plugin_name: plugin.plugin_id.plugin_name,
                marketplace_name: plugin.plugin_id.marketplace_name,
            });
        }

        let marketplace_name = plugin.plugin_id.marketplace_name.clone();
        let plugin_key = plugin.plugin_id.as_key();
        let (installed_plugins, enabled_plugins) = self.configured_plugin_states(config);
        let plugin = self
            .read_plugin_detail_for_marketplace_plugin(
                config,
                &marketplace_name,
                ConfiguredMarketplacePlugin {
                    id: plugin_key.clone(),
                    name: plugin.plugin_id.plugin_name,
                    local_version: plugin
                        .manifest
                        .as_ref()
                        .and_then(|manifest| manifest.version.clone()),
                    source: plugin.source,
                    policy: plugin.policy,
                    interface: plugin.interface,
                    keywords: plugin
                        .manifest
                        .as_ref()
                        .map(|manifest| manifest.keywords.clone())
                        .unwrap_or_default(),
                    installed: installed_plugins.contains(&plugin_key),
                    enabled: enabled_plugins.contains(&plugin_key),
                },
            )
            .await?;

        Ok(PluginReadOutcome {
            marketplace_name,
            marketplace_path: Some(request.marketplace_path.clone()),
            plugin,
        })
    }

    pub async fn read_plugin_detail_for_marketplace_plugin(
        &self,
        config: &PluginsConfigInput,
        marketplace_name: &str,
        plugin: ConfiguredMarketplacePlugin,
    ) -> Result<PluginDetail, MarketplaceError> {
        if !self.restriction_product_matches(plugin.policy.products.as_deref()) {
            return Err(MarketplaceError::PluginNotFound {
                plugin_name: plugin.name,
                marketplace_name: marketplace_name.to_string(),
            });
        }

        let plugin_id =
            PluginId::new(plugin.name.clone(), marketplace_name.to_string()).map_err(|err| {
                match err {
                    PluginIdError::Invalid(message) => MarketplaceError::InvalidPlugin(message),
                }
            })?;
        let plugin_key = plugin_id.as_key();
        if matches!(plugin.source, MarketplacePluginSource::Git { .. }) && !plugin.installed {
            let description = remote_plugin_install_required_description(&plugin.source);
            return Ok(PluginDetail {
                id: plugin_key,
                name: plugin.name,
                local_version: None,
                description: Some(description),
                source: plugin.source,
                policy: plugin.policy,
                interface: plugin.interface,
                keywords: plugin.keywords,
                installed: plugin.installed,
                enabled: plugin.enabled,
                skills: Vec::new(),
                disabled_skill_paths: HashSet::new(),
                hooks: Vec::new(),
                apps: Vec::new(),
                mcp_server_names: Vec::new(),
                details_unavailable_reason: Some(
                    PluginDetailsUnavailableReason::InstallRequiredForRemoteSource,
                ),
            });
        }

        let source_path =
            if matches!(plugin.source, MarketplacePluginSource::Git { .. }) && plugin.installed {
                self.store.active_plugin_root(&plugin_id).ok_or_else(|| {
                    MarketplaceError::InvalidPlugin(format!(
                        "installed plugin cache entry is missing for {plugin_key}"
                    ))
                })?
            } else {
                let codex_home = self.codex_home.clone();
                let source = plugin.source.clone();
                let materialized = tokio::task::spawn_blocking(move || {
                    materialize_marketplace_plugin_source(codex_home.as_path(), &source)
                })
                .await
                .map_err(|err| {
                    MarketplaceError::InvalidPlugin(format!(
                        "failed to materialize plugin source: {err}"
                    ))
                })?
                .map_err(MarketplaceError::InvalidPlugin)?;
                materialized.path.clone()
            };
        if !source_path.as_path().is_dir() {
            return Err(MarketplaceError::InvalidPlugin(
                "path does not exist or is not a directory".to_string(),
            ));
        }
        let manifest = load_plugin_manifest(source_path.as_path()).ok_or_else(|| {
            MarketplaceError::InvalidPlugin("missing or invalid plugin.json".to_string())
        })?;
        let description = manifest.description.clone();
        let marketplace_category = plugin
            .interface
            .as_ref()
            .and_then(|interface| interface.category.clone());
        let interface = plugin_interface_with_marketplace_category(
            manifest.interface.clone(),
            marketplace_category,
        );
        let skill_config_layer_stack =
            skill_config_layer_stack_from_config_layer_stack(&config.config_layer_stack);
        let skill_config_rules =
            skill_service::config_rules::skill_config_rules_from_stack(&skill_config_layer_stack);
        let resolved_skills = load_plugin_skills(
            &source_path,
            &plugin_id,
            &manifest.paths,
            self.restriction_product,
            &skill_config_rules,
        )
        .await;
        let hooks = if config.plugin_hooks_enabled {
            let plugin_data_root = self.store.plugin_data_root(&plugin_id);
            let (hook_sources, _hook_load_warnings) =
                load_plugin_hooks(&source_path, &plugin_id, &plugin_data_root, &manifest.paths);
            plugin_hook_declarations(&hook_sources)
                .into_iter()
                .map(|hook| PluginHookSummary {
                    key: hook.key,
                    event_name: hook.event_name,
                })
                .collect()
        } else {
            Vec::new()
        };
        let apps = load_plugin_apps(source_path.as_path()).await;
        let mut mcp_server_names = load_plugin_mcp_servers(source_path.as_path())
            .await
            .into_keys()
            .collect::<Vec<_>>();
        mcp_server_names.sort_unstable();
        mcp_server_names.dedup();

        Ok(PluginDetail {
            id: plugin.id,
            name: plugin.name,
            local_version: manifest.version.clone(),
            description,
            source: plugin.source,
            policy: plugin.policy,
            interface,
            keywords: manifest.keywords,
            installed: plugin.installed,
            enabled: plugin.enabled,
            skills: resolved_skills.skills,
            disabled_skill_paths: resolved_skills.disabled_skill_paths,
            hooks,
            apps,
            mcp_server_names,
            details_unavailable_reason: None,
        })
    }

    pub fn maybe_start_plugin_startup_tasks_for_config(
        self: &Arc<Self>,
        config: &PluginsConfigInput,
    ) {
        if config.plugins_enabled {
            let should_spawn_marketplace_auto_upgrade = {
                let mut state = match self.configured_marketplace_upgrade_state.write() {
                    Ok(state) => state,
                    Err(err) => err.into_inner(),
                };
                if state.in_flight {
                    false
                } else {
                    state.in_flight = true;
                    true
                }
            };
            if should_spawn_marketplace_auto_upgrade {
                let manager = Arc::clone(self);
                let config = config.clone();
                if let Err(err) = std::thread::Builder::new()
                    .name("plugins-marketplace-auto-upgrade".to_string())
                    .spawn(move || {
                        let outcome = manager.upgrade_configured_marketplaces_for_config(
                            &config, /*marketplace_name*/ None,
                        );
                        match outcome {
                            Ok(outcome) => {
                                for error in outcome.errors {
                                    warn!(
                                        marketplace = error.marketplace_name,
                                        error = %error.message,
                                        "failed to auto-upgrade configured marketplace"
                                    );
                                }
                            }
                            Err(err) => {
                                warn!("failed to auto-upgrade configured marketplaces: {err}");
                            }
                        }

                        let mut state = match manager.configured_marketplace_upgrade_state.write() {
                            Ok(state) => state,
                            Err(err) => err.into_inner(),
                        };
                        state.in_flight = false;
                    })
                {
                    let mut state = match self.configured_marketplace_upgrade_state.write() {
                        Ok(state) => state,
                        Err(err) => err.into_inner(),
                    };
                    state.in_flight = false;
                    warn!("failed to start configured marketplace auto-upgrade task: {err}");
                }
            }
        }
    }

    pub fn upgrade_configured_marketplaces_for_config(
        &self,
        config: &PluginsConfigInput,
        marketplace_name: Option<&str>,
    ) -> Result<ConfiguredMarketplaceUpgradeOutcome, String> {
        if let Some(marketplace_name) = marketplace_name
            && !configured_git_marketplace_names(&config.config_layer_stack)
                .iter()
                .any(|name| name == marketplace_name)
        {
            return Err(format!(
                "marketplace `{marketplace_name}` is not configured as a Git marketplace"
            ));
        }

        let mut outcome = upgrade_configured_git_marketplaces(
            self.codex_home.as_path(),
            &config.config_layer_stack,
            marketplace_name,
        );
        if !outcome.upgraded_roots.is_empty() {
            match refresh_non_curated_plugin_cache_force_reinstall(
                self.codex_home.as_path(),
                &outcome.upgraded_roots,
            ) {
                Ok(cache_refreshed) => {
                    if cache_refreshed {
                        self.clear_cache();
                    }
                }
                Err(err) => {
                    self.clear_cache();
                    outcome.errors.push(ConfiguredMarketplaceUpgradeError {
                        marketplace_name: marketplace_name
                            .unwrap_or("all configured marketplaces")
                            .to_string(),
                        message: format!(
                            "failed to refresh installed plugin cache after marketplace upgrade: {err}"
                        ),
                    });
                }
            }
        }
        Ok(outcome)
    }

    pub fn maybe_start_non_curated_plugin_cache_refresh(
        self: &Arc<Self>,
        roots: &[AbsolutePathBuf],
    ) {
        self.schedule_non_curated_plugin_cache_refresh(
            roots,
            NonCuratedCacheRefreshMode::IfVersionChanged,
        );
    }

    fn schedule_non_curated_plugin_cache_refresh(
        self: &Arc<Self>,
        roots: &[AbsolutePathBuf],
        mode: NonCuratedCacheRefreshMode,
    ) {
        let mut roots = roots.to_vec();
        roots.sort_unstable();
        roots.dedup();
        if roots.is_empty() {
            return;
        }
        let request = NonCuratedCacheRefreshRequest { roots, mode };

        let should_spawn = {
            let mut state = match self.non_curated_cache_refresh_state.write() {
                Ok(state) => state,
                Err(err) => err.into_inner(),
            };
            // Collapse repeated plugin/list requests onto one worker and only queue another pass
            // when the requested roots set actually changes. Forced reinstall requests are not
            // deduped against the last completed pass because the same marketplace root path can
            // point at newly activated files after an auto-upgrade.
            if state.requested.as_ref() == Some(&request)
                || (mode == NonCuratedCacheRefreshMode::IfVersionChanged
                    && !state.in_flight
                    && state.last_refreshed.as_ref() == Some(&request))
            {
                return;
            }
            if mode == NonCuratedCacheRefreshMode::IfVersionChanged
                && state.requested.as_ref().is_some_and(|requested| {
                    requested.mode == NonCuratedCacheRefreshMode::ForceReinstall
                        && requested.roots == request.roots
                })
            {
                return;
            }
            state.requested = Some(request);
            if state.in_flight {
                false
            } else {
                state.in_flight = true;
                true
            }
        };
        if !should_spawn {
            return;
        }

        let manager = Arc::clone(self);
        if let Err(err) = std::thread::Builder::new()
            .name("plugins-non-curated-cache-refresh".to_string())
            .spawn(move || manager.run_non_curated_plugin_cache_refresh_loop())
        {
            let mut state = match self.non_curated_cache_refresh_state.write() {
                Ok(state) => state,
                Err(err) => err.into_inner(),
            };
            state.in_flight = false;
            state.requested = None;
            warn!("failed to start non-curated plugin cache refresh task: {err}");
        }
    }

    fn run_non_curated_plugin_cache_refresh_loop(self: Arc<Self>) {
        loop {
            let request = {
                let state = match self.non_curated_cache_refresh_state.read() {
                    Ok(state) => state,
                    Err(err) => err.into_inner(),
                };
                state.requested.clone()
            };

            let Some(request) = request else {
                let mut state = match self.non_curated_cache_refresh_state.write() {
                    Ok(state) => state,
                    Err(err) => err.into_inner(),
                };
                state.in_flight = false;
                return;
            };

            let refresh_result = match request.mode {
                NonCuratedCacheRefreshMode::IfVersionChanged => {
                    refresh_non_curated_plugin_cache(self.codex_home.as_path(), &request.roots)
                }
                NonCuratedCacheRefreshMode::ForceReinstall => {
                    refresh_non_curated_plugin_cache_force_reinstall(
                        self.codex_home.as_path(),
                        &request.roots,
                    )
                }
            };
            let refreshed = match refresh_result {
                Ok(cache_refreshed) => {
                    if cache_refreshed {
                        self.clear_cache();
                    }
                    true
                }
                Err(err) => {
                    self.clear_cache();
                    warn!("failed to refresh non-curated plugin cache: {err}");
                    false
                }
            };

            let mut state = match self.non_curated_cache_refresh_state.write() {
                Ok(state) => state,
                Err(err) => err.into_inner(),
            };
            if refreshed {
                state.last_refreshed = Some(request.clone());
            }
            if state.requested.as_ref() == Some(&request) {
                state.requested = None;
                state.in_flight = false;
                return;
            }
        }
    }

    fn configured_plugin_states(
        &self,
        config: &PluginsConfigInput,
    ) -> (HashSet<String>, HashSet<String>) {
        let configured_plugins = configured_plugins_from_stack(&config.config_layer_stack);
        let installed_plugins = configured_plugins
            .keys()
            .filter(|plugin_key| {
                PluginId::parse(plugin_key)
                    .ok()
                    .is_some_and(|plugin_id| self.store.is_installed(&plugin_id))
            })
            .cloned()
            .collect::<HashSet<_>>();
        let enabled_plugins = configured_plugins
            .into_iter()
            .filter_map(|(plugin_key, plugin)| plugin.enabled.then_some(plugin_key))
            .collect::<HashSet<_>>();
        (installed_plugins, enabled_plugins)
    }

    fn marketplace_roots(
        &self,
        config: &PluginsConfigInput,
        additional_roots: &[AbsolutePathBuf],
    ) -> Vec<AbsolutePathBuf> {
        // Treat the curated catalog as an extra marketplace root so plugin listing can surface it
        // without requiring every caller to know where it is stored.
        let mut roots = additional_roots.to_vec();
        roots.extend(installed_marketplace_roots_from_layer_stack(
            &config.config_layer_stack,
            self.codex_home.as_path(),
        ));
        let curated_repo_root = curated_plugins_repo_path(self.codex_home.as_path());
        if curated_repo_root.is_dir()
            && let Ok(curated_repo_root) = AbsolutePathBuf::try_from(curated_repo_root)
        {
            roots.push(curated_repo_root);
        }
        roots.sort_unstable();
        roots.dedup();
        roots
    }
}

impl PluginRuntime for PluginsManager {
    fn plugins_for_config<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, PluginLoadOutcome> {
        Box::pin(async move { PluginsManager::plugins_for_config(self, config).await })
    }

    fn is_configured_plugin_installed(&self, config: &PluginsConfigInput, plugin_id: &str) -> bool {
        self.list_marketplaces_for_config(config, &[])
            .ok()
            .into_iter()
            .flat_map(|outcome| outcome.marketplaces)
            .flat_map(|marketplace| marketplace.plugins.into_iter())
            .any(|plugin| plugin.id == plugin_id && plugin.installed)
    }

    fn list_tool_suggest_discoverable_plugins<'a>(
        &'a self,
        config: &'a PluginsConfigInput,
        configured_plugin_ids: &'a HashSet<String>,
        disabled_plugin_ids: &'a HashSet<String>,
    ) -> PluginRuntimeFuture<'a, Result<Vec<ToolSuggestDiscoverablePlugin>, String>> {
        Box::pin(async move {
            const MARKETPLACE_ALLOWLIST: &[&str] = &[
                OPENAI_BUNDLED_MARKETPLACE_NAME,
                OPENAI_CURATED_MARKETPLACE_NAME,
            ];

            let marketplaces = self
                .list_marketplaces_for_config(config, &[])
                .map_err(|err| {
                    format!("failed to list plugin marketplaces for tool suggestions: {err:#}")
                })?
                .marketplaces;
            let mut discoverable_plugins = Vec::<ToolSuggestDiscoverablePlugin>::new();
            for marketplace in marketplaces {
                let marketplace_name = marketplace.name;
                if !MARKETPLACE_ALLOWLIST.contains(&marketplace_name.as_str()) {
                    continue;
                }

                for plugin in marketplace.plugins {
                    if plugin.installed
                        || disabled_plugin_ids.contains(plugin.id.as_str())
                        || (!TOOL_SUGGEST_DISCOVERABLE_PLUGIN_ALLOWLIST
                            .contains(&plugin.id.as_str())
                            && !configured_plugin_ids.contains(plugin.id.as_str()))
                    {
                        continue;
                    }

                    let plugin_id = plugin.id.clone();
                    match self
                        .read_plugin_detail_for_marketplace_plugin(
                            config,
                            &marketplace_name,
                            plugin,
                        )
                        .await
                    {
                        Ok(plugin) => {
                            let plugin = PluginCapabilitySummary::from(plugin);
                            discoverable_plugins.push(ToolSuggestDiscoverablePlugin {
                                id: plugin.config_name,
                                name: plugin.display_name,
                                description: plugin.description,
                                has_skills: plugin.has_skills,
                                mcp_server_names: plugin.mcp_server_names,
                                app_connector_ids: plugin
                                    .app_connector_ids
                                    .into_iter()
                                    .map(|connector_id| connector_id.0)
                                    .collect(),
                            });
                        }
                        Err(err) => {
                            warn!(
                                "failed to load discoverable plugin suggestion {plugin_id}: {err:#}"
                            )
                        }
                    }
                }
            }
            discoverable_plugins.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then_with(|| left.id.cmp(&right.id))
            });
            Ok(discoverable_plugins)
        })
    }

    fn clear_cache(&self) {
        PluginsManager::clear_cache(self);
    }
}

fn remote_plugin_install_required_description(source: &MarketplacePluginSource) -> String {
    let source_description = match source {
        MarketplacePluginSource::Git {
            url,
            path,
            ref_name,
            sha,
        } => {
            let mut parts = vec![url.clone()];
            if let Some(path) = path {
                parts.push(format!("path `{path}`"));
            }
            if let Some(ref_name) = ref_name {
                parts.push(format!("ref `{ref_name}`"));
            }
            if let Some(sha) = sha {
                parts.push(format!("sha `{sha}`"));
            }
            parts.join(", ")
        }
        MarketplacePluginSource::Local { path } => path.as_path().display().to_string(),
    };

    format!(
        "This is a cross-repo plugin. Install it to view more detailed information. The source of the plugin is {source_description}."
    )
}

#[derive(Debug, thiserror::Error)]
pub enum PluginInstallError {
    #[error("{0}")]
    Marketplace(#[from] MarketplaceError),

    #[error("{0}")]
    Store(#[from] PluginStoreError),

    #[error("{0}")]
    Config(#[from] anyhow::Error),

    #[error("failed to join plugin install task: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl PluginInstallError {
    fn join(source: tokio::task::JoinError) -> Self {
        Self::Join(source)
    }

    pub fn is_invalid_request(&self) -> bool {
        matches!(
            self,
            Self::Marketplace(
                MarketplaceError::MarketplaceNotFound { .. }
                    | MarketplaceError::InvalidMarketplaceFile { .. }
                    | MarketplaceError::PluginNotFound { .. }
                    | MarketplaceError::PluginNotAvailable { .. }
                    | MarketplaceError::InvalidPlugin(_)
            ) | Self::Store(PluginStoreError::Invalid(_))
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PluginUninstallError {
    #[error("{0}")]
    InvalidPluginId(#[from] PluginIdError),

    #[error("{0}")]
    Store(#[from] PluginStoreError),

    #[error("{0}")]
    Config(#[from] anyhow::Error),

    #[error("failed to join plugin uninstall task: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl PluginUninstallError {
    fn join(source: tokio::task::JoinError) -> Self {
        Self::Join(source)
    }

    pub fn is_invalid_request(&self) -> bool {
        matches!(self, Self::InvalidPluginId(_))
    }
}

pub fn configured_plugins_from_stack(
    config_layer_stack: &PluginConfigLayerStack,
) -> HashMap<String, PluginConfig> {
    // Plugin entries remain persisted user config only.
    let Some(user_config) = config_layer_stack.effective_user_config() else {
        return HashMap::new();
    };
    configured_plugins_from_user_config_value(&user_config)
}

fn configured_plugins_from_user_config_value(
    user_config: &toml::Value,
) -> HashMap<String, PluginConfig> {
    let Some(plugins_value) = user_config.get("plugins") else {
        return HashMap::new();
    };
    match plugins_value.clone().try_into() {
        Ok(plugins) => plugins,
        Err(err) => {
            warn!("invalid plugins config: {err}");
            HashMap::new()
        }
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
