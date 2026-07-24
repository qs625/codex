use crate::config_manager::ConfigManager;
use crate::live_thread_runtime::AppServerLiveThreadCommandRuntime;
use crate::live_thread_runtime::AppServerLiveThreadInspectionRuntime;
use codex_config_types::McpServerConfig;
use futures::future::BoxFuture;
use protocol::ThreadId;
use protocol::error::CodexErr;
use protocol::protocol::McpServerRefreshConfig;
use protocol::protocol::Op;
use std::collections::HashMap;
use std::io;
use thread_service::ThreadService;
use thread_service::config::Config;
use thread_service_api::LiveThreadConfigRefreshSnapshot;
use tracing::warn;

/// Runtime capability needed to plan and queue MCP refreshes for live threads.
///
/// Implementations own MCP server planning. Live thread inspection and command
/// submission are supplied by the provider-neutral runtime surfaces.
pub(crate) trait McpRefreshRuntime: Send + Sync {
    fn configured_mcp_servers<'a>(
        &'a self,
        config: &'a Config,
    ) -> BoxFuture<'a, HashMap<String, McpServerConfig>>;
}

impl McpRefreshRuntime for ThreadService {
    fn configured_mcp_servers<'a>(
        &'a self,
        config: &'a Config,
    ) -> BoxFuture<'a, HashMap<String, McpServerConfig>> {
        Box::pin(async move {
            self.mcp_service()
                .configured_servers(self.plugin_runtime().as_ref(), config)
                .await
        })
    }
}

pub(crate) async fn queue_strict_refresh<R>(
    runtime: &R,
    config_manager: &ConfigManager,
) -> io::Result<()>
where
    R: McpRefreshRuntime
        + AppServerLiveThreadInspectionRuntime
        + AppServerLiveThreadCommandRuntime
        + ?Sized,
{
    config_manager
        .load_latest_config(/*fallback_cwd*/ None)
        .await?;
    let mut refreshes = Vec::new();
    for thread_id in runtime.list_live_thread_ids().await {
        let refresh_snapshot = live_thread_config_refresh_snapshot(runtime, thread_id).await?;
        let config = build_refresh_config(runtime, config_manager, &refresh_snapshot).await?;
        refreshes.push((thread_id, config));
    }
    for (thread_id, config) in refreshes {
        queue_refresh(runtime, thread_id, config).await?;
    }
    Ok(())
}

pub(crate) async fn queue_best_effort_refresh<R>(runtime: &R, config_manager: &ConfigManager)
where
    R: McpRefreshRuntime
        + AppServerLiveThreadInspectionRuntime
        + AppServerLiveThreadCommandRuntime
        + ?Sized,
{
    for thread_id in runtime.list_live_thread_ids().await {
        let refresh_snapshot = match live_thread_config_refresh_snapshot(runtime, thread_id).await {
            Ok(refresh_snapshot) => refresh_snapshot,
            Err(err) => {
                warn!("failed to load thread {thread_id} for MCP refresh: {err}");
                continue;
            }
        };
        let config = match build_refresh_config(runtime, config_manager, &refresh_snapshot).await {
            Ok(config) => config,
            Err(err) => {
                warn!("failed to build MCP refresh config for thread {thread_id}: {err}");
                continue;
            }
        };
        if let Err(err) = queue_refresh(runtime, thread_id, config).await {
            warn!("{err}");
        }
    }
}

async fn live_thread_config_refresh_snapshot(
    runtime: &(impl AppServerLiveThreadInspectionRuntime + ?Sized),
    thread_id: ThreadId,
) -> io::Result<LiveThreadConfigRefreshSnapshot> {
    runtime
        .live_thread_config_refresh_snapshot(thread_id)
        .await
        .map_err(|err| io::Error::other(format!("failed to load thread {thread_id}: {err}")))
}

