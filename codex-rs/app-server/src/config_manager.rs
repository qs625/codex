use codex_arg0::Arg0DispatchPaths;
use codex_cloud_requirements::cloud_requirements_loader;
use codex_config_types::ConfigLayerSource;
use codex_exec_server::LOCAL_FS;
use codex_features::feature_for_key;
use codex_login::AuthManager;
use codex_login::default_client::set_default_client_residency_requirement;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_json_to_toml::json_to_toml;
use config_service::CloudRequirementsLoader;
use config_service::ConfigLayerStack;
use config_service::ConfigLayerStackOrdering;
use config_service::ConfigLoadOptions;
use config_service::LoaderOverrides;
use config_service::NoopThreadConfigLoader;
use config_service::ThreadConfigLoader;
use config_service::config_toml::ConfigToml;
use config_service::load_config_layers_state;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use thread_service::config::Config;
use thread_service::config::ConfigOverrides;
use thread_service_api::LiveThreadConfigRefreshSnapshot;
use toml::Value as TomlValue;
use tracing::warn;

/// Shared app-server entry point for loading effective Codex configuration.
#[derive(Clone)]
pub(crate) struct ConfigManager {
    codex_home: PathBuf,
    cli_overrides: Arc<RwLock<Vec<(String, TomlValue)>>>,
    runtime_feature_enablement: Arc<RwLock<BTreeMap<String, bool>>>,
    loader_overrides: LoaderOverrides,
    strict_config: bool,
    cloud_requirements: Arc<RwLock<CloudRequirementsLoader>>,
    arg0_paths: Arg0DispatchPaths,
    thread_config_loader: Arc<RwLock<Arc<dyn ThreadConfigLoader>>>,
}

impl ConfigManager {
    pub(crate) fn new(
        codex_home: PathBuf,
        cli_overrides: Vec<(String, TomlValue)>,
        loader_overrides: LoaderOverrides,
        strict_config: bool,
        cloud_requirements: CloudRequirementsLoader,
        arg0_paths: Arg0DispatchPaths,
        thread_config_loader: Arc<dyn ThreadConfigLoader>,
    ) -> Self {
        Self {
            codex_home,
            cli_overrides: Arc::new(RwLock::new(cli_overrides)),
            runtime_feature_enablement: Arc::new(RwLock::new(BTreeMap::new())),
            loader_overrides,
            strict_config,
            cloud_requirements: Arc::new(RwLock::new(cloud_requirements)),
            arg0_paths,
            thread_config_loader: Arc::new(RwLock::new(thread_config_loader)),
        }
    }

    pub(crate) fn codex_home(&self) -> &Path {
        self.codex_home.as_path()
    }

    pub(crate) fn user_config_path(&self) -> std::io::Result<AbsolutePathBuf> {
        self.loader_overrides.user_config_path(self.codex_home())
    }

