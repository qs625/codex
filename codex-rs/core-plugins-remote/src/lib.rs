use codex_auth_types::RequestAuthSnapshot;
use codex_config_edit::PluginConfigEdit;
use codex_config_edit::apply_user_plugin_config_edits;
use codex_core_plugins::PluginsConfigInput;
use codex_core_plugins::PluginsManager;
use codex_core_plugins::configured_plugins_from_stack;
use codex_core_plugins::loader::configured_curated_plugin_ids_from_codex_home;
use codex_core_plugins::loader::curated_plugin_cache_version;
use codex_core_plugins::loader::refresh_curated_plugin_cache;
use codex_core_plugins::marketplace::MarketplaceError;
use codex_core_plugins::marketplace::MarketplacePluginSource;
use codex_core_plugins::marketplace::load_marketplace;
use codex_core_plugins::remote_state::RemoteInstalledPlugin;
use codex_core_plugins::startup_sync::curated_plugins_repo_path;
use codex_core_plugins::store::PLUGINS_CACHE_DIR;
use codex_core_plugins::store::PluginStore;
use codex_core_plugins::store::PluginStoreError;
use codex_default_client::build_reqwest_client;
use codex_plugin::PluginId;
use codex_plugin::PluginIdError;
use codex_plugin_types::PluginAuthPolicy;
use codex_plugin_types::PluginAvailability;
use codex_plugin_types::PluginInstallPolicy;
use codex_plugin_types::PluginInterface;
use codex_plugin_types::SkillInterface;
use codex_protocol::protocol::Product;
use codex_utils_absolute_path::AbsolutePathBuf;
use reqwest::RequestBuilder;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fs;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::info;
use tracing::warn;
use url::Url;

pub mod curated_startup_sync;
pub mod remote_bundle;
pub mod remote_legacy;
pub mod startup_remote_sync;

#[path = "remote/remote_installed_plugin_sync.rs"]
mod remote_installed_plugin_sync;
#[path = "remote/share.rs"]
mod share;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod test_support;

pub use remote_installed_plugin_sync::RemoteInstalledPluginBundleSyncError;
pub use remote_installed_plugin_sync::RemoteInstalledPluginBundleSyncOutcome;
pub use remote_installed_plugin_sync::RemotePluginCacheMutationGuard;
pub use remote_installed_plugin_sync::mark_remote_plugin_cache_mutation_in_flight;
pub use remote_installed_plugin_sync::maybe_start_remote_installed_plugin_bundle_sync;
pub use remote_installed_plugin_sync::sync_remote_installed_plugin_bundles_once;
pub use share::RemotePluginShareAccessPolicy;
pub use share::RemotePluginShareDiscoverability;
pub use share::RemotePluginSharePrincipal;
pub use share::RemotePluginSharePrincipalRole;
pub use share::RemotePluginSharePrincipalType;
pub use share::RemotePluginShareSaveResult;
pub use share::RemotePluginShareTarget;
pub use share::RemotePluginShareTargetRole;
pub use share::RemotePluginShareUpdateDiscoverability;
pub use share::RemotePluginShareUpdateTargetsResult;
pub use share::checkout_remote_plugin_share;
pub use share::delete_remote_plugin_share;
pub use share::list_remote_plugin_shares;
pub use share::load_plugin_share_remote_ids_by_local_path;
pub use share::save_remote_plugin_share;
pub use share::update_remote_plugin_share_targets;

pub const REMOTE_GLOBAL_MARKETPLACE_NAME: &str = "chatgpt-global";
pub const REMOTE_WORKSPACE_MARKETPLACE_NAME: &str = "workspace-directory";
pub const REMOTE_WORKSPACE_SHARED_WITH_ME_MARKETPLACE_NAME: &str = "workspace-shared-with-me";
pub const REMOTE_WORKSPACE_SHARED_WITH_ME_PRIVATE_MARKETPLACE_NAME: &str =
    "workspace-shared-with-me-private";
pub const REMOTE_WORKSPACE_SHARED_WITH_ME_UNLISTED_MARKETPLACE_NAME: &str =
    "workspace-shared-with-me-unlisted";
pub const REMOTE_GLOBAL_MARKETPLACE_DISPLAY_NAME: &str = "ChatGPT Plugins";
pub const REMOTE_WORKSPACE_MARKETPLACE_DISPLAY_NAME: &str = "Workspace Directory";
pub const REMOTE_WORKSPACE_SHARED_WITH_ME_PRIVATE_MARKETPLACE_DISPLAY_NAME: &str = "Shared with me";
pub const REMOTE_WORKSPACE_SHARED_WITH_ME_UNLISTED_MARKETPLACE_DISPLAY_NAME: &str =
    "Shared with me (unlisted)";

const REMOTE_PLUGIN_CATALOG_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_PLUGIN_LIST_PAGE_LIMIT: u32 = 200;
const MAX_REMOTE_DEFAULT_PROMPT_LEN: usize = 128;
const INVALID_REMOTE_PLUGIN_ID_MESSAGE: &str =
    "invalid remote plugin id: only ASCII letters, digits, `_`, `-`, and `~` are allowed";
