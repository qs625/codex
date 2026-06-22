use super::*;

#[derive(Clone, Default)]
pub struct ConfigBuilder {
    codex_home: Option<PathBuf>,
    cli_overrides: Option<Vec<(String, TomlValue)>>,
    harness_overrides: Option<ConfigOverrides>,
    loader_overrides: Option<LoaderOverrides>,
    strict_config: bool,
    cloud_requirements: CloudRequirementsLoader,
    config_layer_loader: Option<Arc<dyn ConfigLayerLoader>>,
    thread_config_loader: Option<Arc<dyn ThreadConfigLoader>>,
    fallback_cwd: Option<PathBuf>,
}

fn missing_config_layer_loader_error() -> std::io::Error {
    std::io::Error::other(
        "ConfigBuilder requires a ConfigLayerLoader; composition roots must inject one",
    )
}

#[cfg(any(test, feature = "test-support"))]
fn default_config_layer_loader_for_tests() -> Option<Arc<dyn ConfigLayerLoader>> {
    Some(Arc::new(
        codex_config_local_loader::LocalConfigLayerLoader::default(),
    ))
}

#[cfg(not(any(test, feature = "test-support")))]
fn default_config_layer_loader_for_tests() -> Option<Arc<dyn ConfigLayerLoader>> {
    None
}

impl ConfigBuilder {
    pub fn codex_home(mut self, codex_home: PathBuf) -> Self {
        self.codex_home = Some(codex_home);
        self
    }

    pub fn cli_overrides(mut self, cli_overrides: Vec<(String, TomlValue)>) -> Self {
        self.cli_overrides = Some(cli_overrides);
        self
    }

    pub fn harness_overrides(mut self, harness_overrides: ConfigOverrides) -> Self {
        self.harness_overrides = Some(harness_overrides);
        self
    }

    pub fn loader_overrides(mut self, loader_overrides: LoaderOverrides) -> Self {
        self.loader_overrides = Some(loader_overrides);
        self
    }

    pub fn strict_config(mut self, strict_config: bool) -> Self {
        self.strict_config = strict_config;
        self
    }

    pub fn cloud_requirements(mut self, cloud_requirements: CloudRequirementsLoader) -> Self {
        self.cloud_requirements = cloud_requirements;
        self
    }

    pub fn config_layer_loader(mut self, config_layer_loader: Arc<dyn ConfigLayerLoader>) -> Self {
        self.config_layer_loader = Some(config_layer_loader);
        self
    }

    pub fn thread_config_loader(
        mut self,
        thread_config_loader: Arc<dyn ThreadConfigLoader>,
    ) -> Self {
        self.thread_config_loader = Some(thread_config_loader);
        self
    }

    pub fn fallback_cwd(mut self, fallback_cwd: Option<PathBuf>) -> Self {
        self.fallback_cwd = fallback_cwd;
        self
    }

    pub async fn build(self) -> std::io::Result<Config> {
        // Keep the large config-loading future off small runtime thread stacks.
        Box::pin(self.build_inner()).await
    }