    pub(crate) fn current_cli_overrides(&self) -> Vec<(String, TomlValue)> {
        self.cli_overrides
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn current_cloud_requirements(&self) -> CloudRequirementsLoader {
        self.cloud_requirements
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn extend_runtime_feature_enablement<I>(&self, enablement: I) -> Result<(), ()>
    where
        I: IntoIterator<Item = (String, bool)>,
    {
        let mut runtime_feature_enablement =
            self.runtime_feature_enablement.write().map_err(|_| ())?;
        runtime_feature_enablement.extend(enablement);
        Ok(())
    }

    pub(crate) fn replace_cloud_requirements_loader(
        &self,
        auth_manager: Arc<AuthManager>,
        chatgpt_base_url: String,
    ) {
        let loader =
            cloud_requirements_loader(auth_manager, chatgpt_base_url, self.codex_home.clone());
        if let Ok(mut guard) = self.cloud_requirements.write() {
            *guard = loader;
        } else {
            warn!("failed to update cloud requirements loader");
        }
    }

    pub(crate) fn replace_thread_config_loader(
        &self,
        thread_config_loader: Arc<dyn ThreadConfigLoader>,
    ) {
        if let Ok(mut guard) = self.thread_config_loader.write() {
            *guard = thread_config_loader;
        } else {
            warn!("failed to update thread config loader");
        }
    }

    fn current_thread_config_loader(&self) -> Arc<dyn ThreadConfigLoader> {
        self.thread_config_loader
            .read()
            .map(|guard| Arc::clone(&*guard))
            .unwrap_or_else(|_| Arc::new(NoopThreadConfigLoader))
    }

    pub(crate) async fn sync_default_client_residency_requirement(&self) {
        match self.load_latest_config(/*fallback_cwd*/ None).await {
            Ok(config) => {
                set_default_client_residency_requirement(config.enforce_residency.value());
            }
            Err(err) => warn!(
                error = %err,
                "failed to sync default client residency requirement after auth refresh"
            ),
        }
    }

    pub(crate) async fn load_latest_config(
        &self,
        fallback_cwd: Option<PathBuf>,
    ) -> std::io::Result<Config> {
        self.load_with_cli_overrides(
            &self.current_cli_overrides(),
            /*request_overrides*/ None,
            ConfigOverrides::default(),
            fallback_cwd,
        )
        .await
    }

    pub(crate) async fn load_latest_config_for_thread_refresh_snapshot(
        &self,
        snapshot: &LiveThreadConfigRefreshSnapshot,
    ) -> std::io::Result<Config> {
        let refreshed_config = self
            .load_latest_config(Some(snapshot.cwd.to_path_buf()))
            .await?;
        let mut layers = refreshed_config
            .config_layer_stack
            .get_layers(
                ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ true,
            )
            .into_iter()
            .filter(|layer| !matches!(layer.name, ConfigLayerSource::SessionFlags))
            .cloned()
            .collect::<Vec<_>>();
        layers.extend(snapshot.session_layers.clone());
        layers.sort_by_key(|layer| layer.name.precedence());

        let config_layer_stack = ConfigLayerStack::new(
            layers,
            refreshed_config.config_layer_stack.requirements().clone(),
            refreshed_config
                .config_layer_stack
                .requirements_toml()
                .clone(),
        )
        .map_err(std::io::Error::other)?
        .with_user_and_project_exec_policy_rules_ignored(
            refreshed_config
                .config_layer_stack
                .ignore_user_and_project_exec_policy_rules(),
        );
        let cfg: ConfigToml = config_layer_stack
            .effective_config()
            .try_into()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        let mut config = Config::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            cfg,
            ConfigOverrides {
                cwd: Some(snapshot.cwd.to_path_buf()),
                ..Default::default()
            },
            refreshed_config.codex_home.clone(),
            config_layer_stack,
        )
        .await?;
        self.apply_runtime_feature_enablement(&mut config);
        self.apply_arg0_paths(&mut config);
        Ok(config)
    }

    pub(crate) async fn load_default_config(&self) -> std::io::Result<Config> {
        let mut config = Config::load_default_with_cli_overrides_for_codex_home(
            self.codex_home.clone(),
            self.current_cli_overrides(),
        )
        .await?;
        if self.loader_overrides.user_config_path.is_some()
            || self.loader_overrides.user_config_profile.is_some()
        {
            let user_config_path = self.loader_overrides.user_config_path(self.codex_home())?;
            config.config_layer_stack = config.config_layer_stack.with_user_config_profile(
                &user_config_path,
                self.loader_overrides.user_config_profile.as_deref(),
                TomlValue::Table(toml::map::Map::new()),
            );
        }
        self.apply_runtime_feature_enablement(&mut config);
        self.apply_arg0_paths(&mut config);
        Ok(config)
    }

    pub(crate) async fn load_with_overrides(
        &self,
        request_overrides: Option<HashMap<String, serde_json::Value>>,
        typesafe_overrides: ConfigOverrides,
    ) -> std::io::Result<Config> {
        self.load_with_cli_overrides(
            &self.current_cli_overrides(),
            request_overrides,
            typesafe_overrides,
            /*fallback_cwd*/ None,
        )
        .await
    }

    pub(crate) async fn load_for_cwd(
        &self,
        request_overrides: Option<HashMap<String, serde_json::Value>>,
        typesafe_overrides: ConfigOverrides,
        cwd: Option<PathBuf>,
    ) -> std::io::Result<Config> {
        self.load_with_cli_overrides(
            &self.current_cli_overrides(),
            request_overrides,
            typesafe_overrides,
            cwd,
        )
        .await
    }

    pub(crate) async fn load_with_cli_overrides(
        &self,
        cli_overrides: &[(String, TomlValue)],
        request_overrides: Option<HashMap<String, serde_json::Value>>,
        typesafe_overrides: ConfigOverrides,
        fallback_cwd: Option<PathBuf>,
    ) -> std::io::Result<Config> {
        let merged_cli_overrides = cli_overrides
            .iter()
            .cloned()
            .chain(
                request_overrides
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, value)| (key, json_to_toml(value))),
            )
            .collect::<Vec<_>>();

