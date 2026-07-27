use std::sync::Arc;
use std::time::Duration;

use crate::agent::external::ExternalSpawnConfig;
use crate::agent::multi_agent;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::thread::ThreadService;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::SpawnAgentProvider;
use protocol::error::Result as CodexResult;
use protocol::models::ResponseItem;
use thread_service_api::AgentDirectoryListRequest;
use thread_service_api::AgentDirectoryListResult;
use thread_service_api::AgentReferenceResolution;
use thread_service_api::AgentReferenceResolutionRequest;
use thread_service_api::ExternalRootThreadInputRoute;
use thread_service_api::ExternalRootThreadProvider;
use thread_service_api::ExternalRootThreadRuntime;
use thread_service_api::ExternalRootThreadStartRequest;
use thread_service_api::ExternalRootThreadStartResult;
use thread_service_api::ExternalRootThreadStartupConfig;
use thread_service_api::LiveExternalRootThreadFacts;
use thread_service_api::NativeAgentRuntime;
use thread_service_api::NativeTurnEventRuntime;
use thread_service_api::PersistedExternalRootThreadFacts;
use thread_service_api::PersistedThreadProviderFactsRuntime;
use thread_service_api::PersistedThreadProviderFactsSelector;
use thread_service_api::RootThreadProviderResolutionError;
use thread_service_api::RootThreadProviderRoute;
use thread_service_api::ThreadAgentDirectoryRuntime;
use thread_service_api::ThreadCloseAgentResult;
use thread_service_api::ThreadCollaborationRuntime;
use thread_service_api::ThreadCreatedEvent;
use thread_service_api::ThreadLifecycleRuntime;
use thread_service_api::ThreadListAgentsResult;
use thread_service_api::ThreadListedAgent;
use thread_service_api::ThreadPollEventRequest;
use thread_service_api::ThreadPollEventResult;
use thread_service_api::ThreadPollEventTimeoutMetadata;
use thread_service_api::ThreadProviderCatalogRuntime;
use thread_service_api::ThreadProviderRootCapability;
use thread_service_api::ThreadProviderRuntimeCapabilities;
use thread_service_api::ThreadProviderRuntimeDescriptor;
use thread_service_api::ThreadProviderRuntimeKind;
use thread_service_api::ThreadServiceFuture;
use thread_service_api::ThreadShutdownReport;
use thread_service_api::ThreadSpawnAgentForkMode;
use thread_service_api::ThreadSpawnAgentRequest;
use thread_service_api::ThreadSpawnAgentResult;
use thread_service_api::ThreadSpawnExternalAgentRequest;
use thread_service_api::ThreadTurnCapability;
use tokio::sync::broadcast;
use tool_service_api::FunctionCallError;

fn turn_context(
    turn: Arc<dyn ThreadTurnCapability>,
) -> Result<Arc<TurnContext>, FunctionCallError> {
    turn.into_any_arc().downcast::<TurnContext>().map_err(|_| {
        FunctionCallError::Fatal("thread turn capability must be TurnContext".to_string())
    })
}

fn session(turn: &TurnContext) -> Arc<Session> {
    turn.session_arc()
}

fn to_runtime_spawn_request(
    request: ThreadSpawnAgentRequest,
) -> codex_agent_runtime::SpawnAgentToolRequest {
    codex_agent_runtime::SpawnAgentToolRequest {
        message: request.message,
        task_name: request.task_name,
        provider: request.provider.map(|provider| match provider {
            thread_service_api::ThreadSpawnAgentProvider::Native => SpawnAgentProvider::Native,
            thread_service_api::ThreadSpawnAgentProvider::CodexCli => SpawnAgentProvider::CodexCli,
            thread_service_api::ThreadSpawnAgentProvider::ClaudeCli => {
                SpawnAgentProvider::ClaudeCli
            }
            thread_service_api::ThreadSpawnAgentProvider::Opencode => SpawnAgentProvider::Opencode,
        }),
        agent_type: request.agent_type,
        cwd: request.cwd,
        model: request.model,
        reasoning_effort: request.reasoning_effort,
        service_tier: request.service_tier,
        fork_mode: request.fork_mode.map(|mode| match mode {
            ThreadSpawnAgentForkMode::FullHistory => SpawnAgentForkMode::FullHistory,
            ThreadSpawnAgentForkMode::LastNTurns { last_n_turns } => {
                SpawnAgentForkMode::LastNTurns(last_n_turns)
            }
        }),
    }
}