    async fn build_inner(self) -> std::io::Result<Config> {
        let Self {
            codex_home,
            cli_overrides,
            harness_overrides,
            loader_overrides,
            strict_config,
            cloud_requirements,
            config_layer_loader,
            thread_config_loader,
            fallback_cwd,
        } = self;
        let codex_home = match codex_home {
            Some(codex_home) => AbsolutePathBuf::from_absolute_path(codex_home)?,
            None => find_codex_home()?,
        };
        let cli_overrides = cli_overrides.unwrap_or_default();
        let mut harness_overrides = harness_overrides.unwrap_or_default();
        let loader_overrides = loader_overrides.unwrap_or_default();
        let cwd_override = harness_overrides.cwd.as_deref().or(fallback_cwd.as_deref());
        let cwd = match cwd_override {
            Some(path) => AbsolutePathBuf::relative_to_current_dir(path)?,
            None => AbsolutePathBuf::current_dir()?,
        };
        harness_overrides.cwd = Some(cwd.to_path_buf());
        let Some(config_layer_loader) =
            config_layer_loader.or_else(default_config_layer_loader_for_tests)
        else {
            return Err(missing_config_layer_loader_error());
        };
        let thread_config_loader: Arc<dyn ThreadConfigLoader> =
            thread_config_loader.unwrap_or_else(|| Arc::new(NoopThreadConfigLoader));
        let config_layer_stack = config_layer_loader
            .load(ConfigLayerLoadRequest {
                codex_home: codex_home.clone(),
                cwd: Some(cwd),
                cli_overrides,
                options: ConfigLoadOptions {
                    loader_overrides,
                    strict_config,
                },
                cloud_requirements,
                thread_config_loader,
            })
            .await?;
        let merged_toml = config_layer_stack.effective_config();

        // Note that each layer in ConfigLayerStack should have resolved
        // relative paths to absolute paths based on the parent folder of the
        // respective config file, so we should be safe to deserialize without
        // AbsolutePathBufGuard here.
        let config_toml: ConfigToml = match merged_toml.try_into() {
            Ok(config_toml) => config_toml,
            Err(err) => {
                if let Some(config_error) =
                    first_layer_config_error::<ConfigToml>(&config_layer_stack, CONFIG_TOML_FILE)
                        .await
                {
                    return Err(io_error_from_config_error(
                        std::io::ErrorKind::InvalidData,
                        config_error,
                        Some(err),
                    ));
                }
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, err));
            }
        };
        let config_lock_settings = config_toml
            .debug
            .as_ref()
            .and_then(|debug| debug.config_lockfile.as_ref());
        if let Some(config_lock_load_path) =
            config_lock_settings.and_then(|config_lock| config_lock.load_path.as_ref())
        {
            let allow_codex_version_mismatch = config_lock_settings
                .and_then(|config_lock| config_lock.allow_codex_version_mismatch)
                .unwrap_or(false);
            let save_fields_resolved_from_model_catalog = config_lock_settings
                .and_then(|config_lock| config_lock.save_fields_resolved_from_model_catalog)
                .unwrap_or(true);
            let lockfile_toml = read_config_lock_from_path(config_lock_load_path).await?;
            let expected_lock_config = lockfile_toml.clone();
            let lock_layer = lock_layer_from_config(config_lock_load_path, &lockfile_toml)?;
            let lock_config_toml = config_without_lock_controls(&lockfile_toml.config);
            let lock_config_layer_stack = ConfigLayerStack::new(
                vec![lock_layer],
                config_layer_stack.requirements().clone(),
                config_layer_stack.requirements_toml().clone(),
            )?;
            let mut config = Config::load_config_with_layer_stack(
                LOCAL_FS.as_ref(),
                lock_config_toml,
                harness_overrides,
                codex_home,
                lock_config_layer_stack,
            )
            .await?;
            config.config_lock_toml = Some(Arc::new(expected_lock_config));
            config.config_lock_allow_codex_version_mismatch = allow_codex_version_mismatch;
            config.config_lock_save_fields_resolved_from_model_catalog =
                save_fields_resolved_from_model_catalog;
            return Ok(config);
        }
        Config::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            config_toml,
            harness_overrides,
            codex_home,
            config_layer_stack,
        )
        .await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn without_managed_config_for_tests() -> Self {
        Self::default().loader_overrides(LoaderOverrides::without_managed_config_for_tests())
    }
}

impl Config {
    pub fn legacy_sandbox_policy(&self) -> SandboxPolicy {
        self.permissions.legacy_sandbox_policy(self.cwd.as_path())
    }

    pub fn set_legacy_sandbox_policy(
        &mut self,
        sandbox_policy: SandboxPolicy,
    ) -> ConstraintResult<()> {
        self.workspace_roots_explicit = matches!(
            &sandbox_policy,
            SandboxPolicy::WorkspaceWrite { writable_roots, .. } if !writable_roots.is_empty()
        );
        self.permissions
            .set_legacy_sandbox_policy(sandbox_policy, self.cwd.as_path())?;
        self.workspace_roots = self.permissions.workspace_roots().to_vec();
        Ok(())
    }

    pub fn effective_workspace_roots(&self) -> Vec<AbsolutePathBuf> {
        let mut workspace_roots = self.workspace_roots.clone();
        workspace_roots.extend(self.permissions.profile_workspace_roots().iter().cloned());
        dedupe_absolute_paths(&mut workspace_roots);
        workspace_roots
    }

    pub fn to_models_manager_config(&self) -> ModelsManagerConfig {
        ModelsManagerConfig {
            model_context_window: self.model_context_window,
            model_auto_compact_token_limit: self.model_auto_compact_token_limit,
            tool_output_token_limit: self.tool_output_token_limit,
            base_instructions: self.base_instructions.clone(),
            personality_enabled: self.features.enabled(Feature::Personality),
            model_supports_reasoning_summaries: self.model_supports_reasoning_summaries,
            model_catalog: self.model_catalog.clone(),
            model_metadata_overrides: self
                .model_options
                .iter()
                .map(|model_option| ModelMetadataOverride {
                    model: model_option.model.clone(),
                    context_window: model_option.context_window,
                    max_context_window: model_option.max_context_window,
                    auto_compact_token_limit: model_option.auto_compact_token_limit,
                })
                .collect(),
        }
    }