        let mut config = crate::config_builder()
            .codex_home(self.codex_home.clone())
            .cli_overrides(merged_cli_overrides)
            .loader_overrides(self.loader_overrides.clone())
            .strict_config(self.strict_config)
            .harness_overrides(typesafe_overrides)
            .fallback_cwd(fallback_cwd)
            .cloud_requirements(self.current_cloud_requirements())
            .thread_config_loader(self.current_thread_config_loader())
            .build()
            .await?;
        self.apply_runtime_feature_enablement(&mut config);
        self.apply_arg0_paths(&mut config);
        Ok(config)
    }

    pub(crate) async fn load_config_layers_for_cwd(
        &self,
        cwd: AbsolutePathBuf,
    ) -> std::io::Result<ConfigLayerStack> {
        self.load_config_layers(Some(cwd)).await
    }

    pub(crate) async fn load_config_layers(
        &self,
        cwd: Option<AbsolutePathBuf>,
    ) -> std::io::Result<ConfigLayerStack> {
        let thread_config_loader = self.current_thread_config_loader();
        load_config_layers_state(
            LOCAL_FS.as_ref(),
            &self.codex_home,
            cwd,
            &self.current_cli_overrides(),
            ConfigLoadOptions {
                loader_overrides: self.loader_overrides.clone(),
                strict_config: self.strict_config,
            },
            self.current_cloud_requirements(),
            thread_config_loader.as_ref(),
        )
        .await
    }

    fn apply_runtime_feature_enablement(&self, config: &mut Config) {
        apply_runtime_feature_enablement(config, &self.current_runtime_feature_enablement());
    }

    fn current_runtime_feature_enablement(&self) -> BTreeMap<String, bool> {
        self.runtime_feature_enablement
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn apply_arg0_paths(&self, config: &mut Config) {
        config.codex_self_exe = self.arg0_paths.codex_self_exe.clone();
        config.codex_linux_sandbox_exe = self.arg0_paths.codex_linux_sandbox_exe.clone();
        config.main_execve_wrapper_exe = self.arg0_paths.main_execve_wrapper_exe.clone();
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests(
        codex_home: PathBuf,
        cli_overrides: Vec<(String, TomlValue)>,
        loader_overrides: LoaderOverrides,
        cloud_requirements: CloudRequirementsLoader,
    ) -> Self {
        Self::new(
            codex_home,
            cli_overrides,
            loader_overrides,
            /*strict_config*/ false,
            cloud_requirements,
            Arg0DispatchPaths::default(),
            Arc::new(NoopThreadConfigLoader),
        )
    }

    #[cfg(test)]
    pub(crate) fn without_managed_config_for_tests(codex_home: PathBuf) -> Self {
        Self::new_for_tests(
            codex_home,
            Vec::new(),
            LoaderOverrides::without_managed_config_for_tests(),
            CloudRequirementsLoader::default(),
        )
    }
}

pub(crate) fn protected_feature_keys(config_layer_stack: &ConfigLayerStack) -> BTreeSet<String> {
    let mut protected_features = config_layer_stack
        .effective_config()
        .get("features")
        .and_then(toml::Value::as_table)
        .map(|features| features.keys().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();

    if let Some(feature_requirements) = config_layer_stack
        .requirements_toml()
        .feature_requirements
        .as_ref()
    {
        protected_features.extend(feature_requirements.entries.keys().cloned());
    }

    protected_features
}

pub(crate) fn apply_runtime_feature_enablement(
    config: &mut Config,
    runtime_feature_enablement: &BTreeMap<String, bool>,
) {
    let protected_features = protected_feature_keys(&config.config_layer_stack);
    for (name, enabled) in runtime_feature_enablement {
        if protected_features.contains(name) {
            continue;
        }
        let Some(feature) = feature_for_key(name) else {
            continue;
        };
        if let Err(err) = config.features.set_enabled(feature, *enabled) {
            warn!(
                feature = name,
                error = %err,
                "failed to apply runtime feature enablement"
            );
        }
    }
}