fn to_runtime_spawn_external_request(
    request: ThreadSpawnExternalAgentRequest,
) -> codex_agent_runtime::SpawnExternalAgentToolRequest {
    codex_agent_runtime::SpawnExternalAgentToolRequest {
        message: request.message,
        task_name: request.task_name,
        provider: match request.provider {
            thread_service_api::ThreadSpawnAgentProvider::Native => SpawnAgentProvider::Native,
            thread_service_api::ThreadSpawnAgentProvider::CodexCli => SpawnAgentProvider::CodexCli,
            thread_service_api::ThreadSpawnAgentProvider::ClaudeCli => {
                SpawnAgentProvider::ClaudeCli
            }
            thread_service_api::ThreadSpawnAgentProvider::Opencode => SpawnAgentProvider::Opencode,
        },
        cwd: request.cwd,
    }
}

fn to_runtime_external_root_provider(provider: ExternalRootThreadProvider) -> SpawnAgentProvider {
    match provider {
        ExternalRootThreadProvider::CodexCli => SpawnAgentProvider::CodexCli,
        ExternalRootThreadProvider::ClaudeCli => SpawnAgentProvider::ClaudeCli,
        ExternalRootThreadProvider::Opencode => SpawnAgentProvider::Opencode,
    }
}

fn to_runtime_external_root_agent_metadata(
    metadata: thread_service_api::ExternalRootAgentMetadata,
) -> codex_agent_runtime::AgentMetadata {
    codex_agent_runtime::AgentMetadata {
        agent_path: Some(metadata.agent_path),
        agent_nickname: metadata.agent_nickname,
        agent_role: metadata.agent_role,
        ..Default::default()
    }
}

const NATIVE_THREAD_PROVIDER_CAPABILITIES: ThreadProviderRuntimeCapabilities =
    ThreadProviderRuntimeCapabilities {
        start_thread: true,
        send_input: true,
        close_thread: true,
        list_children: true,
        restore_thread: true,
        restore_snapshot: true,
        event_stream: true,
        spawn_child: true,
        compact: true,
        workflow: true,
        poll_event: true,
        command_session: true,
        permissions: true,
        dynamic_tools: true,
        fork_thread: true,
    };

const EXTERNAL_CLI_THREAD_PROVIDER_CAPABILITIES: ThreadProviderRuntimeCapabilities =
    ThreadProviderRuntimeCapabilities {
        start_thread: true,
        send_input: true,
        close_thread: true,
        list_children: true,
        restore_thread: false,
        restore_snapshot: true,
        event_stream: true,
        spawn_child: true,
        compact: false,
        workflow: false,
        poll_event: true,
        command_session: false,
        permissions: false,
        dynamic_tools: false,
        fork_thread: false,
    };

fn external_thread_provider_descriptor(
    provider: ExternalRootThreadProvider,
    display_name: &'static str,
    description: &'static str,
) -> ThreadProviderRuntimeDescriptor {
    ThreadProviderRuntimeDescriptor {
        id: provider.provider_id().to_string(),
        display_name: display_name.to_string(),
        description: description.to_string(),
        kind: ThreadProviderRuntimeKind::ExternalCli,
        external_root_provider: Some(provider),
        capabilities: EXTERNAL_CLI_THREAD_PROVIDER_CAPABILITIES,
    }
}