    /// Build the plugin-manager input from the effective config.
    pub fn plugins_config_input(&self) -> PluginsConfigInput {
        PluginsConfigInput::new(
            plugin_config_layer_stack_from_config_layer_stack(&self.config_layer_stack),
            self.features.enabled(Feature::Plugins),
            self.features.enabled(Feature::RemotePlugin),
            self.features.enabled(Feature::PluginHooks),
            self.chatgpt_base_url.clone(),
        )
    }

    pub async fn to_mcp_config(&self, plugins_manager: &dyn PluginRuntime) -> McpConfig {
        let plugins_input = self.plugins_config_input();
        let loaded_plugins = plugins_manager.plugins_for_config(&plugins_input).await;
        let mut configured_mcp_servers = self.mcp_servers.get().clone();
        for plugin in loaded_plugins
            .plugins()
            .iter()
            .filter(|plugin| plugin.is_active())
        {
            let mut plugin_mcp_servers = plugin.mcp_servers.clone();
            filter_plugin_mcp_servers_by_requirements(
                &plugin.config_name,
                &mut plugin_mcp_servers,
                self.config_layer_stack.requirements().plugins.as_ref(),
            );
            for (name, plugin_server) in plugin_mcp_servers {
                configured_mcp_servers.entry(name).or_insert(plugin_server);
            }
        }
        if let Some(mcp_requirements) = self.config_layer_stack.requirements().mcp_servers.as_ref()
            && mcp_requirements.value.is_empty()
        {
            // A present empty allowlist bans configurable MCPs, including plugin MCPs merged
            // above.
            filter_mcp_servers_by_requirements(&mut configured_mcp_servers, Some(mcp_requirements));
        }

        McpConfig {
            chatgpt_base_url: self.chatgpt_base_url.clone(),
            apps_mcp_path_override: self.apps_mcp_path_override.clone(),
            codex_home: self.codex_home.to_path_buf(),
            mcp_oauth_credentials_store_mode: self.mcp_oauth_credentials_store_mode,
            mcp_oauth_callback_port: self.mcp_oauth_callback_port,
            mcp_oauth_callback_url: self.mcp_oauth_callback_url.clone(),
            skill_mcp_dependency_install_enabled: self
                .features
                .enabled(Feature::SkillMcpDependencyInstall),
            approval_policy: self.permissions.approval_policy.clone(),
            codex_linux_sandbox_exe: self.codex_linux_sandbox_exe.clone(),
            use_legacy_landlock: self.features.use_legacy_landlock(),
            apps_enabled: self.features.enabled(Feature::Apps),
            client_elicitation_support:
                codex_mcp_types::McpClientElicitationSupport::from_auth_elicitation_enabled(
                    self.features.enabled(Feature::AuthElicitation),
                ),
            configured_mcp_servers,
            plugin_capability_summaries: loaded_plugins.capability_summaries().to_vec(),
        }
    }

    pub async fn rebuild_preserving_session_layers(
        &self,
        refreshed_config: &Config,
    ) -> std::io::Result<Self> {
        let mut layers = refreshed_config
            .config_layer_stack
            .get_layers(
                ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ true,
            )
            .into_iter()
            .filter(|layer| !is_session_layer(&layer.name))
            .cloned()
            .collect::<Vec<_>>();
        layers.extend(
            self.config_layer_stack
                .get_layers(
                    ConfigLayerStackOrdering::LowestPrecedenceFirst,
                    /*include_disabled*/ true,
                )
                .into_iter()
                .filter(|layer| is_session_layer(&layer.name))
                .cloned(),
        );
        layers.sort_by_key(|layer| layer.name.precedence());

        let config_layer_stack = ConfigLayerStack::new(
            layers,
            refreshed_config.config_layer_stack.requirements().clone(),
            refreshed_config
                .config_layer_stack
                .requirements_toml()
                .clone(),
        )?
        .with_user_and_project_exec_policy_rules_ignored(
            refreshed_config
                .config_layer_stack
                .ignore_user_and_project_exec_policy_rules(),
        );
        let cfg: ConfigToml = config_layer_stack
            .effective_config()
            .try_into()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        Self::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            cfg,
            ConfigOverrides {
                cwd: Some(self.cwd.to_path_buf()),
                ..Default::default()
            },
            refreshed_config.codex_home.clone(),
            config_layer_stack,
        )
        .await
    }

    /// This is the preferred way to create an instance of [Config].
    pub async fn load_with_cli_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        ConfigBuilder::default()
            .cli_overrides(cli_overrides)
            .build()
            .await
    }

    /// Load a default configuration when user config files are invalid.
    pub async fn load_default_with_cli_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        let codex_home = find_codex_home()?;
        Self::load_default_with_cli_overrides_for_codex_home(
            codex_home.to_path_buf(),
            cli_overrides,
        )
        .await
    }

    /// Load a default configuration for a specific Codex home without reading
    /// user, project, or system config layers.
    pub async fn load_default_with_cli_overrides_for_codex_home(
        codex_home: PathBuf,
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        let mut merged = toml::Value::try_from(ConfigToml::default()).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to serialize default config: {e}"),
            )
        })?;
        let cli_layer = build_cli_overrides_layer(&cli_overrides);
        merge_toml_values(&mut merged, &cli_layer);
        let codex_home = AbsolutePathBuf::from_absolute_path_checked(codex_home)?;
        let config_toml = deserialize_config_toml_with_base(merged, &codex_home)?;
        Self::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            config_toml,
            ConfigOverrides::default(),
            codex_home,
            ConfigLayerStack::default(),
        )
        .await
    }
    /// This is a secondary way of creating [Config], which is appropriate when
    /// the harness is meant to be used with a specific configuration that
    /// ignores user settings. For example, the `codex exec` subcommand is
    /// designed to use [AskForApproval::Never] exclusively.
    ///
    /// Further, [ConfigOverrides] contains some options that are not supported
    /// in [ConfigToml], such as `cwd`, `codex_self_exe`, `codex_linux_sandbox_exe`, and
    /// `main_execve_wrapper_exe`.
    pub async fn load_with_cli_overrides_and_harness_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
        harness_overrides: ConfigOverrides,
    ) -> std::io::Result<Self> {
        ConfigBuilder::default()
            .cli_overrides(cli_overrides)
            .harness_overrides(harness_overrides)
            .build()
            .await
    }
}

