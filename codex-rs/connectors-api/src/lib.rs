use std::sync::LazyLock;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use std::time::Instant;

use serde::Deserialize;
use serde::Serialize;

pub mod accessible;
mod app_types;
pub mod directory_cache;
pub mod filter;
pub mod merge;
pub mod metadata;

pub use app_types::AppBranding;
pub use app_types::AppInfo;
pub use app_types::AppMetadata;
pub use app_types::AppReview;
pub use app_types::AppScreenshot;
pub use app_types::AppSummary;
pub use directory_cache::ConnectorDirectoryCacheContext;

pub const CONNECTORS_CACHE_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorDirectoryCacheKey {
    chatgpt_base_url: String,
    account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    is_workspace_account: bool,
}

impl ConnectorDirectoryCacheKey {
    pub fn new(
        chatgpt_base_url: String,
        account_id: Option<String>,
        chatgpt_user_id: Option<String>,
        is_workspace_account: bool,
    ) -> Self {
        Self {
            chatgpt_base_url,
            account_id,
            chatgpt_user_id,
            is_workspace_account,
        }
    }
}

#[derive(Clone)]
struct CachedConnectorDirectory {
    key: ConnectorDirectoryCacheKey,
    expires_at: Instant,
    connectors: Vec<AppInfo>,
}

static CONNECTOR_DIRECTORY_CACHE: LazyLock<StdMutex<Option<CachedConnectorDirectory>>> =
    LazyLock::new(|| StdMutex::new(None));

pub fn cached_directory_connectors(
    cache_context: &ConnectorDirectoryCacheContext,
) -> Option<Vec<AppInfo>> {
    if let Some(cached_connectors) = cached_directory_connectors_in_memory(&cache_context.cache_key)
    {
        return Some(cached_connectors);
    }

    let directory_cache::CachedConnectorDirectoryDiskLoad::Hit { connectors } =
        directory_cache::load_cached_directory_connectors_from_disk(cache_context)
    else {
        return None;
    };
    write_cached_directory_connectors_in_memory(
        cache_context.cache_key.clone(),
        &connectors,
        Duration::ZERO,
    );
    Some(connectors)
}

fn cached_directory_connectors_in_memory(
    cache_key: &ConnectorDirectoryCacheKey,
) -> Option<Vec<AppInfo>> {
    let cache_guard = CONNECTOR_DIRECTORY_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache_guard
        .as_ref()
        .filter(|cached| cached.key == *cache_key)
        .map(|cached| cached.connectors.clone())
}

pub fn unexpired_directory_connectors(
    cache_context: &ConnectorDirectoryCacheContext,
) -> Option<Vec<AppInfo>> {
    unexpired_directory_connectors_in_memory(&cache_context.cache_key)
}

fn unexpired_directory_connectors_in_memory(
    cache_key: &ConnectorDirectoryCacheKey,
) -> Option<Vec<AppInfo>> {
    let cache_guard = CONNECTOR_DIRECTORY_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cached = cache_guard.as_ref()?;
    if cached.key == *cache_key && Instant::now() < cached.expires_at {
        return Some(cached.connectors.clone());
    }
    None
}

pub fn write_cached_directory_connectors(
    cache_context: &ConnectorDirectoryCacheContext,
    connectors: &[AppInfo],
) {
    write_cached_directory_connectors_in_memory(
        cache_context.cache_key.clone(),
        connectors,
        CONNECTORS_CACHE_TTL,
    );
    directory_cache::write_cached_directory_connectors_to_disk(cache_context, connectors);
}

fn write_cached_directory_connectors_in_memory(
    cache_key: ConnectorDirectoryCacheKey,
    connectors: &[AppInfo],
    ttl: Duration,
) {
    let mut cache_guard = CONNECTOR_DIRECTORY_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *cache_guard = Some(CachedConnectorDirectory {
        key: cache_key,
        expires_at: Instant::now() + ttl,
        connectors: connectors.to_vec(),
    });
}

#[doc(hidden)]
pub fn clear_directory_memory_cache_for_tests() {
    let mut cache_guard = CONNECTOR_DIRECTORY_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *cache_guard = None;
}

pub fn connector_install_url(name: &str, connector_id: &str) -> String {
    let slug = connector_name_slug(name);
    format!("https://chatgpt.com/apps/{slug}/{connector_id}")
}

fn connector_name_slug(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push('-');
        }
    }
    let normalized = normalized.trim_matches('-');
    if normalized.is_empty() {
        "app".to_string()
    } else {
        normalized.to_string()
    }
}

pub fn normalize_connector_name(name: &str, connector_id: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        connector_id.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn normalize_connector_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
