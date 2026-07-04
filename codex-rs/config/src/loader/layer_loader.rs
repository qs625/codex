use crate::ConfigLoadOptions;
use crate::ThreadConfigLoader;
use crate::CloudRequirementsLoader;
use codex_config_state::ConfigLayerStack;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use toml::Value as TomlValue;

pub type ConfigLayerLoadFuture<'a> =
    Pin<Box<dyn Future<Output = io::Result<ConfigLayerStack>> + Send + 'a>>;

/// Input for loading the full ordered config layer stack for a thread or
/// thread-agnostic config query.
pub struct ConfigLayerLoadRequest {
    pub codex_home: AbsolutePathBuf,
    pub cwd: Option<AbsolutePathBuf>,
    pub cli_overrides: Vec<(String, TomlValue)>,
    pub options: ConfigLoadOptions,
    pub cloud_requirements: CloudRequirementsLoader,
    pub thread_config_loader: Arc<dyn ThreadConfigLoader>,
}

/// Host-provided loader for resolving raw config sources into a merged layer
/// stack.
///
/// Implementations own filesystem, platform, MDM, git, and other local IO
/// details. Consumers such as session/core config builders should depend on
/// this trait rather than directly calling a concrete local loader.
pub trait ConfigLayerLoader: Send + Sync {
    fn load(&self, request: ConfigLayerLoadRequest) -> ConfigLayerLoadFuture<'_>;
}
