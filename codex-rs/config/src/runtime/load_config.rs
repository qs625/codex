use super::*;

impl Config {
    #[cfg(any(test, feature = "test-support"))]
    pub async fn load_from_base_config_with_overrides(
        cfg: ConfigToml,
        overrides: ConfigOverrides,
        codex_home: AbsolutePathBuf,
    ) -> std::io::Result<Self> {
        // Note this ignores requirements.toml enforcement for tests.
        let config_layer_stack = ConfigLayerStack::default();
        Self::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            cfg,
            overrides,
            codex_home,
            config_layer_stack,
        )
        .await
    }

    pub async fn load_config_with_layer_stack(
        fs: &dyn ExecutorFileSystem,
        cfg: ConfigToml,
        overrides: ConfigOverrides,
        codex_home: AbsolutePathBuf,
        config_layer_stack: ConfigLayerStack,
    ) -> std::io::Result<Self> {
        // Keep the large config-construction future off small test thread stacks.
        Box::pin(async move {
        if cfg.experimental_thread_store_endpoint.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "`experimental_thread_store_endpoint` is no longer supported; remove it from config.toml",
            ));
        }

        validate_model_providers(&cfg.model_providers)
            .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
        // Ensure that every field of ConfigRequirements is applied to the final
        // Config.
        let ConfigRequirements {
            approval_policy: mut constrained_approval_policy,
            approvals_reviewer: mut constrained_approvals_reviewer,
            permission_profile: mut constrained_permission_profile,
            web_search_mode: mut constrained_web_search_mode,
            allow_managed_hooks_only: _,
            feature_requirements,
            managed_hooks: _,
            mcp_servers,
            plugins: _,
            exec_policy: _,
            enforce_residency,
            network: network_requirements,
            filesystem: filesystem_requirements,
            guardian_policy_config_source: _,
        } = config_layer_stack.requirements().clone();

        let user_instructions = load_global_instructions(Some(&codex_home));
        let mut startup_warnings = config_layer_stack
            .startup_warnings()
            .unwrap_or_default()
            .to_vec();

        // Destructure ConfigOverrides fully to ensure all overrides are applied.
        let ConfigOverrides {
            model,
            review_model: override_review_model,
            cwd,
            approval_policy: approval_policy_override,
            approvals_reviewer: approvals_reviewer_override,
            sandbox_mode,
            permission_profile,
            default_permissions: default_permissions_override,
            model_provider,
            service_tier: service_tier_override,
            config_profile: config_profile_key,
            codex_self_exe,
            codex_linux_sandbox_exe,
            main_execve_wrapper_exe,
            zsh_path: zsh_path_override,
            base_instructions,
            developer_instructions,
            personality,
            compact_prompt,
            show_raw_agent_reasoning,
            tools_web_search_request: override_tools_web_search_request,
            ephemeral,
            bypass_hook_trust,
            additional_writable_roots,
            workspace_roots: workspace_roots_override,
        } = overrides;
        let bypass_hook_trust = bypass_hook_trust.unwrap_or_default();

        if bypass_hook_trust {
            startup_warnings.push(
                "`--dangerously-bypass-hook-trust` is enabled. Enabled hooks may run without review for this invocation."
                    .to_string(),
            );
        }

        if sandbox_mode.is_some() && permission_profile.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "`sandbox_mode` and `permission_profile` overrides cannot both be set",
            ));
        }
        if sandbox_mode.is_some() && default_permissions_override.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "`sandbox_mode` and `default_permissions` overrides cannot both be set",
            ));
        }
        if permission_profile.is_some() && default_permissions_override.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "`permission_profile` and `default_permissions` overrides cannot both be set",
            ));
        }

        let active_profile_name = config_profile_key
            .as_ref()
            .or(cfg.profile.as_ref())
            .cloned();
        let config_profile = match active_profile_name.as_ref() {
            Some(key) => cfg
                .profiles
                .get(key)
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("config profile `{key}` not found"),
                    )
                })?
                .clone(),
            None => ConfigProfile::default(),
        };
        let tool_suggest = resolve_tool_suggest_config(&cfg, &config_layer_stack);
        let feature_overrides = FeatureOverrides {
            web_search_request: override_tools_web_search_request,
        };

        let configured_features = Features::from_sources(
            FeatureConfigSource {
                features: cfg.features.as_ref(),
                experimental_use_unified_exec_tool: cfg.experimental_use_unified_exec_tool,
            },
            FeatureConfigSource {
                features: config_profile.features.as_ref(),
                experimental_use_unified_exec_tool: config_profile
                    .experimental_use_unified_exec_tool,
            },
            feature_overrides,
        );
        let features = ManagedFeatures::from_configured_with_warnings(
            configured_features,
            feature_requirements,
            &mut startup_warnings,
        )?;
        let enable_network_proxy = features.enabled(Feature::NetworkProxy);
        let windows_sandbox_mode = resolve_windows_sandbox_mode(&cfg, &config_profile);
        let windows_sandbox_private_desktop =
            resolve_windows_sandbox_private_desktop(&cfg, &config_profile);
        let resolved_cwd = AbsolutePathBuf::try_from(normalize_for_native_workdir({
            use std::env;

            match cwd {
                None => {
                    tracing::info!("cwd not set, using current dir");
                    env::current_dir()?
                }
                Some(p) if p.is_absolute() => p,
                Some(p) => {
                    // Resolve relative path against the current working directory.
                    tracing::info!("cwd is relative, resolving against current dir");
                    let mut current = env::current_dir()?;
                    current.push(p);
                    current
                }
            }
        }))?;
        let requested_additional_writable_roots: Vec<AbsolutePathBuf> = additional_writable_roots
            .into_iter()
            .map(|path| AbsolutePathBuf::resolve_path_against_base(path, resolved_cwd.as_path()))
            .collect();
        let repo_root = resolve_root_git_project_for_trust(fs, &resolved_cwd).await;
        let active_project = cfg
            .get_active_project(
                resolved_cwd.as_path(),
                repo_root.as_ref().map(AbsolutePathBuf::as_path),
            )
            .unwrap_or(ProjectConfig { trust_level: None });
        let permission_config_syntax = resolve_permission_config_syntax(
            &config_layer_stack,
            &cfg,
            sandbox_mode,
            config_profile.sandbox_mode,
        );
        let has_permission_profiles = cfg
            .permissions
            .as_ref()
            .is_some_and(|profiles| !profiles.is_empty());
        let default_permissions = default_permissions_override
            .as_deref()
            .or(cfg.default_permissions.as_deref());
        validate_user_permission_profile_names(cfg.permissions.as_ref())?;
        if has_permission_profiles
            && !matches!(
                permission_config_syntax,
                Some(PermissionConfigSyntax::Legacy)
            )
            && default_permissions.is_none()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "config defines `[permissions]` profiles but does not set `default_permissions`",
            ));
        }

        let windows_sandbox_level = match windows_sandbox_mode {
            Some(WindowsSandboxModeToml::Elevated) => WindowsSandboxLevel::Elevated,
            Some(WindowsSandboxModeToml::Unelevated) => WindowsSandboxLevel::RestrictedToken,
            None => windows_sandbox_level_from_features(&features),
        };
        let memories_root = memory_root(&codex_home);
        std::fs::create_dir_all(&memories_root)?;
        let internal_writable_roots = vec![memories_root];

        let profiles_are_active = default_permissions_override.is_some()
            || matches!(
                permission_config_syntax,
                Some(PermissionConfigSyntax::Profiles)
            )
            || permission_config_syntax.is_none();
        let using_implicit_builtin_profile =
            permission_config_syntax.is_none() && default_permissions.is_none();
        let should_seed_legacy_workspace_roots = default_permissions.is_none()
            && matches!(
                permission_config_syntax,
                None | Some(PermissionConfigSyntax::Legacy)
            );
        let legacy_workspace_roots_explicit = should_seed_legacy_workspace_roots
            && cfg
                .sandbox_workspace_write
                .as_ref()
                .is_some_and(|sandbox_workspace_write| {
                    !sandbox_workspace_write.writable_roots.is_empty()
                });
        let workspace_roots_explicit = workspace_roots_override.is_some()
            || !requested_additional_writable_roots.is_empty()
            || legacy_workspace_roots_explicit;
        let mut workspace_roots = match workspace_roots_override {
            Some(workspace_roots) => workspace_roots
                .into_iter()
                .map(|path| {
                    AbsolutePathBuf::resolve_path_against_base(path, resolved_cwd.as_path())
                })
                .collect(),
            None => {
                let mut workspace_roots = vec![resolved_cwd.clone()];
                workspace_roots.extend(requested_additional_writable_roots.clone());
                if should_seed_legacy_workspace_roots
                    && let Some(sandbox_workspace_write) = cfg.sandbox_workspace_write.as_ref()
                {
                    workspace_roots.extend(sandbox_workspace_write.writable_roots.clone());
                }
                workspace_roots
            }
        };
        dedupe_absolute_paths(&mut workspace_roots);
        let (
            mut configured_network_proxy_config,
            permission_profile,
            file_system_sandbox_policy,
            mut active_permission_profile,
            mut profile_workspace_roots,
        ) = if let Some(mut permission_profile) = permission_profile {
            let (mut file_system_sandbox_policy, network_sandbox_policy) =
                permission_profile.to_runtime_permissions();
            let configured_network_proxy_config =
                if profile_allows_configured_network_proxy(&permission_profile)
                    && profiles_are_active
                {
                    // PermissionProfile carries the active network sandbox bit, not the configured
                    // proxy/allowlist policy. Keep that config so active profiles can round-trip
                    // without broadening network behavior.
                    let default_permissions = default_permissions.unwrap_or_else(|| {
                        default_builtin_permission_profile_name(
                            &active_project,
                            windows_sandbox_level,
                        )
                    });
                    network_proxy_config_for_profile_selection(
                        cfg.permissions.as_ref(),
                        default_permissions,
                    )?
                } else {
                    NetworkProxyConfig::default()
                };
            let materialized_file_system_sandbox_policy = file_system_sandbox_policy
                .clone()
                .materialize_project_roots_with_workspace_roots(&workspace_roots);
            let materialized_permission_profile =
                PermissionProfile::from_runtime_permissions_with_enforcement(
                    permission_profile.enforcement(),
                    &materialized_file_system_sandbox_policy,
                    network_sandbox_policy,
                );
            let sandbox_policy = compatibility_sandbox_policy_for_permission_profile(
                &materialized_permission_profile,
                &materialized_file_system_sandbox_policy,
                network_sandbox_policy,
                resolved_cwd.as_path(),
            );
            if matches!(sandbox_policy, SandboxPolicy::WorkspaceWrite { .. }) {
                file_system_sandbox_policy = file_system_sandbox_policy
                    .with_additional_legacy_workspace_writable_roots(&internal_writable_roots);
                permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
                    permission_profile.enforcement(),
                    &file_system_sandbox_policy,
                    network_sandbox_policy,
                );
            }
            (
                configured_network_proxy_config,
                permission_profile,
                file_system_sandbox_policy,
                None,
                Vec::new(),
            )
        } else if profiles_are_active {
            let default_permissions = default_permissions.unwrap_or_else(|| {
                default_builtin_permission_profile_name(&active_project, windows_sandbox_level)
            });
            let builtin_workspace_write_settings = if using_implicit_builtin_profile {
                cfg.sandbox_workspace_write.as_ref()
            } else {
                None
            };
            let configured_network_proxy_config = network_proxy_config_for_profile_selection(
                cfg.permissions.as_ref(),
                default_permissions,
            )?;
            let (mut file_system_sandbox_policy, network_sandbox_policy) =
                compile_permission_profile_selection(
                    cfg.permissions.as_ref(),
                    default_permissions,
                    builtin_workspace_write_settings,
                    resolved_cwd.as_path(),
                    &mut startup_warnings,
                )?;
            let mut configured_workspace_roots = compile_permission_profile_workspace_roots(
                cfg.permissions.as_ref(),
                default_permissions,
                resolved_cwd.as_path(),
            )?;
            if using_implicit_builtin_profile
                && default_permissions == BUILT_IN_WORKSPACE_PROFILE
                && let Some(sandbox_workspace_write) = cfg.sandbox_workspace_write.as_ref()
            {
                configured_workspace_roots.extend(sandbox_workspace_write.writable_roots.clone());
            }
            dedupe_absolute_paths(&mut configured_workspace_roots);
            file_system_sandbox_policy = file_system_sandbox_policy
                .with_materialized_project_roots_for_workspace_roots(&configured_workspace_roots);
            let mut permission_profile = if let Some(permission_profile) =
                builtin_permission_profile(default_permissions, builtin_workspace_write_settings)
            {
                permission_profile
            } else {
                PermissionProfile::from_runtime_permissions(
                    &file_system_sandbox_policy,
                    network_sandbox_policy,
                )
            };
            let materialized_file_system_sandbox_policy = file_system_sandbox_policy
                .clone()
                .materialize_project_roots_with_workspace_roots(&workspace_roots);
            let materialized_permission_profile =
                PermissionProfile::from_runtime_permissions_with_enforcement(
                    permission_profile.enforcement(),
                    &materialized_file_system_sandbox_policy,
                    network_sandbox_policy,
                );
            let sandbox_policy = compatibility_sandbox_policy_for_permission_profile(
                &materialized_permission_profile,
                &materialized_file_system_sandbox_policy,
                network_sandbox_policy,
                resolved_cwd.as_path(),
            );
            if matches!(sandbox_policy, SandboxPolicy::WorkspaceWrite { .. }) {
                file_system_sandbox_policy = file_system_sandbox_policy
                    .with_additional_legacy_workspace_writable_roots(&internal_writable_roots);
                permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
                    permission_profile.enforcement(),
                    &file_system_sandbox_policy,
                    network_sandbox_policy,
                );
            }
            let active_permission_profile = if using_implicit_builtin_profile
                && default_permissions == BUILT_IN_WORKSPACE_PROFILE
                && cfg.sandbox_workspace_write.is_some()
            {
                // The implicit built-in profile preserves legacy
                // `[sandbox_workspace_write]` customizations, but explicitly
                // selecting `:workspace` intentionally ignores those legacy
                // settings. Do not advertise a re-selectable active profile
                // when doing so would lose roots, network, or tmp settings.
                None
            } else {
                Some(ActivePermissionProfile::new(default_permissions))
            };
            (
                configured_network_proxy_config,
                permission_profile,
                file_system_sandbox_policy,
                active_permission_profile,
                configured_workspace_roots,
            )
        } else {
            let configured_network_proxy_config = NetworkProxyConfig::default();
            // No named `[permissions]` profile is active, but permissions
            // should still flow through the canonical profile representation.
            // Derive the old `sandbox_mode` defaults as a profile first, then
            // keep a legacy-compatible projection only for the remaining code
            // paths that still speak `SandboxPolicy`.
            let mut permission_profile = cfg
                .derive_permission_profile(
                    sandbox_mode,
                    config_profile.sandbox_mode,
                    windows_sandbox_level,
                    Some(&active_project),
                    Some(&constrained_permission_profile),
                )
                .await;
            // The legacy-derived profiles above are expected to be
            // representable as `SandboxPolicy`. This guard keeps the old safe
            // fallback behavior if future changes make this branch derive a
            // profile with split-only filesystem semantics, such as root write
            // with carveouts or writes that are not expressible as
            // workspace-write roots.
            if let Err(err) = permission_profile.to_legacy_sandbox_policy(resolved_cwd.as_path()) {
                tracing::warn!(
                    error = %err,
                    "derived permission profile cannot be represented as a legacy sandbox policy; falling back to read-only"
                );
                permission_profile = PermissionProfile::read_only();
            }
            let (mut file_system_sandbox_policy, network_sandbox_policy) =
                permission_profile.to_runtime_permissions();
            let materialized_file_system_sandbox_policy = permission_profile
                .clone()
                .materialize_project_roots_with_workspace_roots(&workspace_roots)
                .file_system_sandbox_policy();
            if matches!(permission_profile.enforcement(), SandboxEnforcement::Managed)
                && materialized_file_system_sandbox_policy.can_write_path_with_cwd(
                    resolved_cwd.as_path(),
                    resolved_cwd.as_path(),
                )
                && !materialized_file_system_sandbox_policy.has_full_disk_write_access()
            {
                // Keep Codex runtime write access while storing the runtime
                // workspace roots separately on the thread.
                file_system_sandbox_policy = file_system_sandbox_policy
                    .with_additional_legacy_workspace_writable_roots(&internal_writable_roots);
                permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
                    permission_profile.enforcement(),
                    &file_system_sandbox_policy,
                    network_sandbox_policy,
                );
            }
            (
                configured_network_proxy_config,
                permission_profile,
                file_system_sandbox_policy,
                None,
                Vec::new(),
            )
        };
        if enable_network_proxy && permission_profile.network_sandbox_policy().is_enabled() {
            if let Some(network_proxy) = network_proxy_toml_config(cfg.features.as_ref()) {
                apply_network_proxy_feature_config(
                    &mut configured_network_proxy_config,
                    network_proxy,
                );
            }
            if let Some(network_proxy) = network_proxy_toml_config(config_profile.features.as_ref())
            {
                apply_network_proxy_feature_config(
                    &mut configured_network_proxy_config,
                    network_proxy,
                );
            }
            configured_network_proxy_config.network.enabled = true;
        }
        let approval_policy_was_explicit = approval_policy_override.is_some()
            || config_profile.approval_policy.is_some()
            || cfg.approval_policy.is_some();
        let mut approval_policy = approval_policy_override
            .or(config_profile.approval_policy)
            .or(cfg.approval_policy)
            .unwrap_or_else(|| {
                if active_project.is_trusted() {
                    AskForApproval::OnRequest
                } else if active_project.is_untrusted() {
                    AskForApproval::UnlessTrusted
                } else {
                    AskForApproval::default()
                }
            });
        if !approval_policy_was_explicit
            && let Err(err) = constrained_approval_policy.can_set(&approval_policy)
        {
            tracing::warn!(
                error = %err,
                "default approval policy is disallowed by requirements; falling back to required default"
            );
            approval_policy = constrained_approval_policy.value();
        }
        let approvals_reviewer_was_explicit = approvals_reviewer_override.is_some()
            || config_profile.approvals_reviewer.is_some()
            || cfg.approvals_reviewer.is_some();
        let mut approvals_reviewer = approvals_reviewer_override
            .or(config_profile.approvals_reviewer)
            .or(cfg.approvals_reviewer)
            .unwrap_or(ApprovalsReviewer::User);
        if !approvals_reviewer_was_explicit
            && let Err(err) = constrained_approvals_reviewer.can_set(&approvals_reviewer)
        {
            tracing::warn!(
                error = %err,
                "default approvals reviewer is disallowed by requirements; falling back to required default"
            );
            approvals_reviewer = constrained_approvals_reviewer.value();
        }
        let web_search_mode = resolve_web_search_mode(&cfg, &config_profile, &features)
            .unwrap_or(WebSearchMode::Cached);
        let web_search_config = resolve_web_search_config(&cfg, &config_profile);
        let mut multi_agent_v2 = resolve_multi_agent_v2_config(&cfg, &config_profile);
        let apps_mcp_path_override = if features.enabled(Feature::AppsMcpPathOverride) {
            let base = apps_mcp_path_override_toml_config(cfg.features.as_ref());
            let profile = apps_mcp_path_override_toml_config(config_profile.features.as_ref());
            profile
                .and_then(|config| config.path.as_ref())
                .or_else(|| base.and_then(|config| config.path.as_ref()))
                .cloned()
                .or_else(|| Some("/ps/mcp".to_string()))
        } else {
            None
        };
        let terminal_resize_reflow = resolve_terminal_resize_reflow_config(&cfg);

        let agent_roles = agent_roles::load_agent_roles(
            fs,
            &cfg,
            &config_layer_stack,
            &codex_home,
            &resolved_cwd,
            &mut startup_warnings,
        )
        .await?;

        let openai_base_url = cfg
            .openai_base_url
            .clone()
            .filter(|value| !value.is_empty());

        let model_options = cfg.model_options.clone();
        validate_model_options(&model_options)
            .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidData, message))?;
        let mut configured_model_providers = cfg.model_providers;
        for model_option in &model_options {
            if model_option.has_inline_provider_config()
                && !configured_model_providers.contains_key(&model_option.provider)
            {
                configured_model_providers
                    .insert(model_option.provider.clone(), model_option.to_provider_info());
            }
        }
        validate_model_providers(&configured_model_providers)
            .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidData, message))?;
        let model_providers = merge_configured_model_providers(
            built_in_model_providers(openai_base_url),
            configured_model_providers,
        )
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidData, message))?;

        let model_provider_id = model_provider
            .or(config_profile.model_provider)
            .or(cfg.model_provider)
            .unwrap_or_else(|| "openai".to_string());
        let model_provider = model_providers
            .get(&model_provider_id)
            .ok_or_else(|| {
                let message = if model_provider_id == LEGACY_OLLAMA_CHAT_PROVIDER_ID {
                    OLLAMA_CHAT_PROVIDER_REMOVED_ERROR.to_string()
                } else {
                    format!("Model provider `{model_provider_id}` not found")
                };
                std::io::Error::new(std::io::ErrorKind::NotFound, message)
            })?
            .clone();

        let shell_environment_policy = cfg.shell_environment_policy.into();
        let allow_login_shell = cfg.allow_login_shell.unwrap_or(true);

        let history = cfg.history.unwrap_or_default();

        if multi_agent_v2.max_concurrent_threads_per_session == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "features.multi_agent_v2.max_concurrent_threads_per_session must be at least 1",
            ));
        }
        validate_multi_agent_v2_wait_timeout(
            "features.multi_agent_v2.min_wait_timeout_ms",
            multi_agent_v2.min_wait_timeout_ms,
        )?;
        validate_multi_agent_v2_wait_timeout(
            "features.multi_agent_v2.max_wait_timeout_ms",
            multi_agent_v2.max_wait_timeout_ms,
        )?;
        validate_multi_agent_v2_wait_timeout(
            "features.multi_agent_v2.default_wait_timeout_ms",
            multi_agent_v2.default_wait_timeout_ms,
        )?;
        if multi_agent_v2.min_wait_timeout_ms > multi_agent_v2.max_wait_timeout_ms {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "features.multi_agent_v2.min_wait_timeout_ms must be at most features.multi_agent_v2.max_wait_timeout_ms",
            ));
        }
        if multi_agent_v2.default_wait_timeout_ms < multi_agent_v2.min_wait_timeout_ms {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "features.multi_agent_v2.default_wait_timeout_ms must be at least features.multi_agent_v2.min_wait_timeout_ms",
            ));
        }
        if multi_agent_v2.default_wait_timeout_ms > multi_agent_v2.max_wait_timeout_ms {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "features.multi_agent_v2.default_wait_timeout_ms must be at most features.multi_agent_v2.max_wait_timeout_ms",
            ));
        }
        let agent_max_threads_from_config =
            cfg.agents.as_ref().and_then(|agents| agents.max_threads);
        if let Some(agent_max_threads) = agent_max_threads_from_config {
            if agent_max_threads == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "agents.max_threads must be at least 1",
                ));
            }
            multi_agent_v2.max_concurrent_threads_per_session = agent_max_threads.saturating_add(1);
        }
        let agent_max_threads = agent_max_threads_from_config.or(Some(
            multi_agent_v2
                .max_concurrent_threads_per_session
                .saturating_sub(1),
        ));
        let agent_max_depth = cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.max_depth)
            .unwrap_or(DEFAULT_AGENT_MAX_DEPTH);
        if agent_max_depth < 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agents.max_depth must be at least 1",
            ));
        }
        let agent_job_max_runtime_seconds = cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.job_max_runtime_seconds)
            .or(DEFAULT_AGENT_JOB_MAX_RUNTIME_SECONDS);
        if agent_job_max_runtime_seconds == Some(0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agents.job_max_runtime_seconds must be at least 1",
            ));
        }
        if let Some(max_runtime_seconds) = agent_job_max_runtime_seconds
            && max_runtime_seconds > i64::MAX as u64
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agents.job_max_runtime_seconds must fit within a 64-bit signed integer",
            ));
        }
        let agent_interrupt_message_enabled = cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.interrupt_message)
            .unwrap_or(true);
        let background_terminal_max_timeout = cfg
            .background_terminal_max_timeout
            .unwrap_or(DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS);

        let ghost_snapshot = {
            let mut config = GhostSnapshotConfig::default();
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(ignore_over_bytes) = ghost_snapshot.ignore_large_untracked_files
            {
                config.ignore_large_untracked_files = if ignore_over_bytes > 0 {
                    Some(ignore_over_bytes)
                } else {
                    None
                };
            }
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(threshold) = ghost_snapshot.ignore_large_untracked_dirs
            {
                config.ignore_large_untracked_dirs =
                    if threshold > 0 { Some(threshold) } else { None };
            }
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(disable_warnings) = ghost_snapshot.disable_warnings
            {
                config.disable_warnings = disable_warnings;
            }
            config
        };

        let use_experimental_unified_exec_tool = features.enabled(Feature::UnifiedExec);

        let forced_chatgpt_workspace_id = cfg
            .forced_chatgpt_workspace_id
            .clone()
            .map(ForcedChatgptWorkspaceIds::into_vec)
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|values| !values.is_empty());

        let forced_login_method = cfg.forced_login_method;

        let model = model.or(config_profile.model).or(cfg.model);
        let mut notices = cfg.notice.unwrap_or_default();
        let service_tier = match service_tier_override {
            Some(Some(service_tier)) => Some(service_tier),
            Some(None) => {
                // Preserve explicit standard/clear intent after the nested override
                // collapses into `Config.service_tier = None`.
                notices.fast_default_opt_out = Some(true);
                None
            }
            None => config_profile.service_tier.or(cfg.service_tier),
        };
        let service_tier = service_tier.and_then(|service_tier| {
            match ServiceTier::from_request_value(&service_tier) {
                Some(ServiceTier::Fast) => features
                    .enabled(Feature::FastMode)
                    .then(|| ServiceTier::Fast.request_value().to_string()),
                Some(ServiceTier::Flex) => Some(ServiceTier::Flex.request_value().to_string()),
                None => Some(service_tier),
            }
        });

        let compact_prompt = compact_prompt.or(cfg.compact_prompt).and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        // Load base instructions override from a file if specified. If the
        // path is relative, resolve it against the effective cwd so the
        // behaviour matches other path-like config values.
        let model_instructions_path = config_profile
            .model_instructions_file
            .as_ref()
            .or(cfg.model_instructions_file.as_ref());
        let file_base_instructions = Self::try_read_non_empty_file(
            fs,
            model_instructions_path,
            "model instructions file",
        )
        .await?;
        let base_instructions = base_instructions
            .or(file_base_instructions)
            .or(cfg.instructions.clone());
        let developer_instructions = developer_instructions.or(cfg.developer_instructions);
        let include_permissions_instructions = config_profile
            .include_permissions_instructions
            .or(cfg.include_permissions_instructions)
            .unwrap_or(true);
        let include_apps_instructions = config_profile
            .include_apps_instructions
            .or(cfg.include_apps_instructions)
            .unwrap_or(true);
        let include_collaboration_mode_instructions = config_profile
            .include_collaboration_mode_instructions
            .or(cfg.include_collaboration_mode_instructions)
            .unwrap_or(true);
        let include_skill_instructions = cfg
            .skills
            .as_ref()
            .and_then(|skills| skills.include_instructions)
            .unwrap_or(true);
        let include_environment_context = config_profile
            .include_environment_context
            .or(cfg.include_environment_context)
            .unwrap_or(true);
        let guardian_policy_config =
            guardian_policy_config_from_requirements(config_layer_stack.requirements_toml())
                .or_else(|| {
                    cfg.auto_review
                        .as_ref()
                        .and_then(|auto_review| normalize_guardian_policy_config(
                            auto_review.policy.as_deref(),
                        ))
                });
        let personality = personality
            .or(config_profile.personality)
            .or(cfg.personality)
            .or_else(|| {
                features
                    .enabled(Feature::Personality)
                    .then_some(Personality::Pragmatic)
            });

        let experimental_compact_prompt_path = config_profile
            .experimental_compact_prompt_file
            .as_ref()
            .or(cfg.experimental_compact_prompt_file.as_ref());
        let file_compact_prompt = Self::try_read_non_empty_file(
            fs,
            experimental_compact_prompt_path,
            "experimental compact prompt file",
        )
        .await?;
        let default_compact_prompt =
            if compact_prompt.is_some() || experimental_compact_prompt_path.is_some() {
            None
        } else {
            let workspace_compact_prompt_path = AbsolutePathBuf::try_from(
                resolved_cwd
                    .join(".codex")
                    .join("compact")
                    .join("COMPACT.md"),
            )
            .expect("workspace compact prompt path should remain absolute");
            match Self::try_read_non_empty_file(
                fs,
                Some(&workspace_compact_prompt_path),
                "workspace compact prompt file",
            )
            .await
            {
                Ok(prompt) => prompt,
                Err(err) if Self::is_default_compact_prompt_fallback(&err) => {
                    let codex_home_compact_prompt_path =
                        AbsolutePathBuf::try_from(codex_home.join("compact").join("COMPACT.md"))
                            .expect("Codex home compact prompt path should remain absolute");
                    match Self::try_read_non_empty_file(
                        fs,
                        Some(&codex_home_compact_prompt_path),
                        "Codex home compact prompt file",
                    )
                    .await
                    {
                        Ok(prompt) => prompt,
                        Err(err) if Self::is_default_compact_prompt_fallback(&err) => None,
                        Err(err) => return Err(err),
                    }
                }
                Err(err) => return Err(err),
            }
        };
        let compact_prompt = compact_prompt
            .or(file_compact_prompt)
            .or(default_compact_prompt);
        let zsh_path = zsh_path_override
            .or(config_profile.zsh_path.map(Into::into))
            .or(cfg.zsh_path.map(Into::into));

        let review_model = override_review_model.or(cfg.review_model);

        let check_for_update_on_startup = cfg.check_for_update_on_startup.unwrap_or(true);
        let model_catalog = load_model_catalog(
            config_profile
                .model_catalog_json
                .clone()
                .or(cfg.model_catalog_json.clone()),
        )?;

        let log_dir = cfg
            .log_dir
            .as_ref()
            .map(AbsolutePathBuf::to_path_buf)
            .unwrap_or_else(|| codex_home.join("log").to_path_buf());
        let sqlite_home = cfg
            .sqlite_home
            .as_ref()
            .map(AbsolutePathBuf::to_path_buf)
            .or_else(|| resolve_sqlite_home_env(&resolved_cwd))
            .unwrap_or_else(|| codex_home.to_path_buf());
        let original_permission_profile = permission_profile.clone();
        apply_requirement_constrained_value(
            "approval_policy",
            approval_policy,
            &mut constrained_approval_policy,
            &mut startup_warnings,
        )?;
        if let Some(Sourced {
            value: filesystem_requirements,
            source: filesystem_requirements_source,
        }) = filesystem_requirements.as_ref()
            && !filesystem_requirements.deny_read.is_empty()
        {
            let requirement_source = filesystem_requirements_source.clone();
            constrained_permission_profile
                .value
                .add_validator(move |permission_profile| {
                    let mode = sandbox_mode_requirement_for_permission_profile(permission_profile);
                    match mode {
                        SandboxModeRequirement::ReadOnly
                        | SandboxModeRequirement::WorkspaceWrite => Ok(()),
                        SandboxModeRequirement::DangerFullAccess
                        | SandboxModeRequirement::ExternalSandbox => {
                            Err(ConstraintError::InvalidValue {
                                field_name: "sandbox_mode",
                                candidate: format!("{mode:?}"),
                                allowed: "[read-only, workspace-write]".to_string(),
                                requirement_source: requirement_source.clone(),
                            })
                        }
                    }
                })
                .map_err(std::io::Error::from)?;
        }
        apply_requirement_constrained_value(
            "approvals_reviewer",
            approvals_reviewer,
            &mut constrained_approvals_reviewer,
            &mut startup_warnings,
        )?;
        let permission_profile_was_constrained = apply_requirement_constrained_value(
            "permission_profile",
            permission_profile,
            &mut constrained_permission_profile,
            &mut startup_warnings,
        )?;
        if permission_profile_was_constrained {
            // The selected profile no longer describes the effective
            // permissions after requirements forced a fallback.
            active_permission_profile = None;
            profile_workspace_roots.clear();
        }
        apply_requirement_constrained_value(
            "web_search_mode",
            web_search_mode,
            &mut constrained_web_search_mode,
            &mut startup_warnings,
        )?;

        let mcp_servers = constrain_mcp_servers(cfg.mcp_servers.clone(), mcp_servers.as_ref())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e}")))?;

        let (network_requirements, network_requirements_source) = match network_requirements {
            Some(Sourced { value, source }) => (Some(value), Some(source)),
            None => (None, None),
        };
        let has_network_requirements = network_requirements.is_some();
        let network_permission_profile = constrained_permission_profile.get().clone();
        let network = NetworkProxySpec::from_config_and_constraints(
            configured_network_proxy_config,
            network_requirements,
            &network_permission_profile,
        )
        .map_err(|err| {
            if let Some(source) = network_requirements_source.as_ref() {
                std::io::Error::new(
                    err.kind(),
                    format!("failed to build managed network proxy from {source}: {err}"),
                )
            } else {
                err
            }
        })?;
        let network = if has_network_requirements {
            Some(network)
        } else {
            network.enabled().then_some(network)
        };
        let helper_readable_roots = get_readable_roots_required_for_codex_runtime(
            &codex_home,
            zsh_path.as_ref(),
            main_execve_wrapper_exe.as_ref(),
        );
        let effective_permission_profile = constrained_permission_profile.value.get().clone();
        let (mut effective_file_system_sandbox_policy, effective_network_sandbox_policy) =
            effective_permission_profile.to_runtime_permissions();
        if effective_permission_profile != original_permission_profile {
            effective_file_system_sandbox_policy
                .preserve_deny_read_restrictions_from(&file_system_sandbox_policy);
        }
        if let Some(Sourced {
            value: filesystem_requirements,
            ..
        }) = filesystem_requirements.as_ref()
        {
            apply_managed_filesystem_constraints(
                &mut effective_file_system_sandbox_policy,
                filesystem_requirements,
            );
        }
        let effective_file_system_sandbox_policy = effective_file_system_sandbox_policy
            .with_additional_readable_roots(resolved_cwd.as_path(), &helper_readable_roots);
        let effective_permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
            effective_permission_profile.enforcement(),
            &effective_file_system_sandbox_policy,
            effective_network_sandbox_policy,
        );
        constrained_permission_profile
            .value
            .set(effective_permission_profile)
            .map_err(std::io::Error::from)?;
        let permission_profile_state = PermissionProfileState::from_constrained_active_profile(
            constrained_permission_profile.value,
            active_permission_profile,
            profile_workspace_roots,
        )
        .map_err(std::io::Error::from)?;
        let otel = otel::resolve_config(cfg.otel.unwrap_or_default(), &mut startup_warnings);
        let config = Self {
            model,
            service_tier,
            review_model,
            model_context_window: cfg.model_context_window,
            model_auto_compact_token_limit: cfg.model_auto_compact_token_limit,
            model_provider_id,
            model_provider,
            model_options,
            cwd: resolved_cwd,
            workspace_roots: workspace_roots.clone(),
            workspace_roots_explicit,
            startup_warnings,
            permissions: Permissions {
                approval_policy: constrained_approval_policy.value,
                permission_profile_state,
                workspace_roots,
                network,
                allow_login_shell,
                shell_environment_policy,
                windows_sandbox_mode,
                windows_sandbox_private_desktop,
            },
            approvals_reviewer: constrained_approvals_reviewer.value(),
            enforce_residency: enforce_residency.value,
            notify: cfg.notify,
            user_instructions,
            base_instructions,
            personality,
            developer_instructions,
            compact_prompt,
            include_permissions_instructions,
            include_apps_instructions,
            include_collaboration_mode_instructions,
            include_skill_instructions,
            include_environment_context,
            // The config.toml omits "_mode" because it's a config file. However, "_mode"
            // is important in code to differentiate the mode from the store implementation.
            cli_auth_credentials_store_mode: resolve_cli_auth_credentials_store_mode(
                cfg.cli_auth_credentials_store.unwrap_or_default(),
                env!("CARGO_PKG_VERSION"),
            ),
            mcp_servers,
            // The config.toml omits "_mode" because it's a config file. However, "_mode"
            // is important in code to differentiate the mode from the store implementation.
            mcp_oauth_credentials_store_mode: resolve_mcp_oauth_credentials_store_mode(
                cfg.mcp_oauth_credentials_store.unwrap_or_default(),
                env!("CARGO_PKG_VERSION"),
            ),
            mcp_oauth_callback_port: cfg.mcp_oauth_callback_port,
            mcp_oauth_callback_url: cfg.mcp_oauth_callback_url.clone(),
            model_providers,
            project_doc_max_bytes: cfg.project_doc_max_bytes.unwrap_or(AGENTS_MD_MAX_BYTES),
            project_doc_fallback_filenames: cfg
                .project_doc_fallback_filenames
                .unwrap_or_default()
                .into_iter()
                .filter_map(|name| {
                    let trimmed = name.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect(),
            tool_output_token_limit: cfg.tool_output_token_limit,
            agent_max_threads,
            agent_max_depth,
            agent_roles,
            agent_tool_patterns: None,
            agent_skill_patterns: None,
            memories: cfg.memories.unwrap_or_default().into(),
            agent_job_max_runtime_seconds,
            agent_interrupt_message_enabled,
            codex_home,
            sqlite_home,
            log_dir,
            config_lock_export_dir: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.export_dir.clone()),
            config_lock_allow_codex_version_mismatch: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.allow_codex_version_mismatch)
                .unwrap_or(false),
            config_lock_save_fields_resolved_from_model_catalog: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.save_fields_resolved_from_model_catalog)
                .unwrap_or(true),
            config_lock_toml: None,
            config_layer_stack,
            history,
            ephemeral: ephemeral.unwrap_or_default(),
            bypass_hook_trust,
            file_opener: cfg.file_opener.unwrap_or(UriBasedFileOpener::VsCode),
            codex_self_exe,
            codex_linux_sandbox_exe,
            main_execve_wrapper_exe,
            zsh_path,

            hide_agent_reasoning: cfg.hide_agent_reasoning.unwrap_or(false),
            show_raw_agent_reasoning: cfg
                .show_raw_agent_reasoning
                .or(show_raw_agent_reasoning)
                .unwrap_or(false),
            guardian_policy_config,
            model_reasoning_effort: config_profile
                .model_reasoning_effort
                .or(cfg.model_reasoning_effort),
            plan_mode_reasoning_effort: config_profile
                .plan_mode_reasoning_effort
                .or(cfg.plan_mode_reasoning_effort),
            model_reasoning_summary: config_profile
                .model_reasoning_summary
                .or(cfg.model_reasoning_summary),
            model_supports_reasoning_summaries: cfg.model_supports_reasoning_summaries,
            model_catalog,
            model_verbosity: config_profile.model_verbosity.or(cfg.model_verbosity),
            chatgpt_base_url: config_profile
                .chatgpt_base_url
                .or(cfg.chatgpt_base_url)
                .unwrap_or("https://chatgpt.com/backend-api/".to_string()),
            apps_mcp_path_override,
            realtime_audio: cfg
                .audio
                .map_or_else(RealtimeAudioConfig::default, |audio| RealtimeAudioConfig {
                    microphone: audio.microphone,
                    speaker: audio.speaker,
                }),
            experimental_realtime_ws_base_url: cfg.experimental_realtime_ws_base_url,
            experimental_realtime_ws_model: cfg.experimental_realtime_ws_model,
            realtime: cfg
                .realtime
                .map_or_else(RealtimeConfig::default, |realtime| {
                    let defaults = RealtimeConfig::default();
                    RealtimeConfig {
                        version: realtime.version.unwrap_or(defaults.version),
                        session_type: realtime.session_type.unwrap_or(defaults.session_type),
                        transport: realtime.transport.unwrap_or(defaults.transport),
                        voice: realtime.voice,
                    }
                }),
            experimental_realtime_ws_backend_prompt: cfg.experimental_realtime_ws_backend_prompt,
            experimental_realtime_ws_startup_context: cfg.experimental_realtime_ws_startup_context,
            experimental_realtime_start_instructions: cfg.experimental_realtime_start_instructions,
            experimental_thread_config_endpoint: cfg.experimental_thread_config_endpoint,
            experimental_thread_store: thread_store_config(cfg.experimental_thread_store),
            forced_chatgpt_workspace_id,
            forced_login_method,
            web_search_mode: constrained_web_search_mode.value,
            web_search_config,
            use_experimental_unified_exec_tool,
            background_terminal_max_timeout,
            ghost_snapshot,
            multi_agent_v2,
            features,
            suppress_unstable_features_warning: cfg
                .suppress_unstable_features_warning
                .unwrap_or(false),
            active_profile: active_profile_name,
            active_project,
            notices,
            check_for_update_on_startup,
            disable_paste_burst: cfg.disable_paste_burst.unwrap_or(false),
            analytics_enabled: config_profile
                .analytics
                .as_ref()
                .and_then(|a| a.enabled)
                .or(cfg.analytics.as_ref().and_then(|a| a.enabled)),
            feedback_enabled: cfg
                .feedback
                .as_ref()
                .and_then(|feedback| feedback.enabled)
                .unwrap_or(true),
            tool_suggest,
            tui_notifications: cfg
                .tui
                .as_ref()
                .map(|t| t.notification_settings.clone())
                .unwrap_or_default(),
            animations: cfg.tui.as_ref().map(|t| t.animations).unwrap_or(true),
            show_tooltips: cfg.tui.as_ref().map(|t| t.show_tooltips).unwrap_or(true),
            model_availability_nux: cfg
                .tui
                .as_ref()
                .map(|t| t.model_availability_nux.clone())
                .unwrap_or_default(),
            tui_vim_mode_default: cfg
                .tui
                .as_ref()
                .map(|t| t.vim_mode_default)
                .unwrap_or(false),
            tui_raw_output_mode: cfg
                .tui
                .as_ref()
                .map(|t| t.raw_output_mode)
                .unwrap_or(false),
            tui_alternate_screen: cfg
                .tui
                .as_ref()
                .map(|t| t.alternate_screen)
                .unwrap_or_default(),
            tui_status_line: cfg.tui.as_ref().and_then(|t| t.status_line.clone()),
            tui_status_line_use_colors: cfg
                .tui
                .as_ref()
                .map(|t| t.status_line_use_colors)
                .unwrap_or(true),
            tui_terminal_title: cfg.tui.as_ref().and_then(|t| t.terminal_title.clone()),
            tui_theme: cfg.tui.as_ref().and_then(|t| t.theme.clone()),
            tui_pet: cfg.tui.as_ref().and_then(|t| t.pet.clone()),
            tui_pet_anchor: cfg
                .tui
                .as_ref()
                .map(|t| t.pet_anchor)
                .unwrap_or_default(),
            tui_session_picker_view: config_profile
                .tui
                .as_ref()
                .and_then(|t| t.session_picker_view)
                .or_else(|| cfg.tui.as_ref().and_then(|t| t.session_picker_view))
                .unwrap_or_default(),
            terminal_resize_reflow,
            tui_keymap: cfg
                .tui
                .as_ref()
                .map(|t| t.keymap.clone())
                .unwrap_or_default(),
            otel,
        };
        Ok(config)
        })
        .await
    }

    /// If `path` is `Some`, attempts to read the file at the given path and
    /// returns its contents as a trimmed `String`. If the file is empty, or
    /// is `Some` but cannot be read, returns an `Err`.
    async fn try_read_non_empty_file(
        fs: &dyn ExecutorFileSystem,
        path: Option<&AbsolutePathBuf>,
        context: &str,
    ) -> std::io::Result<Option<String>> {
        let Some(path) = path else {
            return Ok(None);
        };

        let contents = fs
            .read_file_text(path, /*sandbox*/ None)
            .await
            .map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("failed to read {context} {}: {e}", path.display()),
                )
            })?;

        let s = contents.trim().to_string();
        if s.is_empty() {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{context} is empty: {}", path.display()),
            ))
        } else {
            Ok(Some(s))
        }
    }

    fn is_default_compact_prompt_fallback(err: &std::io::Error) -> bool {
        err.kind() == std::io::ErrorKind::NotFound
            || (err.kind() == std::io::ErrorKind::InvalidData
                && err.to_string().contains("is empty"))
    }

    pub fn set_windows_sandbox_enabled(&mut self, value: bool) {
        self.permissions.windows_sandbox_mode = if value {
            Some(WindowsSandboxModeToml::Unelevated)
        } else if matches!(
            self.permissions.windows_sandbox_mode,
            Some(WindowsSandboxModeToml::Unelevated)
        ) {
            None
        } else {
            self.permissions.windows_sandbox_mode
        };
    }

    pub fn set_windows_elevated_sandbox_enabled(&mut self, value: bool) {
        self.permissions.windows_sandbox_mode = if value {
            Some(WindowsSandboxModeToml::Elevated)
        } else if matches!(
            self.permissions.windows_sandbox_mode,
            Some(WindowsSandboxModeToml::Elevated)
        ) {
            None
        } else {
            self.permissions.windows_sandbox_mode
        };
    }

    pub fn managed_network_requirements_enabled(&self) -> bool {
        !matches!(
            self.permissions.permission_profile(),
            PermissionProfile::Disabled
        ) && self
            .config_layer_stack
            .requirements_toml()
            .network
            .is_some()
    }

    pub fn bundled_skills_enabled(&self) -> bool {
        skill_service_api::bundled_skills_enabled_from_stack(
            &skill_config_layer_stack_from_config_layer_stack(&self.config_layer_stack),
        )
    }
}