static CURATED_REPO_SYNC_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePluginServiceConfig {
    pub chatgpt_base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePluginAuth {
    request_auth: RequestAuthSnapshot,
    account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    is_workspace_account: bool,
}

impl RemotePluginAuth {
    pub fn new(
        request_auth: RequestAuthSnapshot,
        account_id: Option<String>,
        chatgpt_user_id: Option<String>,
        is_workspace_account: bool,
    ) -> Self {
        Self {
            request_auth,
            account_id,
            chatgpt_user_id,
            is_workspace_account,
        }
    }

    pub fn request_auth_snapshot(&self) -> &RequestAuthSnapshot {
        &self.request_auth
    }

    pub fn uses_codex_backend(&self) -> bool {
        self.request_auth.uses_codex_backend()
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }

    pub fn chatgpt_user_id(&self) -> Option<&str> {
        self.chatgpt_user_id.as_deref()
    }

    pub fn is_workspace_account(&self) -> bool {
        self.is_workspace_account
    }
}

pub type RemotePluginAuthFuture =
    Pin<Box<dyn Future<Output = Option<RemotePluginAuth>> + Send + 'static>>;

/// Supplies the current auth snapshot for remote plugin catalog and sync tasks.
///
/// Implementations are owned by login-aware composition roots. The core plugin
/// runtime only needs request headers and account metadata, not the full login
/// manager or token refresh implementation.
pub trait RemotePluginAuthProvider: Send + Sync {
    fn remote_plugin_auth(&self) -> RemotePluginAuthFuture;
}

pub type SharedRemotePluginAuthProvider = Arc<dyn RemotePluginAuthProvider>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemotePluginSyncResult {
    pub installed_plugin_ids: Vec<String>,
    pub enabled_plugin_ids: Vec<String>,
    pub disabled_plugin_ids: Vec<String>,
    pub uninstalled_plugin_ids: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginRemoteSyncError {
    #[error("chatgpt authentication required to sync remote plugins")]
    AuthRequired,

    #[error(
        "chatgpt authentication required to sync remote plugins; api key auth is not supported"
    )]
    UnsupportedAuthMode,

    #[error("failed to read auth token for remote plugin sync: {0}")]
    AuthToken(#[source] std::io::Error),

    #[error("failed to send remote plugin sync request to {url}: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("remote plugin sync request to {url} failed with status {status}: {body}")]
    UnexpectedStatus {
        url: String,
        status: reqwest::StatusCode,
        body: String,
    },

    #[error("failed to parse remote plugin sync response from {url}: {source}")]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("local curated marketplace is not available")]
    LocalMarketplaceNotFound,

    #[error("remote marketplace `{marketplace_name}` is not available locally")]
    UnknownRemoteMarketplace { marketplace_name: String },

    #[error("duplicate remote plugin `{plugin_name}` in sync response")]
    DuplicateRemotePlugin { plugin_name: String },

    #[error("{0}")]
    InvalidPluginId(#[from] PluginIdError),

    #[error("{0}")]
    Marketplace(#[from] MarketplaceError),

    #[error("{0}")]
    Store(#[from] PluginStoreError),

    #[error("{0}")]
    Config(#[from] anyhow::Error),

    #[error("failed to join remote plugin sync task: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl PluginRemoteSyncError {
    fn join(source: tokio::task::JoinError) -> Self {
        Self::Join(source)
    }
}

impl From<remote_legacy::RemotePluginFetchError> for PluginRemoteSyncError {
    fn from(value: remote_legacy::RemotePluginFetchError) -> Self {
        match value {
            remote_legacy::RemotePluginFetchError::AuthRequired => Self::AuthRequired,
            remote_legacy::RemotePluginFetchError::UnsupportedAuthMode => Self::UnsupportedAuthMode,
            remote_legacy::RemotePluginFetchError::AuthToken(source) => Self::AuthToken(source),
            remote_legacy::RemotePluginFetchError::Request { url, source } => {
                Self::Request { url, source }
            }
            remote_legacy::RemotePluginFetchError::UnexpectedStatus { url, status, body } => {
                Self::UnexpectedStatus { url, status, body }
            }
            remote_legacy::RemotePluginFetchError::Decode { url, source } => {
                Self::Decode { url, source }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum RemoteInstalledPluginsCacheRefreshNotify {
    IfCacheChanged,
    AfterSuccessfulRefresh,
}

fn remote_plugin_service_config(config: &PluginsConfigInput) -> RemotePluginServiceConfig {
    RemotePluginServiceConfig {
        chatgpt_base_url: config.chatgpt_base_url.clone(),
    }
}

pub fn maybe_start_plugin_startup_tasks_for_config(
    manager: Arc<PluginsManager>,
    config: &PluginsConfigInput,
    auth_provider: SharedRemotePluginAuthProvider,
    on_effective_plugins_changed: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
) {
    manager.maybe_start_plugin_startup_tasks_for_config(config);
    if !config.plugins_enabled {
        return;
    }

    start_curated_repo_sync(Arc::clone(&manager));
    startup_remote_sync::start_startup_remote_plugin_sync_once(
        Arc::clone(&manager),
        manager.codex_home().to_path_buf(),
        config.clone(),
        auth_provider.clone(),
    );

    let config_for_remote_sync = config.clone();
    let manager_for_remote_sync = Arc::clone(&manager);
    let auth_provider_for_remote_sync = auth_provider.clone();
    tokio::spawn(async move {
        let auth = auth_provider_for_remote_sync.remote_plugin_auth().await;
        maybe_start_remote_installed_plugins_cache_refresh(
            Arc::clone(&manager_for_remote_sync),
            &config_for_remote_sync,
            auth.clone(),
            on_effective_plugins_changed.clone(),
        );
        maybe_start_remote_installed_plugin_bundle_sync_for_config(
            manager_for_remote_sync,
            &config_for_remote_sync,
            auth,
            on_effective_plugins_changed,
        );
    });

    let config = config.clone();
    tokio::spawn(async move {
        let auth = auth_provider.remote_plugin_auth().await;
        if let Err(err) =
            featured_plugin_ids_for_config(&config, auth.as_ref(), manager.restriction_product())
                .await
        {
            warn!(
                error = %err,
                "failed to warm featured plugin ids cache"
            );
        }
    });
}

pub fn maybe_start_plugin_list_background_tasks_for_config(
    manager: Arc<PluginsManager>,
    config: &PluginsConfigInput,
    auth: Option<RemotePluginAuth>,
    roots: &[AbsolutePathBuf],
    on_effective_plugins_changed: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
) {
    manager.maybe_start_plugin_list_background_tasks_for_config(roots);
    maybe_start_remote_installed_plugins_cache_refresh(
        Arc::clone(&manager),
        config,
        auth.clone(),
        on_effective_plugins_changed.clone(),
    );
    maybe_start_remote_installed_plugin_bundle_sync_for_config(
        manager,
        config,
        auth,
        on_effective_plugins_changed,
    );
}

pub fn maybe_start_remote_installed_plugins_cache_refresh(
    manager: Arc<PluginsManager>,
    config: &PluginsConfigInput,
    auth: Option<RemotePluginAuth>,
    on_effective_plugins_changed: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
) {
    maybe_start_remote_installed_plugins_cache_refresh_with_notify(
        manager,
        config,
        auth,
        RemoteInstalledPluginsCacheRefreshNotify::IfCacheChanged,
        on_effective_plugins_changed,
    );
}

pub fn maybe_start_remote_installed_plugins_cache_refresh_after_mutation(
    manager: Arc<PluginsManager>,
    config: &PluginsConfigInput,
    auth: Option<RemotePluginAuth>,
    on_effective_plugins_changed: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
) {
    maybe_start_remote_installed_plugins_cache_refresh_with_notify(
        manager,
        config,
        auth,
        RemoteInstalledPluginsCacheRefreshNotify::AfterSuccessfulRefresh,
        on_effective_plugins_changed,
    );
}

fn maybe_start_remote_installed_plugins_cache_refresh_with_notify(
    manager: Arc<PluginsManager>,
    config: &PluginsConfigInput,
    auth: Option<RemotePluginAuth>,
    notify: RemoteInstalledPluginsCacheRefreshNotify,
    on_effective_plugins_changed: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
) {
    if !config.plugins_enabled {
        return;
    }

    let service_config = remote_plugin_service_config(config);
    tokio::spawn(async move {
        let installed_plugins =
            fetch_remote_installed_plugins(&service_config, auth.as_ref()).await;
        match installed_plugins {
            Ok(installed_plugins) => {
                let changed = manager.replace_remote_installed_plugins_cache(installed_plugins);
                let should_notify = changed
                    || matches!(
                        notify,
                        RemoteInstalledPluginsCacheRefreshNotify::AfterSuccessfulRefresh
                    );
                if should_notify
                    && let Some(on_effective_plugins_changed) = on_effective_plugins_changed
                {
                    on_effective_plugins_changed();
                }
            }
            Err(
                RemotePluginCatalogError::AuthRequired
                | RemotePluginCatalogError::UnsupportedAuthMode,
            ) => {
                let changed = manager.clear_remote_installed_plugins_cache();
                if changed && let Some(on_effective_plugins_changed) = on_effective_plugins_changed
                {
                    on_effective_plugins_changed();
                }
            }
            Err(err) => {
                warn!(
                    error = %err,
                    "failed to refresh remote installed plugins cache"
                );
            }
        }
    });
}

fn maybe_start_remote_installed_plugin_bundle_sync_for_config(
    manager: Arc<PluginsManager>,
    config: &PluginsConfigInput,
    auth: Option<RemotePluginAuth>,
    on_effective_plugins_changed: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
) {
    if !config.plugins_enabled {
        return;
    }

    let config_for_refresh = config.clone();
    let auth_for_refresh = auth.clone();
    let manager_for_refresh = Arc::clone(&manager);
    let on_local_cache_changed = Arc::new(move || {
        maybe_start_remote_installed_plugins_cache_refresh_after_mutation(
            Arc::clone(&manager_for_refresh),
            &config_for_refresh,
            auth_for_refresh.clone(),
            on_effective_plugins_changed.clone(),
        );
    });

    maybe_start_remote_installed_plugin_bundle_sync(
        manager.codex_home().to_path_buf(),
        remote_plugin_service_config(config),
        auth,
        Some(on_local_cache_changed),
    );
}

pub async fn featured_plugin_ids_for_config(
    config: &PluginsConfigInput,
    auth: Option<&RemotePluginAuth>,
    product: Option<Product>,
) -> Result<Vec<String>, remote_legacy::RemotePluginFetchError> {
    if !config.plugins_enabled {
        return Ok(Vec::new());
    }

    remote_legacy::fetch_remote_featured_plugin_ids(
        &remote_plugin_service_config(config),
        auth,
        product,
    )
    .await
}

fn start_curated_repo_sync(manager: Arc<PluginsManager>) {
    if CURATED_REPO_SYNC_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let codex_home = manager.codex_home().to_path_buf();
    if let Err(err) = std::thread::Builder::new()
        .name("plugins-curated-repo-sync".to_string())
        .spawn(
            move || match curated_startup_sync::sync_openai_plugins_repo(codex_home.as_path()) {
                Ok(curated_plugin_version) => {
                    let configured_curated_plugin_ids =
                        configured_curated_plugin_ids_from_codex_home(codex_home.as_path());
                    match refresh_curated_plugin_cache(
                        codex_home.as_path(),
                        &curated_plugin_version,
                        &configured_curated_plugin_ids,
                    ) {
                        Ok(cache_refreshed) => {
                            if cache_refreshed {
                                manager.clear_cache();
                            }
                        }
                        Err(err) => {
                            manager.clear_cache();
                            CURATED_REPO_SYNC_STARTED.store(false, Ordering::SeqCst);
                            warn!("failed to refresh curated plugin cache after sync: {err}");
                        }
                    }
                }
                Err(err) => {
                    CURATED_REPO_SYNC_STARTED.store(false, Ordering::SeqCst);
                    warn!("failed to sync curated plugins repo: {err}");
                }
            },
        )
    {
        CURATED_REPO_SYNC_STARTED.store(false, Ordering::SeqCst);
        warn!("failed to start curated plugins repo sync task: {err}");
    }
}

pub async fn sync_plugins_from_remote(
    manager: &PluginsManager,
    config: &PluginsConfigInput,
    auth: Option<&RemotePluginAuth>,
    additive_only: bool,
) -> Result<RemotePluginSyncResult, PluginRemoteSyncError> {
    if !config.plugins_enabled {
        return Ok(RemotePluginSyncResult::default());
    }

    info!("starting remote plugin sync");
    let remote_plugins =
        remote_legacy::fetch_remote_plugin_status(&remote_plugin_service_config(config), auth)
            .await
            .map_err(PluginRemoteSyncError::from)?;
    let configured_plugins = configured_plugins_from_stack(&config.config_layer_stack);
    let curated_marketplace_root = curated_plugins_repo_path(manager.codex_home());
    let curated_marketplace_path = AbsolutePathBuf::try_from(
        curated_marketplace_root.join(".agents/plugins/marketplace.json"),
    )
    .map_err(|_| PluginRemoteSyncError::LocalMarketplaceNotFound)?;
    let curated_marketplace = match load_marketplace(&curated_marketplace_path) {
        Ok(marketplace) => marketplace,
        Err(MarketplaceError::MarketplaceNotFound { .. }) => {
            return Err(PluginRemoteSyncError::LocalMarketplaceNotFound);
        }
        Err(err) => return Err(err.into()),
    };

    let marketplace_name = curated_marketplace.name.clone();
    let curated_plugin_version =
        codex_core_plugins::startup_sync::read_curated_plugins_sha(manager.codex_home())
            .ok_or_else(|| {
                PluginStoreError::Invalid(
                    "local curated marketplace sha is not available".to_string(),
                )
            })?;
    let cache_plugin_version = curated_plugin_cache_version(&curated_plugin_version);
    let store = manager.plugin_store();
    let mut local_plugins = Vec::<(
        String,
        PluginId,
        AbsolutePathBuf,
        Option<bool>,
        Option<String>,
        bool,
    )>::new();
    let mut local_plugin_names = HashSet::new();
    for plugin in curated_marketplace.plugins {
        let plugin_name = plugin.name;
        if !local_plugin_names.insert(plugin_name.clone()) {
            warn!(
                plugin = plugin_name,
                marketplace = %marketplace_name,
                "ignoring duplicate local plugin entry during remote sync"
            );
            continue;
        }

        let plugin_id = PluginId::new(plugin_name.clone(), marketplace_name.clone())?;
        let plugin_key = plugin_id.as_key();
        let source_path = match plugin.source {
            MarketplacePluginSource::Local { path } => path,
            MarketplacePluginSource::Git { .. } => {
                warn!(
                    plugin = plugin_name,
                    marketplace = %marketplace_name,
                    "skipping remote plugin source during remote sync"
                );
                continue;
            }
        };
        let current_enabled = configured_plugins
            .get(&plugin_key)
            .map(|plugin| plugin.enabled);
        let installed_version = store.active_plugin_version(&plugin_id);
        let product_allowed =
            manager.restriction_product_matches(plugin.policy.products.as_deref());
        local_plugins.push((
            plugin_name,
            plugin_id,
            source_path,
            current_enabled,
            installed_version,
            product_allowed,
        ));
    }

    let mut missing_remote_plugins = Vec::<String>::new();
    let mut remote_installed_plugin_names = HashSet::<String>::new();
    for plugin in remote_plugins {
        if plugin.marketplace_name != marketplace_name {
            return Err(PluginRemoteSyncError::UnknownRemoteMarketplace {
                marketplace_name: plugin.marketplace_name,
            });
        }
        if !local_plugin_names.contains(&plugin.name) {
            missing_remote_plugins.push(plugin.name);
            continue;
        }
        if !plugin.enabled {
            continue;
        }
        if !remote_installed_plugin_names.insert(plugin.name.clone()) {
            return Err(PluginRemoteSyncError::DuplicateRemotePlugin {
                plugin_name: plugin.name,
            });
        }
    }

    let mut config_edits = Vec::new();
    let mut installs = Vec::new();
    let mut uninstalls = Vec::new();
    let mut result = RemotePluginSyncResult::default();
    let remote_plugin_count = remote_installed_plugin_names.len();
    let local_plugin_count = local_plugins.len();
    if !missing_remote_plugins.is_empty() {
        let sample_missing_plugins = missing_remote_plugins
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>();
        warn!(
            marketplace = %marketplace_name,
            missing_remote_plugin_count = missing_remote_plugins.len(),
            missing_remote_plugin_examples = ?sample_missing_plugins,
            "ignoring remote plugins missing from local marketplace during sync"
        );
    }

    for (
        plugin_name,
        plugin_id,
        source_path,
        current_enabled,
        installed_version,
        product_allowed,
    ) in local_plugins
    {
        let plugin_key = plugin_id.as_key();
        let is_installed = installed_version.is_some();
        if !product_allowed {
            continue;
        }
        if remote_installed_plugin_names.contains(&plugin_name) {
            if !is_installed {
                installs.push((source_path, plugin_id.clone(), cache_plugin_version.clone()));
                result.installed_plugin_ids.push(plugin_key.clone());
            }

            if current_enabled != Some(true) {
                result.enabled_plugin_ids.push(plugin_key.clone());
                config_edits.push(PluginConfigEdit::SetEnabled {
                    plugin_key,
                    enabled: true,
                });
            }
        } else if !additive_only {
            if is_installed {
                uninstalls.push(plugin_id);
            }
            if is_installed || current_enabled.is_some() {
                result.uninstalled_plugin_ids.push(plugin_key.clone());
            }
            if current_enabled.is_some() {
                config_edits.push(PluginConfigEdit::Clear { plugin_key });
            }
        }
    }

    let store_result = tokio::task::spawn_blocking(move || {
        for (source_path, plugin_id, plugin_version) in installs {
            store.install_with_version(source_path, plugin_id, plugin_version)?;
        }
        for plugin_id in uninstalls {
            store.uninstall(&plugin_id)?;
        }
        Ok::<(), PluginStoreError>(())
    })
    .await
    .map_err(PluginRemoteSyncError::join)?;
    if let Err(err) = store_result {
        manager.clear_cache();
        return Err(err.into());
    }

    let config_result = if config_edits.is_empty() {
        Ok(())
    } else {
        apply_user_plugin_config_edits(manager.codex_home(), config_edits).await
    };
    manager.clear_cache();
    config_result.map_err(anyhow::Error::from)?;

    info!(
        marketplace = %marketplace_name,
        remote_plugin_count,
        local_plugin_count,
        installed_plugin_ids = ?result.installed_plugin_ids,
        enabled_plugin_ids = ?result.enabled_plugin_ids,
        disabled_plugin_ids = ?result.disabled_plugin_ids,
        uninstalled_plugin_ids = ?result.uninstalled_plugin_ids,
        "completed remote plugin sync"
    );

    Ok(result)
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteMarketplace {
    pub name: String,
    pub display_name: String,
    pub plugins: Vec<RemotePluginSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteMarketplaceSource {
    Global,
    WorkspaceDirectory,
    SharedWithMe,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemotePluginSummary {
    pub id: String,
    pub remote_plugin_id: String,
    pub name: String,
    pub share_context: Option<RemotePluginShareContext>,
    pub installed: bool,
    pub enabled: bool,
    pub install_policy: PluginInstallPolicy,
    pub auth_policy: PluginAuthPolicy,
    pub availability: PluginAvailability,
    pub interface: Option<PluginInterface>,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePluginShareContext {
    pub remote_plugin_id: String,
    pub remote_version: Option<String>,
    pub discoverability: RemotePluginShareDiscoverability,
    pub share_url: Option<String>,
    pub creator_account_user_id: Option<String>,
    pub creator_name: Option<String>,
    pub share_principals: Option<Vec<RemotePluginSharePrincipal>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemotePluginShareSummary {
    pub summary: RemotePluginSummary,
    pub local_plugin_path: Option<AbsolutePathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemotePluginDetail {
    pub marketplace_name: String,
    pub marketplace_display_name: String,
    pub summary: RemotePluginSummary,
    pub description: Option<String>,
    pub release_version: Option<String>,
    pub bundle_download_url: Option<String>,
    pub skills: Vec<RemotePluginSkill>,
    pub app_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemotePluginSkill {
    pub name: String,
    pub description: String,
    pub short_description: Option<String>,
    pub interface: Option<SkillInterface>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemotePluginSkillDetail {
    pub contents: Option<String>,
}

pub fn is_valid_remote_plugin_id(plugin_id: &str) -> bool {
    !plugin_id.is_empty()
        && plugin_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '~')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{INVALID_REMOTE_PLUGIN_ID_MESSAGE}")]
pub struct RemotePluginIdError;

pub fn validate_remote_plugin_id(plugin_id: &str) -> Result<(), RemotePluginIdError> {
    if !is_valid_remote_plugin_id(plugin_id) {
        return Err(RemotePluginIdError);
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum RemotePluginCatalogError {
    #[error("chatgpt authentication required for remote plugin catalog")]
    AuthRequired,

    #[error(
        "chatgpt authentication required for remote plugin catalog; api key auth is not supported"
    )]
    UnsupportedAuthMode,

    #[error("failed to read auth token for remote plugin catalog: {0}")]
    AuthToken(#[source] std::io::Error),

    #[error("failed to send remote plugin catalog request to {url}: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("remote plugin catalog request to {url} failed with status {status}: {body}")]
    UnexpectedStatus {
        url: String,
        status: reqwest::StatusCode,
        body: String,
    },

    #[error("failed to parse remote plugin catalog response from {url}: {source}")]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid remote plugin catalog base URL: {0}")]
    InvalidBaseUrl(#[source] url::ParseError),

    #[error("invalid remote plugin catalog base URL path")]
    InvalidBaseUrlPath,

    #[error("remote marketplace `{marketplace_name}` is not supported")]
    UnknownMarketplace { marketplace_name: String },

    #[error(
        "remote plugin mutation returned unexpected plugin id: expected `{expected}`, got `{actual}`"
    )]
    UnexpectedPluginId { expected: String, actual: String },

    #[error(
        "remote plugin skill response returned unexpected skill name: expected `{expected}`, got `{actual}`"
    )]
    UnexpectedSkillName { expected: String, actual: String },

    #[error(
        "remote plugin mutation returned unexpected enabled state for `{plugin_id}`: expected {expected_enabled}, got {actual_enabled}"
    )]
    UnexpectedEnabledState {
        plugin_id: String,
        expected_enabled: bool,
        actual_enabled: bool,
    },

    #[error("invalid plugin path `{path}`: {reason}")]
    InvalidPluginPath { path: PathBuf, reason: String },

    #[error("remote plugin `{remote_plugin_id}` is not available for plugin/share/checkout")]
    PluginShareCheckoutNotAvailable { remote_plugin_id: String },

    #[error("failed to archive plugin at `{path}`: {source}")]
    Archive {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to join plugin archive task: {0}")]
    ArchiveJoin(#[source] tokio::task::JoinError),

    #[error(
        "plugin archive would be {bytes} bytes, exceeding the maximum upload size of {max_bytes} bytes"
    )]
    ArchiveTooLarge { bytes: usize, max_bytes: usize },

    #[error("workspace plugin upload response did not include an etag")]
    MissingUploadEtag,

    #[error("{0}")]
    UnexpectedResponse(String),

    #[error("{0}")]
    CacheRemove(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
enum RemotePluginScope {
    #[serde(rename = "GLOBAL")]
    Global,
    #[serde(rename = "WORKSPACE")]
    Workspace,
}

impl RemotePluginScope {
    fn api_value(self) -> &'static str {
        match self {
            Self::Global => "GLOBAL",
            Self::Workspace => "WORKSPACE",
        }
    }

    fn marketplace_name(self) -> &'static str {
        match self {
            Self::Global => REMOTE_GLOBAL_MARKETPLACE_NAME,
            Self::Workspace => REMOTE_WORKSPACE_MARKETPLACE_NAME,
        }
    }

    fn marketplace_display_name(self) -> &'static str {
        match self {
            Self::Global => REMOTE_GLOBAL_MARKETPLACE_DISPLAY_NAME,
            Self::Workspace => REMOTE_WORKSPACE_MARKETPLACE_DISPLAY_NAME,
        }
    }

    fn from_marketplace_name(name: &str) -> Option<Self> {
        match name {
            REMOTE_GLOBAL_MARKETPLACE_NAME => Some(Self::Global),
            REMOTE_WORKSPACE_MARKETPLACE_NAME
            | REMOTE_WORKSPACE_SHARED_WITH_ME_MARKETPLACE_NAME
            | REMOTE_WORKSPACE_SHARED_WITH_ME_PRIVATE_MARKETPLACE_NAME
            | REMOTE_WORKSPACE_SHARED_WITH_ME_UNLISTED_MARKETPLACE_NAME => Some(Self::Workspace),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginPagination {
    next_page_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginSkillInterfaceResponse {
    display_name: Option<String>,
    short_description: Option<String>,
    brand_color: Option<String>,
    default_prompt: Option<String>,
    icon_small_url: Option<String>,
    icon_large_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginSkillResponse {
    name: String,
    description: String,
    interface: Option<RemotePluginSkillInterfaceResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginSkillDetailResponse {
    plugin_id: String,
    name: String,
    skill_md_contents: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginReleaseInterfaceResponse {
    short_description: Option<String>,
    long_description: Option<String>,
    developer_name: Option<String>,
    category: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    website_url: Option<String>,
    privacy_policy_url: Option<String>,
    terms_of_service_url: Option<String>,
    brand_color: Option<String>,
    default_prompt: Option<String>,
    composer_icon_url: Option<String>,
    logo_url: Option<String>,
    #[serde(default)]
    screenshot_urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginReleaseResponse {
    #[serde(default)]
    version: Option<String>,
    display_name: String,
    description: String,
    #[serde(default)]
    bundle_download_url: Option<String>,
    #[serde(default)]
    app_ids: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    interface: RemotePluginReleaseInterfaceResponse,
    #[serde(default)]
    skills: Vec<RemotePluginSkillResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginDirectoryItem {
    id: String,
    name: String,
    scope: RemotePluginScope,
    #[serde(default)]
    discoverability: Option<RemotePluginShareDiscoverability>,
    #[serde(default)]
    creator_account_user_id: Option<String>,
    #[serde(default)]
    creator_name: Option<String>,
    #[serde(default)]
    share_url: Option<String>,
    #[serde(default)]
    share_principals: Option<Vec<RemotePluginDirectorySharePrincipal>>,
    installation_policy: PluginInstallPolicy,
    authentication_policy: PluginAuthPolicy,
    #[serde(rename = "status", default)]
    availability: PluginAvailability,
    release: RemotePluginReleaseResponse,
}

fn remote_plugin_canonical_marketplace_name(
    plugin: &RemotePluginDirectoryItem,
) -> Result<&'static str, RemotePluginCatalogError> {
    match plugin.scope {
        RemotePluginScope::Global => Ok(REMOTE_GLOBAL_MARKETPLACE_NAME),
        RemotePluginScope::Workspace => match workspace_plugin_discoverability(plugin)? {
            RemotePluginShareDiscoverability::Listed => Ok(REMOTE_WORKSPACE_MARKETPLACE_NAME),
            RemotePluginShareDiscoverability::Private
            | RemotePluginShareDiscoverability::Unlisted => {
                Ok(REMOTE_WORKSPACE_SHARED_WITH_ME_MARKETPLACE_NAME)
            }
        },
    }
}

fn workspace_plugin_discoverability(
    plugin: &RemotePluginDirectoryItem,
) -> Result<RemotePluginShareDiscoverability, RemotePluginCatalogError> {
    plugin.discoverability.ok_or_else(|| {
        RemotePluginCatalogError::UnexpectedResponse(format!(
            "workspace plugin `{}` did not include discoverability",
            plugin.id
        ))
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginDirectorySharePrincipal {
    principal_type: RemotePluginSharePrincipalType,
    principal_id: String,
    role: RemotePluginSharePrincipalRole,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginInstalledItem {
    #[serde(flatten)]
    plugin: RemotePluginDirectoryItem,
    enabled: bool,
    #[serde(default)]
    disabled_skill_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginListResponse {
    plugins: Vec<RemotePluginDirectoryItem>,
    pagination: RemotePluginPagination,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginInstalledResponse {
    plugins: Vec<RemotePluginInstalledItem>,
    pagination: RemotePluginPagination,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RemotePluginMutationResponse {
    id: String,
    enabled: bool,
}

pub async fn fetch_remote_marketplaces(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
    sources: &[RemoteMarketplaceSource],
) -> Result<Vec<RemoteMarketplace>, RemotePluginCatalogError> {
    let auth = ensure_chatgpt_auth(auth)?;
    let mut marketplaces = Vec::new();
    let needs_workspace_installed = sources.iter().any(|source| {
        matches!(
            source,
            RemoteMarketplaceSource::WorkspaceDirectory | RemoteMarketplaceSource::SharedWithMe
        )
    });
    let workspace_installed_plugins = if needs_workspace_installed {
        Some(fetch_installed_plugins_for_scope(config, auth, RemotePluginScope::Workspace).await?)
    } else {
        None
    };

    for source in sources {
        match source {
            RemoteMarketplaceSource::Global => {
                let scope = RemotePluginScope::Global;
                let (directory_plugins, installed_plugins) = tokio::try_join!(
                    fetch_directory_plugins_for_scope(config, auth, scope),
                    fetch_installed_plugins_for_scope(config, auth, scope),
                )?;
                if let Some(marketplace) = build_remote_marketplace(
                    scope.marketplace_name(),
                    scope.marketplace_display_name(),
                    directory_plugins,
                    installed_plugins,
                    /*include_installed_only*/ true,
                )? {
                    marketplaces.push(marketplace);
                }
            }
            RemoteMarketplaceSource::WorkspaceDirectory => {
                let scope = RemotePluginScope::Workspace;
                let directory_plugins =
                    fetch_directory_plugins_for_scope(config, auth, scope).await?;
                if let Some(marketplace) = build_remote_marketplace(
                    scope.marketplace_name(),
                    scope.marketplace_display_name(),
                    directory_plugins,
                    workspace_installed_plugins.clone().unwrap_or_default(),
                    /*include_installed_only*/ false,
                )? {
                    marketplaces.push(marketplace);
                }
            }
            RemoteMarketplaceSource::SharedWithMe => {
                // The shared endpoint is the source of truth for plugins explicitly shared
                // with the user. Installed unlisted plugins that are not returned there are
                // link-installed and stay in the separate unlisted bucket.
                let shared_plugins = fetch_shared_workspace_plugins(config, auth).await?;
                let shared_plugin_ids = shared_plugins
                    .iter()
                    .map(|plugin| plugin.id.clone())
                    .collect::<BTreeSet<_>>();
                let directly_shared_plugins = shared_plugins
                    .into_iter()
                    .filter_map(|plugin| match workspace_plugin_discoverability(&plugin) {
                        Ok(
                            RemotePluginShareDiscoverability::Private
                            | RemotePluginShareDiscoverability::Unlisted,
                        ) => Some(Ok(plugin)),
                        Ok(RemotePluginShareDiscoverability::Listed) => None,
                        Err(err) => Some(Err(err)),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(marketplace) = build_remote_marketplace(
                    REMOTE_WORKSPACE_SHARED_WITH_ME_PRIVATE_MARKETPLACE_NAME,
                    REMOTE_WORKSPACE_SHARED_WITH_ME_PRIVATE_MARKETPLACE_DISPLAY_NAME,
                    directly_shared_plugins,
                    workspace_installed_plugins.clone().unwrap_or_default(),
                    /*include_installed_only*/ false,
                )? {
                    marketplaces.push(marketplace);
                }

                let unlisted_installed_plugins = workspace_installed_plugins
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(
                        |plugin| match workspace_plugin_discoverability(&plugin.plugin) {
                            Ok(RemotePluginShareDiscoverability::Unlisted)
                                if !shared_plugin_ids.contains(&plugin.plugin.id) =>
                            {
                                Some(Ok(plugin))
                            }
                            Ok(RemotePluginShareDiscoverability::Unlisted) => None,
                            Ok(RemotePluginShareDiscoverability::Listed)
                            | Ok(RemotePluginShareDiscoverability::Private) => None,
                            Err(err) => Some(Err(err)),
                        },
                    )
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(marketplace) = build_remote_marketplace(
                    REMOTE_WORKSPACE_SHARED_WITH_ME_UNLISTED_MARKETPLACE_NAME,
                    REMOTE_WORKSPACE_SHARED_WITH_ME_UNLISTED_MARKETPLACE_DISPLAY_NAME,
                    Vec::new(),
                    unlisted_installed_plugins,
                    /*include_installed_only*/ true,
                )? {
                    marketplaces.push(marketplace);
                }
            }
        }
    }

    Ok(marketplaces)
}

fn build_remote_marketplace(
    name: &str,
    display_name: &str,
    directory_plugins: Vec<RemotePluginDirectoryItem>,
    installed_plugins: Vec<RemotePluginInstalledItem>,
    include_installed_only: bool,
) -> Result<Option<RemoteMarketplace>, RemotePluginCatalogError> {
    let directory_plugins = directory_plugins
        .into_iter()
        .map(|plugin| (plugin.id.clone(), plugin))
        .collect::<BTreeMap<_, _>>();
    let installed_plugins = installed_plugins
        .into_iter()
        .map(|plugin| (plugin.plugin.id.clone(), plugin))
        .collect::<BTreeMap<_, _>>();
    let plugin_ids = directory_plugins
        .keys()
        .chain(
            include_installed_only
                .then_some(&installed_plugins)
                .into_iter()
                .flat_map(|plugins| plugins.keys()),
        )
        .cloned()
        .collect::<BTreeSet<_>>();
    if plugin_ids.is_empty() {
        return Ok(None);
    }

    let mut plugins = plugin_ids
        .into_iter()
        .filter_map(|plugin_id| {
            let directory_plugin = directory_plugins.get(&plugin_id);
            let installed_plugin = installed_plugins.get(&plugin_id);
            directory_plugin
                .or_else(|| installed_plugin.map(|plugin| &plugin.plugin))
                .map(|plugin| (plugin, installed_plugin))
        })
        .map(|(plugin, installed_plugin)| build_remote_plugin_summary(plugin, installed_plugin))
        .collect::<Result<Vec<_>, _>>()?;
    plugins.sort_by(|left, right| {
        remote_plugin_display_name(left)
            .to_ascii_lowercase()
            .cmp(&remote_plugin_display_name(right).to_ascii_lowercase())
            .then_with(|| remote_plugin_display_name(left).cmp(remote_plugin_display_name(right)))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(Some(RemoteMarketplace {
        name: name.to_string(),
        display_name: display_name.to_string(),
        plugins,
    }))
}

pub async fn fetch_remote_installed_plugins(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
) -> Result<Vec<RemoteInstalledPlugin>, RemotePluginCatalogError> {
    let auth = ensure_chatgpt_auth(auth)?;
    let global = async {
        let scope = RemotePluginScope::Global;
        let installed_plugins = fetch_installed_plugins_for_scope(config, auth, scope).await?;
        Ok::<_, RemotePluginCatalogError>((scope, installed_plugins))
    };
    let workspace = async {
        let scope = RemotePluginScope::Workspace;
        let installed_plugins = fetch_installed_plugins_for_scope(config, auth, scope).await?;
        Ok::<_, RemotePluginCatalogError>((scope, installed_plugins))
    };

    let (global, workspace) = tokio::try_join!(global, workspace)?;
    let mut installed_plugins = [global, workspace]
        .into_iter()
        .flat_map(|(_scope, plugins)| plugins)
        .map(|plugin| remote_installed_plugin_to_info(&plugin))
        .collect::<Result<Vec<_>, _>>()?;
    installed_plugins.sort_by(|left, right| {
        left.marketplace_name
            .cmp(&right.marketplace_name)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(installed_plugins)
}

pub async fn fetch_remote_plugin_detail(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
    marketplace_name: &str,
    plugin_id: &str,
) -> Result<RemotePluginDetail, RemotePluginCatalogError> {
    fetch_remote_plugin_detail_with_download_url_option(
        config,
        auth,
        marketplace_name,
        plugin_id,
        /*include_download_urls*/ false,
    )
    .await
}

pub async fn fetch_remote_plugin_share_context(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
    plugin_id: &str,
) -> Result<Option<RemotePluginShareContext>, RemotePluginCatalogError> {
    let auth = ensure_chatgpt_auth(auth)?;
    let plugin = fetch_plugin_detail(
        config, auth, plugin_id, /*include_download_urls*/ false,
    )
    .await?;
    remote_plugin_share_context(&plugin)
}

pub async fn fetch_remote_plugin_detail_with_download_urls(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
    marketplace_name: &str,
    plugin_id: &str,
) -> Result<RemotePluginDetail, RemotePluginCatalogError> {
    fetch_remote_plugin_detail_with_download_url_option(
        config,
        auth,
        marketplace_name,
        plugin_id,
        /*include_download_urls*/ true,
    )
    .await
}

pub async fn fetch_remote_plugin_skill_detail(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
    marketplace_name: &str,
    plugin_id: &str,
    skill_name: &str,
) -> Result<RemotePluginSkillDetail, RemotePluginCatalogError> {
    let auth = ensure_chatgpt_auth(auth)?;
    if RemotePluginScope::from_marketplace_name(marketplace_name).is_none() {
        return Err(RemotePluginCatalogError::UnknownMarketplace {
            marketplace_name: marketplace_name.to_string(),
        });
    }

    let url = remote_plugin_skill_detail_url(config, plugin_id, skill_name)?;
    let client = build_reqwest_client();
    let request = authenticated_request(client.get(&url), auth)?;
    let response: RemotePluginSkillDetailResponse = send_and_decode(request, &url).await?;
    if response.plugin_id != plugin_id {
        return Err(RemotePluginCatalogError::UnexpectedPluginId {
            expected: plugin_id.to_string(),
            actual: response.plugin_id,
        });
    }
    if response.name != skill_name {
        return Err(RemotePluginCatalogError::UnexpectedSkillName {
            expected: skill_name.to_string(),
            actual: response.name,
        });
    }

    Ok(RemotePluginSkillDetail {
        contents: response.skill_md_contents,
    })
}

async fn fetch_remote_plugin_detail_with_download_url_option(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
    _marketplace_name: &str,
    plugin_id: &str,
    include_download_urls: bool,
) -> Result<RemotePluginDetail, RemotePluginCatalogError> {
    let auth = ensure_chatgpt_auth(auth)?;
    let plugin = fetch_plugin_detail(config, auth, plugin_id, include_download_urls).await?;
    let scope = plugin.scope;
    let marketplace_name = remote_plugin_canonical_marketplace_name(&plugin)?.to_string();
    // Remote plugin IDs uniquely identify remote plugins, so the caller-provided
    // marketplace name is not validated here. The backend detail response is the
    // source of truth for the plugin's actual scope/marketplace.

    build_remote_plugin_detail(config, auth, scope, marketplace_name, plugin_id, plugin).await
}

async fn build_remote_plugin_detail(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
    scope: RemotePluginScope,
    marketplace_name: String,
    plugin_id: &str,
    plugin: RemotePluginDirectoryItem,
) -> Result<RemotePluginDetail, RemotePluginCatalogError> {
    let installed_plugin = fetch_installed_plugins_for_scope(config, auth, scope)
        .await?
        .into_iter()
        .find(|installed_plugin| installed_plugin.plugin.id == plugin_id);
    let disabled_skill_names = installed_plugin
        .as_ref()
        .map(|plugin| {
            plugin
                .disabled_skill_names
                .iter()
                .cloned()
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let skills = plugin
        .release
        .skills
        .iter()
        .map(|skill| RemotePluginSkill {
            name: skill.name.clone(),
            description: skill.description.clone(),
            short_description: skill
                .interface
                .as_ref()
                .and_then(|interface| interface.short_description.clone()),
            interface: remote_skill_interface_to_info(skill.interface.clone()),
            enabled: !disabled_skill_names.contains(&skill.name),
        })
        .collect();

    Ok(RemotePluginDetail {
        marketplace_name,
        marketplace_display_name: scope.marketplace_display_name().to_string(),
        summary: build_remote_plugin_summary(&plugin, installed_plugin.as_ref())?,
        description: non_empty_string(Some(&plugin.release.description)),
        release_version: plugin.release.version,
        bundle_download_url: plugin.release.bundle_download_url,
        skills,
        app_ids: plugin.release.app_ids,
    })
}

pub async fn install_remote_plugin(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
    _marketplace_name: &str,
    plugin_id: &str,
) -> Result<(), RemotePluginCatalogError> {
    let auth = ensure_chatgpt_auth(auth)?;
    // Remote plugin IDs uniquely identify remote plugins, so the caller-provided
    // marketplace name is not validated before sending the install mutation.

    let base_url = config.chatgpt_base_url.trim_end_matches('/');
    let url = format!("{base_url}/ps/plugins/{plugin_id}/install");
    let client = build_reqwest_client();
    let request = authenticated_request(client.post(&url), auth)?;
    let response: RemotePluginMutationResponse = send_and_decode(request, &url).await?;
    if response.id != plugin_id {
        return Err(RemotePluginCatalogError::UnexpectedPluginId {
            expected: plugin_id.to_string(),
            actual: response.id,
        });
    }
    if !response.enabled {
        return Err(RemotePluginCatalogError::UnexpectedEnabledState {
            plugin_id: plugin_id.to_string(),
            expected_enabled: true,
            actual_enabled: response.enabled,
        });
    }

    Ok(())
}

pub async fn uninstall_remote_plugin(
    config: &RemotePluginServiceConfig,
    auth: Option<&RemotePluginAuth>,
    codex_home: PathBuf,
    plugin_id: &str,
) -> Result<(), RemotePluginCatalogError> {
    let auth = ensure_chatgpt_auth(auth)?;
    let plugin = fetch_plugin_detail(
        config, auth, plugin_id, /*include_download_urls*/ false,
    )
    .await?;
    let marketplace_name = remote_plugin_canonical_marketplace_name(&plugin)?.to_string();
    let plugin_name = plugin.name;

    let base_url = config.chatgpt_base_url.trim_end_matches('/');
    let url = format!("{base_url}/plugins/{plugin_id}/uninstall");
    let client = build_reqwest_client();
    let request = authenticated_request(client.post(&url), auth)?;
    let response: RemotePluginMutationResponse = send_and_decode(request, &url).await?;
    if response.id != plugin_id {
        return Err(RemotePluginCatalogError::UnexpectedPluginId {
            expected: plugin_id.to_string(),
            actual: response.id,
        });
    }
    if response.enabled {
        return Err(RemotePluginCatalogError::UnexpectedEnabledState {
            plugin_id: plugin_id.to_string(),
            expected_enabled: false,
            actual_enabled: response.enabled,
        });
    }

    let legacy_plugin_id = plugin_id.to_string();
    tokio::task::spawn_blocking(move || {
        remove_remote_plugin_cache(codex_home, marketplace_name, plugin_name, legacy_plugin_id)
    })
    .await
    .map_err(|err| {
        RemotePluginCatalogError::CacheRemove(format!(
            "failed to join remote plugin cache removal task: {err}"
        ))
    })?
    .map_err(RemotePluginCatalogError::CacheRemove)?;

    Ok(())
}

fn remove_remote_plugin_cache(
    codex_home: PathBuf,
    marketplace_name: String,
    plugin_name: String,
    legacy_plugin_id: String,
) -> Result<(), String> {
    let store = PluginStore::try_new(codex_home.clone())
        .map_err(|err| format!("failed to resolve remote plugin cache root: {err}"))?;
    let plugin_id =
        PluginId::new(plugin_name.clone(), marketplace_name.clone()).map_err(|err| {
            format!(
                "invalid remote plugin cache id for `{plugin_name}` in `{marketplace_name}`: {err}"
            )
        })?;
    let plugin_cache_root = store.plugin_base_root(&plugin_id);
    store.uninstall(&plugin_id).map_err(|err| {
        format!(
            "failed to remove remote plugin cache entry {}: {err}",
            plugin_cache_root.display()
        )
    })?;

    let legacy_remote_plugin_cache_root = codex_home
        .join(PLUGINS_CACHE_DIR)
        .join(marketplace_name)
        .join(legacy_plugin_id);
    if legacy_remote_plugin_cache_root != plugin_cache_root.as_path()
        && legacy_remote_plugin_cache_root.exists()
    {
        let result = if legacy_remote_plugin_cache_root.is_dir() {
            fs::remove_dir_all(&legacy_remote_plugin_cache_root)
        } else {
            fs::remove_file(&legacy_remote_plugin_cache_root)
        };
        result.map_err(|err| {
            format!(
                "failed to remove remote plugin cache entry {}: {err}",
                legacy_remote_plugin_cache_root.display()
            )
        })?;
    }
    Ok(())
}

fn build_remote_plugin_summary(
    plugin: &RemotePluginDirectoryItem,
    installed_plugin: Option<&RemotePluginInstalledItem>,
) -> Result<RemotePluginSummary, RemotePluginCatalogError> {
    let marketplace_name = remote_plugin_canonical_marketplace_name(plugin)?;
    let plugin_id =
        PluginId::new(plugin.name.clone(), marketplace_name.to_string()).map_err(|err| {
            RemotePluginCatalogError::UnexpectedResponse(format!(
                "invalid remote plugin config id for `{}` in `{marketplace_name}`: {err}",
                plugin.name
            ))
        })?;
    Ok(RemotePluginSummary {
        id: plugin_id.as_key(),
        remote_plugin_id: plugin.id.clone(),
        name: plugin.name.clone(),
        share_context: remote_plugin_share_context(plugin)?,
        installed: installed_plugin.is_some(),
        enabled: installed_plugin.is_some_and(|plugin| plugin.enabled),
        install_policy: plugin.installation_policy,
        auth_policy: plugin.authentication_policy,
        availability: plugin.availability,
        interface: remote_plugin_interface_to_info(plugin),
        keywords: plugin.release.keywords.clone(),
    })
}

fn remote_plugin_share_context(
    plugin: &RemotePluginDirectoryItem,
) -> Result<Option<RemotePluginShareContext>, RemotePluginCatalogError> {
    match plugin.scope {
        RemotePluginScope::Global => Ok(None),
        RemotePluginScope::Workspace => {
            let discoverability = workspace_plugin_discoverability(plugin)?;
            Ok(Some(RemotePluginShareContext {
                remote_plugin_id: plugin.id.clone(),
                remote_version: plugin.release.version.clone(),
                discoverability,
                share_url: plugin.share_url.clone(),
                creator_account_user_id: plugin.creator_account_user_id.clone(),
                creator_name: plugin.creator_name.clone(),
                share_principals: plugin.share_principals.as_ref().map(|share_principals| {
                    share_principals
                        .iter()
                        .map(|principal| RemotePluginSharePrincipal {
                            principal_type: principal.principal_type,
                            principal_id: principal.principal_id.clone(),
                            role: principal.role,
                            name: principal.name.clone(),
                        })
                        .collect()
                }),
            }))
        }
    }
}

fn remote_installed_plugin_to_info(
    installed_plugin: &RemotePluginInstalledItem,
) -> Result<RemoteInstalledPlugin, RemotePluginCatalogError> {
    let plugin = &installed_plugin.plugin;
    // Remote per-skill disabled state (`disabled_skill_names`) is intentionally
    // not projected into skills/list yet; local skills.config remains the
    // supported source for skill enablement.
    Ok(RemoteInstalledPlugin {
        marketplace_name: remote_plugin_canonical_marketplace_name(plugin)?.to_string(),
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        enabled: installed_plugin.enabled,
    })
}

fn remote_plugin_interface_to_info(plugin: &RemotePluginDirectoryItem) -> Option<PluginInterface> {
    let interface = &plugin.release.interface;
    let display_name = non_empty_string(Some(&plugin.release.display_name));
    let default_prompt = interface
        .default_prompt
        .as_ref()
        .and_then(|prompt| normalize_remote_default_prompt(prompt));
    let result = PluginInterface {
        display_name,
        short_description: interface.short_description.clone(),
        long_description: interface.long_description.clone(),
        developer_name: interface.developer_name.clone(),
        category: interface.category.clone(),
        capabilities: interface.capabilities.clone(),
        website_url: interface.website_url.clone(),
        privacy_policy_url: interface.privacy_policy_url.clone(),
        terms_of_service_url: interface.terms_of_service_url.clone(),
        default_prompt,
        brand_color: interface.brand_color.clone(),
        composer_icon: None,
        composer_icon_url: interface.composer_icon_url.clone(),
        logo: None,
        logo_url: interface.logo_url.clone(),
        screenshots: Vec::new(),
        screenshot_urls: interface.screenshot_urls.clone(),
    };
    let has_fields = result.display_name.is_some()
        || result.short_description.is_some()
        || result.long_description.is_some()
        || result.developer_name.is_some()
        || result.category.is_some()
        || !result.capabilities.is_empty()
        || result.website_url.is_some()
        || result.privacy_policy_url.is_some()
        || result.terms_of_service_url.is_some()
        || result.default_prompt.is_some()
        || result.brand_color.is_some()
        || result.composer_icon_url.is_some()
        || result.logo_url.is_some()
        || !result.screenshot_urls.is_empty();
    has_fields.then_some(result)
}

fn remote_skill_interface_to_info(
    interface: Option<RemotePluginSkillInterfaceResponse>,
) -> Option<SkillInterface> {
    interface.and_then(|interface| {
        let result = SkillInterface {
            display_name: interface.display_name,
            short_description: interface.short_description,
            icon_small: None,
            icon_large: None,
            brand_color: interface.brand_color,
            default_prompt: interface.default_prompt,
        };
        let has_fields = result.display_name.is_some()
            || result.short_description.is_some()
            || result.brand_color.is_some()
            || result.default_prompt.is_some();
        has_fields.then_some(result)
    })
}

fn remote_plugin_display_name(plugin: &RemotePluginSummary) -> &str {
    plugin
        .interface
        .as_ref()
        .and_then(|interface| interface.display_name.as_deref())
        .unwrap_or(&plugin.name)
}

fn non_empty_string(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn normalize_remote_default_prompt(prompt: &str) -> Option<Vec<String>> {
    let prompt = prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > MAX_REMOTE_DEFAULT_PROMPT_LEN {
        return None;
    }
    Some(vec![prompt.to_string()])
}

async fn fetch_directory_plugins_for_scope(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
    scope: RemotePluginScope,
) -> Result<Vec<RemotePluginDirectoryItem>, RemotePluginCatalogError> {
    let mut plugins = Vec::new();
    let mut page_token = None;
    loop {
        let response =
            get_remote_plugin_list_page(config, auth, scope, page_token.as_deref()).await?;
        plugins.extend(response.plugins);
        let Some(next_page_token) = response.pagination.next_page_token else {
            break;
        };
        page_token = Some(next_page_token);
    }
    Ok(plugins)
}

async fn fetch_shared_workspace_plugins(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
) -> Result<Vec<RemotePluginDirectoryItem>, RemotePluginCatalogError> {
    let mut plugins = Vec::new();
    let mut page_token = None;
    loop {
        let response =
            get_remote_shared_workspace_plugins_page(config, auth, page_token.as_deref()).await?;
        plugins.extend(response.plugins);
        let Some(next_page_token) = response.pagination.next_page_token else {
            break;
        };
        page_token = Some(next_page_token);
    }
    Ok(plugins)
}

async fn fetch_installed_plugins_for_scope(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
    scope: RemotePluginScope,
) -> Result<Vec<RemotePluginInstalledItem>, RemotePluginCatalogError> {
    fetch_installed_plugins_for_scope_with_download_url(
        config, auth, scope, /*include_download_urls*/ false,
    )
    .await
}

async fn fetch_installed_plugins_for_scope_with_download_url(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
    scope: RemotePluginScope,
    include_download_urls: bool,
) -> Result<Vec<RemotePluginInstalledItem>, RemotePluginCatalogError> {
    let mut plugins = Vec::new();
    let mut page_token = None;
    loop {
        let response = get_remote_plugin_installed_page(
            config,
            auth,
            scope,
            page_token.as_deref(),
            include_download_urls,
        )
        .await?;
        plugins.extend(response.plugins);
        let Some(next_page_token) = response.pagination.next_page_token else {
            break;
        };
        page_token = Some(next_page_token);
    }
    Ok(plugins)
}

async fn get_remote_plugin_list_page(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
    scope: RemotePluginScope,
    page_token: Option<&str>,
) -> Result<RemotePluginListResponse, RemotePluginCatalogError> {
    let base_url = config.chatgpt_base_url.trim_end_matches('/');
    let url = format!("{base_url}/ps/plugins/list");
    let client = build_reqwest_client();
    let mut request = authenticated_request(client.get(&url), auth)?;
    request = request.query(&[("scope", scope.api_value())]);
    request = request.query(&[("limit", REMOTE_PLUGIN_LIST_PAGE_LIMIT)]);
    if let Some(page_token) = page_token {
        request = request.query(&[("pageToken", page_token)]);
    }
    send_and_decode(request, &url).await
}

async fn get_remote_shared_workspace_plugins_page(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
    page_token: Option<&str>,
) -> Result<RemotePluginListResponse, RemotePluginCatalogError> {
    let base_url = config.chatgpt_base_url.trim_end_matches('/');
    let url = format!("{base_url}/ps/plugins/workspace/shared");
    let client = build_reqwest_client();
    let mut request = authenticated_request(client.get(&url), auth)?;
    request = request.query(&[("limit", REMOTE_PLUGIN_LIST_PAGE_LIMIT)]);
    if let Some(page_token) = page_token {
        request = request.query(&[("pageToken", page_token)]);
    }
    send_and_decode(request, &url).await
}

async fn get_remote_plugin_installed_page(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
    scope: RemotePluginScope,
    page_token: Option<&str>,
    include_download_urls: bool,
) -> Result<RemotePluginInstalledResponse, RemotePluginCatalogError> {
    let base_url = config.chatgpt_base_url.trim_end_matches('/');
    let url = format!("{base_url}/ps/plugins/installed");
    let client = build_reqwest_client();
    let mut request = authenticated_request(client.get(&url), auth)?;
    request = request.query(&[("scope", scope.api_value())]);
    if include_download_urls {
        request = request.query(&[("includeDownloadUrls", true)]);
    }
    if let Some(page_token) = page_token {
        request = request.query(&[("pageToken", page_token)]);
    }
    send_and_decode(request, &url).await
}

async fn fetch_plugin_detail(
    config: &RemotePluginServiceConfig,
    auth: &RemotePluginAuth,
    plugin_id: &str,
    include_download_urls: bool,
) -> Result<RemotePluginDirectoryItem, RemotePluginCatalogError> {
    let base_url = config.chatgpt_base_url.trim_end_matches('/');
    let url = format!("{base_url}/ps/plugins/{plugin_id}");
    let client = build_reqwest_client();
    let mut request = authenticated_request(client.get(&url), auth)?;
    if include_download_urls {
        request = request.query(&[("includeDownloadUrls", true)]);
    }
    send_and_decode(request, &url).await
}

fn remote_plugin_skill_detail_url(
    config: &RemotePluginServiceConfig,
    plugin_id: &str,
    skill_name: &str,
) -> Result<String, RemotePluginCatalogError> {
    let mut url = Url::parse(config.chatgpt_base_url.trim_end_matches('/'))
        .map_err(RemotePluginCatalogError::InvalidBaseUrl)?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| RemotePluginCatalogError::InvalidBaseUrlPath)?;
        segments.pop_if_empty();
        segments.push("ps");
        segments.push("plugins");
        segments.push(plugin_id);
        segments.push("skills");
        segments.push(skill_name);
    }
    Ok(url.to_string())
}

fn ensure_chatgpt_auth(
    auth: Option<&RemotePluginAuth>,
) -> Result<&RemotePluginAuth, RemotePluginCatalogError> {
    let Some(auth) = auth else {
        return Err(RemotePluginCatalogError::AuthRequired);
    };
    if !auth.uses_codex_backend() {
        return Err(RemotePluginCatalogError::UnsupportedAuthMode);
    }
    Ok(auth)
}

fn authenticated_request(
    request: RequestBuilder,
    auth: &RemotePluginAuth,
) -> Result<RequestBuilder, RemotePluginCatalogError> {
    Ok(request.timeout(REMOTE_PLUGIN_CATALOG_TIMEOUT).headers(
        codex_api_auth::auth_provider_from_auth_snapshot(auth.request_auth_snapshot())
            .to_auth_headers(),
    ))
}

async fn send_and_decode<T: for<'de> Deserialize<'de>>(
    request: RequestBuilder,
    url: &str,
) -> Result<T, RemotePluginCatalogError> {
    let response = request
        .send()
        .await
        .map_err(|source| RemotePluginCatalogError::Request {
            url: url.to_string(),
            source,
        })?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(RemotePluginCatalogError::UnexpectedStatus {
            url: url.to_string(),
            status,
            body,
        });
    }

    serde_json::from_str(&body).map_err(|source| RemotePluginCatalogError::Decode {
        url: url.to_string(),
        source,
    })
}
