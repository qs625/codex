use crate::config_manager::ConfigManager;
use codex_config_types::McpServerConfig;
use codex_protocol::ThreadId;
use codex_protocol::protocol::McpServerRefreshConfig;
use codex_protocol::protocol::Op;
use thread_service_api::LiveThreadRegistry;
use thread_service::ThreadService;
use thread_service::config::Config;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use tracing::warn;

/// Runtime capability needed to plan and queue MCP refreshes for live threads.
///
/// Implementations own concrete thread lookup, config snapshots, MCP server
/// planning, and op submission. The refresh helper stays generic so app-server
/// request processors do not need to depend on a concrete `ThreadService`.
pub(crate) trait McpRefreshRuntime: Send + Sync {
    fn list_thread_ids(&self) -> BoxFuture<'_, Vec<ThreadId>>;

    fn live_thread_config(&self, thread_id: ThreadId) -> BoxFuture<'_, io::Result<Arc<Config>>>;

    fn configured_mcp_servers<'a>(
        &'a self,
        config: &'a Config,
    ) -> BoxFuture<'a, HashMap<String, McpServerConfig>>;

    fn queue_mcp_refresh(
        &self,
        thread_id: ThreadId,
        config: McpServerRefreshConfig,
    ) -> BoxFuture<'_, io::Result<()>>;
}

impl McpRefreshRuntime for ThreadService {
    fn list_thread_ids(&self) -> BoxFuture<'_, Vec<ThreadId>> {
        Box::pin(ThreadService::list_thread_ids(self))
    }

    fn live_thread_config(&self, thread_id: ThreadId) -> BoxFuture<'_, io::Result<Arc<Config>>> {
        Box::pin(async move {
            ThreadService::live_thread_config(self, thread_id)
                .await
                .map_err(|err| {
                    io::Error::other(format!("failed to load thread {thread_id}: {err}"))
                })
        })
    }

    fn configured_mcp_servers<'a>(
        &'a self,
        config: &'a Config,
    ) -> BoxFuture<'a, HashMap<String, McpServerConfig>> {
        Box::pin(async move { self.mcp_manager().configured_servers(config).await })
    }

    fn queue_mcp_refresh(
        &self,
        thread_id: ThreadId,
        config: McpServerRefreshConfig,
    ) -> BoxFuture<'_, io::Result<()>> {
        Box::pin(queue_refresh(thread_id, self, config))
    }
}

pub(crate) async fn queue_strict_refresh(
    runtime: &(impl McpRefreshRuntime + ?Sized),
    config_manager: &ConfigManager,
) -> io::Result<()> {
    config_manager
        .load_latest_config(/*fallback_cwd*/ None)
        .await?;
    let mut refreshes = Vec::new();
    for thread_id in runtime.list_thread_ids().await {
        let thread_config = runtime.live_thread_config(thread_id).await?;
        let config = build_refresh_config(runtime, config_manager, thread_config).await?;
        refreshes.push((thread_id, config));
    }
    for (thread_id, config) in refreshes {
        runtime.queue_mcp_refresh(thread_id, config).await?;
    }
    Ok(())
}

pub(crate) async fn queue_best_effort_refresh(
    runtime: &(impl McpRefreshRuntime + ?Sized),
    config_manager: &ConfigManager,
) {
    for thread_id in runtime.list_thread_ids().await {
        let thread_config = match runtime.live_thread_config(thread_id).await {
            Ok(thread_config) => thread_config,
            Err(err) => {
                warn!("failed to load thread {thread_id} for MCP refresh: {err}");
                continue;
            }
        };
        let config = match build_refresh_config(runtime, config_manager, thread_config).await {
            Ok(config) => config,
            Err(err) => {
                warn!("failed to build MCP refresh config for thread {thread_id}: {err}");
                continue;
            }
        };
        if let Err(err) = runtime.queue_mcp_refresh(thread_id, config).await {
            warn!("{err}");
        }
    }
}

async fn build_refresh_config(
    runtime: &(impl McpRefreshRuntime + ?Sized),
    config_manager: &ConfigManager,
    thread_config: Arc<Config>,
) -> io::Result<McpServerRefreshConfig> {
    let config = config_manager
        .load_latest_config_for_thread(thread_config.as_ref())
        .await?;
    let mcp_servers = runtime.configured_mcp_servers(&config).await;
    Ok(McpServerRefreshConfig {
        mcp_servers: serde_json::to_value(mcp_servers).map_err(io::Error::other)?,
        mcp_oauth_credentials_store_mode: serde_json::to_value(
            config.mcp_oauth_credentials_store_mode,
        )
        .map_err(io::Error::other)?,
    })
}