fn runtime_thread_provider_descriptors() -> Vec<ThreadProviderRuntimeDescriptor> {
    vec![
        ThreadProviderRuntimeDescriptor {
            id: "native".to_string(),
            display_name: "Morpheus".to_string(),
            description: "Native Morpheus runtime with agent roles, model catalog, tools, compact, workflow, and unified EventMsg replay.".to_string(),
            kind: ThreadProviderRuntimeKind::Native,
            external_root_provider: None,
            capabilities: NATIVE_THREAD_PROVIDER_CAPABILITIES,
        },
        external_thread_provider_descriptor(
            ExternalRootThreadProvider::ClaudeCli,
            "Claude Code",
            "External Claude CLI session normalized by the external-agent adapter.",
        ),
        external_thread_provider_descriptor(
            ExternalRootThreadProvider::Opencode,
            "OpenCode",
            "External OpenCode session normalized by the external-agent adapter.",
        ),
        external_thread_provider_descriptor(
            ExternalRootThreadProvider::CodexCli,
            "Codex CLI",
            "External official Codex CLI app-server session normalized by the external-agent adapter.",
        ),
    ]
}

fn resolve_runtime_root_thread_provider(
    provider_id: Option<&str>,
    capability: ThreadProviderRootCapability,
) -> Result<RootThreadProviderRoute, RootThreadProviderResolutionError> {
    let Some(provider_id) = provider_id else {
        return Ok(RootThreadProviderRoute::Native);
    };
    let Some(descriptor) = runtime_thread_provider_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.id == provider_id)
    else {
        return Err(RootThreadProviderResolutionError::UnknownProvider {
            provider_id: provider_id.to_string(),
            capability,
        });
    };
    if !descriptor.capabilities.supports_root_capability(capability) {
        return Err(RootThreadProviderResolutionError::UnsupportedCapability {
            provider_id: descriptor.id,
            capability,
        });
    }
    match descriptor.kind {
        ThreadProviderRuntimeKind::Native => Ok(RootThreadProviderRoute::Native),
        ThreadProviderRuntimeKind::ExternalCli => Ok(RootThreadProviderRoute::External(
            descriptor
                .external_root_provider
                .expect("external CLI descriptor must include root provider"),
        )),
    }
}

fn to_external_spawn_config(config: ExternalRootThreadStartupConfig) -> ExternalSpawnConfig {
    ExternalSpawnConfig {
        cwd: config.cwd,
        workspace_roots: config.workspace_roots,
        agent_max_threads: config.agent_max_threads,
        agent_roles: config.agent_roles,
        model: config.model,
        model_provider_id: config.model_provider_id,
        service_tier: config.service_tier,
        approval_policy: config.approval_policy,
        approvals_reviewer: config.approvals_reviewer,
        permission_profile: config.permission_profile,
        active_permission_profile: config.active_permission_profile,
        reasoning_effort: config.reasoning_effort,
        personality: config.personality,
        features: config.features,
        generate_memories: config.generate_memories,
        default_wait_timeout_ms: config.default_wait_timeout_ms,
        max_wait_timeout_ms: config.max_wait_timeout_ms,
    }
}

fn from_runtime_spawn_result(
    result: codex_agent_runtime::SpawnAgentToolResult,
) -> ThreadSpawnAgentResult {
    match result {
        codex_agent_runtime::SpawnAgentToolResult::WithNickname {
            task_name,
            nickname,
        } => ThreadSpawnAgentResult::WithNickname {
            task_name,
            nickname,
        },
        codex_agent_runtime::SpawnAgentToolResult::HiddenMetadata { task_name } => {
            ThreadSpawnAgentResult::HiddenMetadata { task_name }
        }
    }
}

fn from_runtime_close_result(
    result: codex_agent_runtime::CloseAgentToolResult,
) -> ThreadCloseAgentResult {
    ThreadCloseAgentResult {
        previous_status: result.previous_status,
    }
}

fn from_runtime_list_result(
    result: codex_agent_runtime::ListAgentsToolResult,
) -> ThreadListAgentsResult {
    ThreadListAgentsResult {
        agents: result
            .agents
            .into_iter()
            .map(|agent| ThreadListedAgent {
                agent_name: agent.agent_name,
                agent_nickname: agent.agent_nickname,
                agent_role: agent.agent_role,
                lifecycle_status: agent.lifecycle_status,
                last_task_message: agent.last_task_message,
            })
            .collect(),
    }
}