async fn build_refresh_config(
    runtime: &(impl McpRefreshRuntime + ?Sized),
    config_manager: &ConfigManager,
    refresh_snapshot: &LiveThreadConfigRefreshSnapshot,
) -> io::Result<McpServerRefreshConfig> {
    let config = config_manager
        .load_latest_config_for_thread_refresh_snapshot(refresh_snapshot)
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

async fn queue_refresh(
    runtime: &(impl AppServerLiveThreadCommandRuntime + ?Sized),
    thread_id: ThreadId,
    config: McpServerRefreshConfig,
) -> io::Result<()> {
    match runtime
        .submit_live_thread_op(thread_id, Op::RefreshMcpServers { config })
        .await
    {
        Ok(_) | Err(CodexErr::ThreadNotFound(_)) => Ok(()),
        Err(err) => Err(io::Error::other(format!(
            "failed to queue MCP refresh for thread {thread_id}: {err}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::FileSubscriptionThreadHost;
    use crate::extensions::thread_extensions;
    use codex_arg0::Arg0DispatchPaths;
    use codex_exec_server::EnvironmentManager;
    use codex_file_watcher::FileWatcher;
    use codex_login::AuthManager;
    use codex_login::CodexAuth;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use config_service::CloudRequirementsLoader;
    use config_service::LoaderOverrides;
    use config_service::ThreadConfigContext;
    use config_service::ThreadConfigLoadError;
    use config_service::ThreadConfigLoadErrorCode;
    use config_service::ThreadConfigLoader;
    use config_service::ThreadConfigSource;
    use pretty_assertions::assert_eq;
    use protocol::protocol::SessionSource;
    use protocol::error::Result as CodexResult;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::Weak;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use tempfile::TempDir;
    use thread_service::config::ConfigOverrides;

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

    #[tokio::test]
    async fn strict_refresh_skips_threads_that_cannot_accept_refresh_op() -> anyhow::Result<()> {
        let temp_dir = TempDir::new()?;
        let refresh_cwd = temp_dir.path().join("refresh");
        let unused_bad_cwd = temp_dir.path().join("bad");
        std::fs::create_dir_all(&refresh_cwd)?;
        std::fs::create_dir_all(&unused_bad_cwd)?;
        let loader = Arc::new(CountingThreadConfigLoader {
            good_cwd: AbsolutePathBuf::try_from(refresh_cwd.clone())?,
            bad_cwd: AbsolutePathBuf::try_from(unused_bad_cwd)?,
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
            loader,
        );
        let native_thread_id = ThreadId::new();
        let external_thread_id = ThreadId::new();
        let refresh_snapshot = LiveThreadConfigRefreshSnapshot {
            cwd: AbsolutePathBuf::try_from(refresh_cwd)?,
            session_layers: Vec::new(),
        };
        let runtime = FakeRefreshRuntime {
            live_thread_ids: vec![native_thread_id, external_thread_id],
            refresh_snapshots: HashMap::from([
                (native_thread_id, refresh_snapshot.clone()),
                (external_thread_id, refresh_snapshot),
            ]),
            rejected_thread_id: external_thread_id,
            submitted_thread_ids: Mutex::new(Vec::new()),
        };

        queue_strict_refresh(&runtime, &config_manager).await?;

        assert_eq!(runtime.submitted_thread_ids(), vec![native_thread_id]);
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
        let state_db = rollout::state_db::init(&good_config)
            .await
            .expect("refresh tests require state db");
        let thread_store = crate::thread_store_factory::thread_store_from_config(
            &good_config,
            Some(state_db.clone()),
        );
        let thread_watch_manager = crate::thread_status::ThreadWatchManager::new();
        let thread_service: Arc<ThreadService> =
            Arc::new_cyclic(|thread_service: &Weak<ThreadService>| {
                let thread_service_api: Weak<dyn thread_service_api::ThreadServiceApi> =
                    thread_service.clone();
                let workflow_thread_runtime: Weak<dyn codex_workflow::WorkflowThreadRuntime> =
                    thread_service.clone();
                let workflow_service = Arc::new(codex_workflow::WorkflowService::new(
                    good_config.codex_home.clone(),
                    workflow_thread_runtime,
                ));
                let approval_service = Arc::new(approval_service::ApprovalService);
                let mcp_service = Arc::new(mcp_service::McpService::new(approval_service.clone()));
                let tool_service = Arc::new(codex_tool_service::ToolService::new(
                    approval_service,
                    Arc::new(command_service::CommandService::new()),
                    Arc::new(goal_service::GoalService),
                    mcp_service.clone(),
                    Arc::new(permissions_service::PermissionsService),
                    workflow_service,
                    thread_service_api,
                ));
                let auth_runtimes = thread_service::ThreadAuthRuntimes::from_auth_runtime(
                    auth_manager.clone(),
                    codex_login::model_provider_auth_manager(Some(auth_manager.clone())),
                );
                let file_subscription_host: Weak<dyn FileSubscriptionThreadHost> =
                    Weak::<ThreadService>::clone(thread_service);
                ThreadService::new_with_openai_file_uploader(
                    &good_config,
                    auth_runtimes,
                    SessionSource::Exec,
                    Arc::new(EnvironmentManager::default_for_tests()),
                    thread_extensions(
                        Arc::new(FileWatcher::noop()),
                        file_subscription_host,
                        thread_watch_manager.clone(),
                    ),
                    /*analytics_events_client*/ None,
                    thread_store,
                    Some(state_db.clone()),
                    Arc::new(thread_store::DefaultLiveThreadFactory),
                    "11111111-1111-4111-8111-111111111111".to_string(),
                    /*attestation_provider*/ None,
                    Arc::new(model_service::DefaultModelProviderFactory),
                    Arc::new(codex_code_mode::V8CodeModeRuntimeFactory),
                    Arc::new(command_service::CommandService::new()),
                    Arc::new(approval_service::ApprovalService),
                    Arc::new(goal_service::GoalService),
                    Arc::new(mcp_service::DefaultMcpAuthRuntime),
                    Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
                    Arc::new(codex_openai_files::ReqwestOpenAiFileUploader),
                    Arc::new(permissions_service::StarlarkExecPolicyLoader),
                    Arc::new(model_service::DefaultApiRuntimeFactory),
                    Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
                    Arc::new(codex_sandboxing::SandboxManager::new()),
                    Arc::new(codex_otel::OtelSessionTelemetryFactory),
                    Arc::new(hooks::HooksRuntimeFactory),
                    Arc::new(memory_service::FsMemoryToolDeveloperInstructionsProvider),
                    Arc::new(skill_service::SkillService::new_with_restriction_product(
                        good_config.codex_home.clone(),
                        good_config.bundled_skills_enabled(),
                        SessionSource::Exec.restriction_product(),
                    )),
                    Arc::new(
                        plugin_service::PluginsManager::new_with_restriction_product(
                            good_config.codex_home.to_path_buf(),
                            SessionSource::Exec.restriction_product(),
                        ),
                    ),
                    tool_service,
                    mcp_service,
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

    struct FakeRefreshRuntime {
        live_thread_ids: Vec<ThreadId>,
        refresh_snapshots: HashMap<ThreadId, LiveThreadConfigRefreshSnapshot>,
        rejected_thread_id: ThreadId,
        submitted_thread_ids: Mutex<Vec<ThreadId>>,
    }

    impl FakeRefreshRuntime {
        fn submitted_thread_ids(&self) -> Vec<ThreadId> {
            self.submitted_thread_ids
                .lock()
                .expect("submitted thread ids")
                .clone()
        }
    }

    impl McpRefreshRuntime for FakeRefreshRuntime {
        fn configured_mcp_servers<'a>(
            &'a self,
            _config: &'a Config,
        ) -> BoxFuture<'a, HashMap<String, McpServerConfig>> {
            Box::pin(async { HashMap::new() })
        }
    }

    impl AppServerLiveThreadInspectionRuntime for FakeRefreshRuntime {
        fn list_live_thread_ids(&self) -> BoxFuture<'_, Vec<ThreadId>> {
            Box::pin(async { self.live_thread_ids.clone() })
        }

        fn is_live_thread_loaded(&self, thread_id: ThreadId) -> BoxFuture<'_, bool> {
            Box::pin(async move { self.live_thread_ids.contains(&thread_id) })
        }

        fn live_thread_info(
            &self,
            thread_id: ThreadId,
        ) -> BoxFuture<'_, CodexResult<thread_service_api::LiveThreadInfo>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }

        fn live_thread_snapshot(
            &self,
            thread_id: ThreadId,
        ) -> BoxFuture<'_, CodexResult<thread_service_api::LiveThreadSnapshot>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }

        fn live_thread_config_snapshot(
            &self,
            thread_id: ThreadId,
        ) -> BoxFuture<'_, CodexResult<thread_service_api::ThreadConfigSnapshot>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }

        fn live_thread_config_refresh_snapshot(
            &self,
            thread_id: ThreadId,
        ) -> BoxFuture<'_, CodexResult<LiveThreadConfigRefreshSnapshot>> {
            Box::pin(async move {
                self.refresh_snapshots
                    .get(&thread_id)
                    .cloned()
                    .ok_or(CodexErr::ThreadNotFound(thread_id))
            })
        }

        fn live_thread_feature_enabled(
            &self,
            thread_id: ThreadId,
            _feature: codex_features::Feature,
        ) -> BoxFuture<'_, CodexResult<bool>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }
    }

    impl AppServerLiveThreadCommandRuntime for FakeRefreshRuntime {
        fn submit_live_thread_op(
            &self,
            thread_id: ThreadId,
            _op: Op,
        ) -> BoxFuture<'_, CodexResult<String>> {
            Box::pin(async move {
                if thread_id == self.rejected_thread_id {
                    Err(CodexErr::ThreadNotFound(thread_id))
                } else {
                    self.submitted_thread_ids
                        .lock()
                        .expect("submitted thread ids")
                        .push(thread_id);
                    Ok("queued".to_string())
                }
            })
        }

        fn submit_live_thread_op_with_trace(
            &self,
            thread_id: ThreadId,
            _op: Op,
            _trace: Option<protocol::protocol::W3cTraceContext>,
        ) -> BoxFuture<'_, CodexResult<String>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }

        fn set_live_thread_app_server_client_info(
            &self,
            thread_id: ThreadId,
            _info: thread_service_api::AppServerClientInfo,
        ) -> BoxFuture<'_, CodexResult<()>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }
    }
}