pub fn resolve_profile_v2_config_path(
    codex_home: &Path,
    profile_name: &ProfileV2Name,
) -> AbsolutePathBuf {
    AbsolutePathBuf::resolve_path_against_base(
        format!("{profile_name}{CONFIG_PROFILE_V2_SUFFIX}"),
        codex_home,
    )
}

/// DEPRECATED: Use [Config::load_with_cli_overrides()] instead because working
/// with [ConfigToml] directly means that [ConfigRequirements] have not been
/// applied yet, which risks failing to enforce required constraints.
pub async fn load_config_as_toml_with_cli_overrides(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    loader_overrides: LoaderOverrides,
) -> std::io::Result<ConfigToml> {
    load_config_as_toml_with_cli_and_loader_overrides(
        codex_home,
        cwd,
        cli_overrides,
        loader_overrides,
    )
    .await
}

/// DEPRECATED for most callers: prefer [Config::load_with_cli_overrides()] or
/// [ConfigBuilder] because working with [ConfigToml] directly means
/// [ConfigRequirements] have not been applied yet, which risks skipping
/// required constraints.
pub async fn load_config_as_toml_with_cli_and_loader_overrides(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    loader_overrides: LoaderOverrides,
) -> std::io::Result<ConfigToml> {
    load_config_as_toml_with_cli_and_load_options(codex_home, cwd, cli_overrides, loader_overrides)
        .await
}

/// DEPRECATED for most callers: prefer [Config::load_with_cli_overrides()] or
/// [ConfigBuilder] because working with [ConfigToml] directly means
/// [ConfigRequirements] have not been applied yet, which risks skipping
/// required constraints.
pub async fn load_config_as_toml_with_cli_and_load_options(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    options: impl Into<ConfigLoadOptions>,
) -> std::io::Result<ConfigToml> {
    let Some(config_layer_loader) = default_config_layer_loader_for_tests() else {
        return Err(missing_config_layer_loader_error());
    };
    load_config_as_toml_with_cli_and_load_options_and_layer_loader(
        codex_home,
        cwd,
        cli_overrides,
        options,
        config_layer_loader,
    )
    .await
}

pub async fn load_config_as_toml_with_cli_and_load_options_and_layer_loader(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    options: impl Into<ConfigLoadOptions>,
    config_layer_loader: Arc<dyn ConfigLayerLoader>,
) -> std::io::Result<ConfigToml> {
    let config_layer_stack = config_layer_loader
        .load(ConfigLayerLoadRequest {
            codex_home: AbsolutePathBuf::from_absolute_path(codex_home.to_path_buf())?,
            cwd: cwd.cloned(),
            cli_overrides,
            options: options.into(),
            cloud_requirements: CloudRequirementsLoader::default(),
            thread_config_loader: Arc::new(NoopThreadConfigLoader),
        })
        .await?;

    let merged_toml = config_layer_stack.effective_config();
    let cfg = deserialize_config_toml_with_base(merged_toml, codex_home).map_err(|e| {
        tracing::error!("Failed to deserialize overridden config: {e}");
        e
    })?;

    Ok(cfg)
}