impl ThreadLifecycleRuntime for ThreadService {
    fn shutdown_all_threads_bounded<'a>(
        &'a self,
        timeout: Duration,
    ) -> ThreadServiceFuture<'a, ThreadShutdownReport> {
        Box::pin(ThreadService::shutdown_all_threads_bounded(self, timeout))
    }

    fn shutdown_all_threads_for_runtime_teardown_bounded<'a>(
        &'a self,
        timeout: Duration,
    ) -> ThreadServiceFuture<'a, ThreadShutdownReport> {
        Box::pin(ThreadService::shutdown_all_threads_for_runtime_teardown_bounded(self, timeout))
    }

    fn shutdown_live_thread<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, protocol::error::Result<String>> {
        Box::pin(ThreadService::shutdown_live_thread(self, thread_id))
    }

    fn remove_live_thread<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, bool> {
        Box::pin(ThreadService::remove_live_thread(self, thread_id))
    }

    fn subscribe_thread_created(&self) -> broadcast::Receiver<ThreadCreatedEvent> {
        ThreadService::subscribe_thread_created(self)
    }

    fn live_thread_agent_status<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, protocol::error::Result<protocol::protocol::AgentStatus>> {
        Box::pin(ThreadService::live_thread_agent_status(self, thread_id))
    }

    fn live_thread_runtime_status<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, protocol::error::Result<thread_service_api::ThreadRuntimeStatus>>
    {
        Box::pin(ThreadService::live_thread_runtime_status(self, thread_id))
    }

    fn subscribe_live_thread_status<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<
        'a,
        protocol::error::Result<tokio::sync::watch::Receiver<protocol::protocol::AgentStatus>>,
    > {
        Box::pin(ThreadService::subscribe_live_thread_status(self, thread_id))
    }

    fn active_event_subscriptions(
        &self,
    ) -> Arc<thread_service_api::ActiveEventSubscriptionTracker> {
        ThreadService::active_event_subscriptions(self)
    }
}

impl NativeAgentRuntime for ThreadService {
    fn spawn_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        request: ThreadSpawnAgentRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::spawn_agent_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                to_runtime_spawn_request(request),
            )
            .await
            .map(from_runtime_spawn_result)
        })
    }

    fn followup_task<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
        message: String,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::followup_task_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                target,
                message,
            )
            .await
        })
    }

    fn close_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadCloseAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::close_agent_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                target,
            )
            .await
            .map(from_runtime_close_result)
        })
    }

    fn list_agents<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        path_prefix: Option<String>,
    ) -> ThreadServiceFuture<'a, Result<ThreadListAgentsResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::list_agents_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                path_prefix,
            )
            .await
            .map(from_runtime_list_result)
        })
    }
}

impl ThreadCollaborationRuntime for ThreadService {
    fn spawn_external_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        request: ThreadSpawnExternalAgentRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::spawn_external_agent_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                to_runtime_spawn_external_request(request),
            )
            .await
            .map(from_runtime_spawn_result)
        })
    }

    fn followup_external_task<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
        message: String,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::followup_external_task_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                target,
                message,
            )
            .await
        })
    }

    fn close_external_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadCloseAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::close_external_agent_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                target,
            )
            .await
            .map(from_runtime_close_result)
        })
    }

    fn list_external_agents<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        path_prefix: Option<String>,
    ) -> ThreadServiceFuture<'a, Result<ThreadListAgentsResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::list_external_agents_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                path_prefix,
            )
            .await
            .map(from_runtime_list_result)
        })
    }
}

impl ThreadProviderCatalogRuntime for ThreadService {
    fn list_thread_providers(&self) -> Vec<ThreadProviderRuntimeDescriptor> {
        runtime_thread_provider_descriptors()
    }

    fn resolve_root_thread_provider(
        &self,
        provider_id: Option<&str>,
        capability: ThreadProviderRootCapability,
    ) -> Result<RootThreadProviderRoute, RootThreadProviderResolutionError> {
        resolve_runtime_root_thread_provider(provider_id, capability)
    }
}

