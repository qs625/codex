use std::collections::HashSet;
use std::sync::Arc;

use codex_exec_server_api::ExecEnvironmentProvider;
use codex_mcp_runtime_api::McpAuthRuntime;
use codex_mcp_runtime_api::McpConnectionRuntimeFactory;
use codex_model_provider_api::SharedModelProviderFactory;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use codex_thread_store_api::LiveThreadFactory;
use codex_thread_store_api::ThreadStore;
use tokio_util::sync::CancellationToken;

use crate::client_common::PromptBuildParams;
use crate::client_common::build_prompt;
use crate::config::Config;
use crate::resolve_installation_id;
use crate::session::session::Session;
use crate::session::turn::built_tools;
use crate::state_db_bridge::StateDbHandle;
use crate::thread::ThreadAuthRuntimes;
use crate::thread::ThreadManager;
use crate::tools::router::ToolRouterFactory;
use codex_extension_api::empty_extension_registry;

/// Build the model-visible `input` list for a single debug turn.
#[doc(hidden)]
pub async fn build_prompt_input(
    mut config: Config,
    input: Vec<UserInput>,
    state_db: Option<StateDbHandle>,
    environment_provider: Arc<dyn ExecEnvironmentProvider>,
    thread_store: Arc<dyn ThreadStore>,
    live_thread_factory: Arc<dyn LiveThreadFactory>,
    auth_runtimes: ThreadAuthRuntimes,
    model_provider_factory: SharedModelProviderFactory,
    tool_router_factory: Arc<dyn ToolRouterFactory>,
    mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
    mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
) -> CodexResult<Vec<ResponseItem>> {
    config.ephemeral = true;

    let installation_id = resolve_installation_id(&config.codex_home).await?;
    let thread_manager = ThreadManager::new_with_mcp_auth_runtime(
        &config,
        auth_runtimes,
        SessionSource::Exec,
        environment_provider,
        empty_extension_registry(),
        /*analytics_events_client*/ None,
        thread_store,
        state_db.clone(),
        live_thread_factory,
        installation_id,
        /*attestation_provider*/ None,
        model_provider_factory,
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        tool_router_factory,
        mcp_auth_runtime,
        mcp_connection_runtime_factory,
    );
    let thread = thread_manager.start_thread(config).await?;

    let output = build_prompt_input_from_session(thread.thread.codex.session.as_ref(), input).await;
    let shutdown = thread.thread.shutdown_and_wait().await;
    let _removed = thread_manager.remove_thread(&thread.thread_id).await;

    shutdown?;
    output
}

pub(crate) async fn build_prompt_input_from_session(
    sess: &Session,
    input: Vec<UserInput>,
) -> CodexResult<Vec<ResponseItem>> {
    let turn_context = sess.new_default_turn().await;
    sess.record_context_updates_and_set_reference_context_item(turn_context.as_ref())
        .await;

    if !input.is_empty() {
        let input_item = codex_model_input::response_input_item_from_user_input(input);
        let response_item = ResponseItem::from(input_item);
        sess.record_conversation_items(turn_context.as_ref(), std::slice::from_ref(&response_item))
            .await;
    }

    let prompt_input = sess
        .clone_history()
        .await
        .for_prompt(&turn_context.model_info.input_modalities);
    let router = built_tools(
        sess,
        turn_context.as_ref(),
        &prompt_input,
        &HashSet::new(),
        Some(turn_context.turn_skills.outcome.as_ref()),
        &CancellationToken::new(),
    )
    .await?;
    let base_instructions = sess.get_base_instructions().await;
    let prompt = build_prompt(PromptBuildParams {
        input: prompt_input,
        tools: router.model_visible_specs(),
        parallel_tool_calls: turn_context.model_info.supports_parallel_tool_calls,
        base_instructions,
        personality: turn_context.personality,
        output_schema: turn_context.final_output_json_schema.clone(),
        output_schema_strict: !crate::guardian::is_guardian_reviewer_source(
            &turn_context.session_source,
        ),
    });

    Ok(prompt.get_formatted_input())
}