async fn queue_refresh<H>(
    thread_id: ThreadId,
    thread_registry: &H,
    config: McpServerRefreshConfig,
) -> io::Result<()>
where
    H: LiveThreadRegistry + ?Sized,
{
    thread_registry
        .send_op(thread_id, Op::RefreshMcpServers { config })
        .await
        .map(|_| ())
        .map_err(|err| {
            io::Error::other(format!(
                "failed to queue MCP refresh for thread {thread_id}: {err}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::FileSubscriptionThreadHost;
    use crate::extensions::GuardianAgentSpawnHost;
    use crate::extensions::guardian_agent_spawner;
    use crate::extensions::thread_extensions;
    use codex_arg0::Arg0DispatchPaths;
    use codex_config_loader::LoaderOverrides;
    use codex_config_loader::ThreadConfigContext;
    use codex_config_loader::ThreadConfigLoadError;
    use codex_config_loader::ThreadConfigLoadErrorCode;
    use codex_config_loader::ThreadConfigLoader;
    use codex_config_loader::ThreadConfigSource;
    use codex_config_requirements::CloudRequirementsLoader;
    use codex_exec_server::EnvironmentManager;
    use codex_file_watcher::FileWatcher;
    use codex_login::AuthManager;
    use codex_login::CodexAuth;
    use codex_protocol::protocol::SessionSource;
    use thread_service::config::ConfigOverrides;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Weak;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use tempfile::TempDir;

    #[tokio::test]
    async fn strict_refresh_reports_thread_planning_failures() -> anyhow::Result<()> {
        let (_temp_dir, thread_service, config_manager, _loader) = refresh_test_state().await?;

        let err = queue_strict_refresh(thread_service.as_ref(), &config_manager)
            .await
            .expect_err("strict refresh should fail");

        assert_eq!(err.to_string(), "failed to load refresh config");
        Ok(())
    }

    #[tokio::test]
    async fn best_effort_refresh_attempts_every_loaded_thread() -> anyhow::Result<()> {
        let (_temp_dir, thread_service, config_manager, loader) = refresh_test_state().await?;

        queue_best_effort_refresh(thread_service.as_ref(), &config_manager).await;

        assert_eq!(loader.good_loads.load(Ordering::Relaxed), 1);
        assert_eq!(loader.bad_loads.load(Ordering::Relaxed), 1);
        Ok(())
    }

    async fn refresh_test_state() -> anyhow::Result<(
        TempDir,
        Arc<ThreadService>,
        ConfigManager,
        Arc<CountingThreadConfigLoader>,
    )> {
        let temp_dir = TempDir::new()?;
        let good_cwd = temp_dir.path().join("good");
        let bad_cwd = temp_dir.path().join("bad");
        std::fs::create_dir_all(&good_cwd)?;
        std::fs::create_dir_all(&bad_cwd)?;

        let initial_config_manager =
            ConfigManager::without_managed_config_for_tests(temp_dir.path().to_path_buf());
        let good_config = initial_config_manager
            .load_for_cwd(
                /*request_overrides*/ None,
                ConfigOverrides::default(),
                Some(good_cwd.clone()),
            )
            .await?;
        let bad_config = initial_config_manager
            .load_for_cwd(
                /*request_overrides*/ None,
                ConfigOverrides::default(),
                Some(bad_cwd.clone()),
            )
            .await?;

        let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("dummy"));
        let state_db = codex_rollout::state_db::init(&good_config)
            .await
            .expect("refresh tests require state db");
        let shared_state_db = Some(state_db.clone());
        let thread_store = crate::thread_store_factory::thread_store_from_config(
            &good_config,
            Some(state_db.clone()),
        );
        let thread_watch_manager = crate::thread_status::ThreadWatchManager::new();
        let thread_service: Arc<ThreadService> =
            Arc::new_cyclic(|thread_service: &Weak<ThreadService>| {
                let thread_service_api: Weak<dyn thread_service_api::ThreadServiceApi> =
                    thread_service.clone();
                let workflow_service = Arc::new(codex_workflow::WorkflowService::new(
                    good_config.codex_home.clone(),
                    thread_service_api.clone(),
                ));
                let approval_service = Arc::new(approval_service::ApprovalService);
                let mcp_service =
                    Arc::new(mcp_service::McpService::new(approval_service.clone()));
                let tool_service = Arc::new(codex_tool_service::ToolService::new(
                    approval_service,
                    Arc::new(codex_command_service::CommandService::new()),
                    Arc::new(goal_service::GoalService),
                    mcp_service.clone(),
                    Arc::new(thread_service::RequestPluginInstallService),
                    workflow_service,
                    thread_service_api,
                ));
                let auth_runtimes = thread_service::ThreadAuthRuntimes::from_auth_runtime(
                    auth_manager.clone(),
                    codex_login::model_provider_auth_manager(Some(auth_manager.clone())),
                );
                let guardian_agent_host: Weak<dyn GuardianAgentSpawnHost> = thread_service.clone();
                let file_subscription_host: Weak<dyn FileSubscriptionThreadHost> =
                    Weak::<ThreadService>::clone(thread_service);
                ThreadService::new_with_openai_file_uploader(
                    &good_config,
                    auth_runtimes,
                    SessionSource::Exec,
                    Arc::new(EnvironmentManager::default_for_tests()),
                    thread_extensions(
                        guardian_agent_spawner(guardian_agent_host),
                        Arc::new(FileWatcher::noop()),
                        file_subscription_host,
                        thread_watch_manager.clone(),
                    ),
                    /*analytics_events_client*/ None,
                    thread_store,
                    state_db.clone(),
                    Arc::new(codex_thread_store::DefaultLiveThreadFactory),
                    "11111111-1111-4111-8111-111111111111".to_string(),
                    /*attestation_provider*/ None,
                    Arc::new(codex_model_provider::DefaultModelProviderFactory),
                    Arc::new(codex_code_mode::V8CodeModeRuntimeFactory),
                    Arc::new(goal_service::GoalService),
                    Arc::new(codex_mcp::DefaultMcpAuthRuntime),
                    Arc::new(codex_mcp::DefaultMcpConnectionRuntimeFactory),
                    Arc::new(codex_openai_files::ReqwestOpenAiFileUploader),
                    Arc::new(codex_execpolicy_loader::StarlarkExecPolicyLoader),
                    Arc::new(codex_api::DefaultApiRuntimeFactory),
                    Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
                    Arc::new(codex_sandboxing::SandboxManager::new()),
                    Arc::new(codex_otel::OtelSessionTelemetryFactory),
                    Arc::new(codex_hooks::HooksRuntimeFactory),
                    Arc::new(codex_memories_read::FsMemoryToolDeveloperInstructionsProvider),
                    Arc::new(
                        codex_core_skills::SkillsManager::new_with_restriction_product(
                            good_config.codex_home.clone(),
                            good_config.bundled_skills_enabled(),
                            SessionSource::Exec.restriction_product(),
                        ),
                    ),
                    Arc::new(
                        codex_core_plugins::PluginsManager::new_with_restriction_product(
                            good_config.codex_home.to_path_buf(),
                            SessionSource::Exec.restriction_product(),
                        ),
                    ),
                    tool_service.clone(),
                    mcp_service.clone(),
                )
            });
        thread_service.start_thread(good_config).await?;
        thread_service.start_thread(bad_config).await?;

        let loader = Arc::new(CountingThreadConfigLoader {
            good_cwd: AbsolutePathBuf::try_from(good_cwd)?,
            bad_cwd: AbsolutePathBuf::try_from(bad_cwd)?,
            good_loads: AtomicUsize::new(0),
            bad_loads: AtomicUsize::new(0),
        });
        let config_manager = ConfigManager::new(
            temp_dir.path().to_path_buf(),
            Vec::new(),
            LoaderOverrides::without_managed_config_for_tests(),
            /*strict_config*/ false,
            CloudRequirementsLoader::default(),
            Arg0DispatchPaths::default(),
            loader.clone(),
        );

        Ok((temp_dir, thread_service, config_manager, loader))
    }

    struct CountingThreadConfigLoader {
        good_cwd: AbsolutePathBuf,
        bad_cwd: AbsolutePathBuf,
        good_loads: AtomicUsize,
        bad_loads: AtomicUsize,
    }

    impl ThreadConfigLoader for CountingThreadConfigLoader {
        fn load(
            &self,
            context: ThreadConfigContext,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<Vec<ThreadConfigSource>, ThreadConfigLoadError>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                if context.cwd.as_ref() == Some(&self.good_cwd) {
                    self.good_loads.fetch_add(1, Ordering::Relaxed);
                }
                if context.cwd.as_ref() == Some(&self.bad_cwd) {
                    self.bad_loads.fetch_add(1, Ordering::Relaxed);
                    return Err(ThreadConfigLoadError::new(
                        ThreadConfigLoadErrorCode::Internal,
                        /*status_code*/ None,
                        "failed to load refresh config",
                    ));
                }
                Ok(Vec::new())
            })
        }
    }
}