impl ThreadAgentDirectoryRuntime for ThreadService {
    fn list_agent_directory<'a>(
        &'a self,
        request: AgentDirectoryListRequest,
    ) -> ThreadServiceFuture<'a, CodexResult<AgentDirectoryListResult>> {
        Box::pin(async move { self.agent_control().list_agent_directory(request).await })
    }

    fn resolve_agent_reference_in_directory<'a>(
        &'a self,
        request: AgentReferenceResolutionRequest,
    ) -> ThreadServiceFuture<'a, CodexResult<AgentReferenceResolution>> {
        Box::pin(async move {
            self.agent_control()
                .resolve_agent_reference_in_directory(request)
                .await
        })
    }

    fn list_agent_subtree_thread_ids<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<Vec<protocol::ThreadId>>> {
        Box::pin(async move {
            self.agent_control()
                .list_agent_subtree_thread_ids(thread_id)
                .await
        })
    }
}

impl PersistedThreadProviderFactsRuntime for ThreadService {
    fn persisted_external_root_thread_facts<'a>(
        &'a self,
        selector: PersistedThreadProviderFactsSelector,
    ) -> ThreadServiceFuture<'a, CodexResult<Option<PersistedExternalRootThreadFacts>>> {
        Box::pin(async move {
            ThreadService::persisted_external_root_thread_facts(self, selector).await
        })
    }
}

impl ExternalRootThreadRuntime for ThreadService {
    fn start_external_root_thread<'a>(
        &'a self,
        request: ExternalRootThreadStartRequest,
    ) -> ThreadServiceFuture<'a, CodexResult<ExternalRootThreadStartResult>> {
        Box::pin(async move {
            let new_thread = ThreadService::start_external_root_thread_with_spawn_config(
                self,
                to_external_spawn_config(request.startup_config),
                to_runtime_external_root_provider(request.provider),
                request
                    .agent_metadata
                    .map(to_runtime_external_root_agent_metadata),
            )
            .await?;
            Ok(ExternalRootThreadStartResult {
                thread_id: new_thread.thread_id,
                session_configured: new_thread.session_configured,
            })
        })
    }

    fn has_external_root_thread(&self, thread_id: protocol::ThreadId) -> bool {
        ThreadService::has_external_root_thread(self, thread_id)
    }

    fn live_external_root_thread_facts(
        &self,
        thread_id: protocol::ThreadId,
    ) -> Option<LiveExternalRootThreadFacts> {
        ThreadService::live_external_root_thread_facts(self, thread_id)
    }

    fn external_root_thread_input_route<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<ExternalRootThreadInputRoute>> {
        Box::pin(
            async move { ThreadService::external_root_thread_input_route(self, thread_id).await },
        )
    }

    fn submit_external_root_input<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
        message: String,
    ) -> ThreadServiceFuture<'a, CodexResult<String>> {
        Box::pin(ThreadService::submit_external_root_input(
            self, thread_id, message,
        ))
    }

    fn close_external_root_thread<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<String>> {
        Box::pin(ThreadService::close_external_root_thread(self, thread_id))
    }
}

impl NativeTurnEventRuntime for ThreadService {
    fn poll_event<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        request: ThreadPollEventRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadPollEventResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            let (default_initial_timeout_ms, default_hard_cap_timeout_ms) =
                turn.default_wait_agent_timeouts();
            session(turn.as_ref())
                .poll_event(ThreadPollEventRequest {
                    initial_timeout_ms: Some(
                        request
                            .initial_timeout_ms
                            .unwrap_or(default_initial_timeout_ms),
                    ),
                    hard_cap_timeout_ms: Some(
                        request
                            .hard_cap_timeout_ms
                            .unwrap_or(default_hard_cap_timeout_ms),
                    ),
                })
                .await
        })
    }

    fn poll_event_timeout_metadata<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        request: ThreadPollEventRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadPollEventTimeoutMetadata, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            let (default_initial_timeout_ms, default_hard_cap_timeout_ms) =
                turn.default_wait_agent_timeouts();
            session(turn.as_ref())
                .poll_event_timeout_metadata(ThreadPollEventRequest {
                    initial_timeout_ms: Some(
                        request
                            .initial_timeout_ms
                            .unwrap_or(default_initial_timeout_ms),
                    ),
                    hard_cap_timeout_ms: Some(
                        request
                            .hard_cap_timeout_ms
                            .unwrap_or(default_hard_cap_timeout_ms),
                    ),
                })
                .await
        })
    }

    fn reset_thread_wait_backoff<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
    ) -> ThreadServiceFuture<'a, ()> {
        Box::pin(async move {
            let turn = match turn_context(turn) {
                Ok(turn) => turn,
                Err(_) => return,
            };
            session(turn.as_ref()).reset_thread_wait_backoff().await;
        })
    }

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        items: Vec<ResponseItem>,
    ) -> ThreadServiceFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let turn = turn_context(turn).map_err(|err| err.to_string())?;
            session(turn.as_ref())
                .record_model_items_and_emit_display_events(turn.as_ref(), items.as_slice())
                .await;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_thread_service_api_split<T>()
    where
        T: ThreadLifecycleRuntime
            + NativeAgentRuntime
            + ThreadCollaborationRuntime
            + ThreadProviderCatalogRuntime
            + ThreadAgentDirectoryRuntime
            + ExternalRootThreadRuntime
            + NativeTurnEventRuntime,
    {
    }

    #[test]
    fn thread_service_implements_split_runtime_traits() {
        assert_thread_service_api_split::<ThreadService>();
    }

    #[test]
    fn runtime_thread_provider_catalog_exposes_root_capabilities() {
        let providers = runtime_thread_provider_descriptors();
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["native", "claude_cli", "opencode", "codex_cli"]
        );

        let native = providers
            .iter()
            .find(|provider| provider.id == "native")
            .expect("native provider");
        assert_eq!(native.kind, ThreadProviderRuntimeKind::Native);
        assert!(native.capabilities.restore_thread);
        assert!(native.capabilities.restore_snapshot);
        assert!(native.capabilities.fork_thread);

        for id in ["claude_cli", "opencode", "codex_cli"] {
            let provider = providers
                .iter()
                .find(|provider| provider.id == id)
                .unwrap_or_else(|| panic!("{id} provider"));
            assert_eq!(provider.kind, ThreadProviderRuntimeKind::ExternalCli);
            assert!(provider.capabilities.start_thread);
            assert!(!provider.capabilities.restore_thread);
            assert!(provider.capabilities.restore_snapshot);
            assert!(!provider.capabilities.fork_thread);
        }

        assert_eq!(
            resolve_runtime_root_thread_provider(
                Some("opencode"),
                ThreadProviderRootCapability::StartThread,
            )
            .unwrap(),
            RootThreadProviderRoute::External(ExternalRootThreadProvider::Opencode)
        );
        assert_eq!(
            resolve_runtime_root_thread_provider(
                Some("opencode"),
                ThreadProviderRootCapability::RestoreSnapshot,
            )
            .unwrap(),
            RootThreadProviderRoute::External(ExternalRootThreadProvider::Opencode)
        );
        assert_eq!(
            resolve_runtime_root_thread_provider(
                Some("native"),
                ThreadProviderRootCapability::RestoreSnapshot,
            )
            .unwrap(),
            RootThreadProviderRoute::Native
        );
        assert_eq!(
            resolve_runtime_root_thread_provider(
                Some("opencode"),
                ThreadProviderRootCapability::RestoreThread,
            )
            .unwrap_err(),
            RootThreadProviderResolutionError::UnsupportedCapability {
                provider_id: "opencode".to_string(),
                capability: ThreadProviderRootCapability::RestoreThread,
            }
        );
        assert_eq!(
            resolve_runtime_root_thread_provider(
                Some("unknown"),
                ThreadProviderRootCapability::StartThread,
            )
            .unwrap_err(),
            RootThreadProviderResolutionError::UnknownProvider {
                provider_id: "unknown".to_string(),
                capability: ThreadProviderRootCapability::StartThread,
            }
        );
    }
}
