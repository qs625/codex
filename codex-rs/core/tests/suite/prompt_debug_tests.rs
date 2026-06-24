use anyhow::Result;
use codex_core::ThreadAuthRuntimes;
use codex_core::build_prompt_input;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_core::config::ThreadStoreConfig;
use codex_login::CodexAuth;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::user_input::UserInput;
use codex_rollout::StateDbHandle;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tempfile::TempDir;

struct TestToolRouterFactory;

impl codex_core::ToolRouterFactory for TestToolRouterFactory {
    fn build_tool_router(
        &self,
        config: &codex_tool_config::ToolsConfig,
        params: codex_core::ToolRouterBuildParams<'_>,
    ) -> codex_core::CoreToolRuntimeRouter {
        codex_tool_handlers::build_tool_router(
            config,
            &codex_core::CoreToolDomainHost,
            codex_tool_handlers::ToolRuntimeBuildParams {
                mcp_tools: params.mcp_tools,
                deferred_mcp_tools: params.deferred_mcp_tools,
                discoverable_tools: params.discoverable_tools,
                extension_tool_executors: params.extension_tool_executors,
                dynamic_tools: params.dynamic_tools,
                default_agent_type_description: params.default_agent_type_description,
            },
        )
    }
}

fn thread_store_from_config(
    config: &Config,
    state_db: Option<StateDbHandle>,
) -> Arc<dyn codex_thread_store::ThreadStore> {
    match &config.experimental_thread_store {
        ThreadStoreConfig::Local => Arc::new(codex_thread_store::LocalThreadStore::new(
            codex_thread_store::LocalThreadStoreConfig::from_config(config),
            state_db,
        )),
        ThreadStoreConfig::InMemory { id } => codex_thread_store::InMemoryThreadStore::for_id(id),
    }
}

#[tokio::test]
async fn build_prompt_input_includes_context_and_user_message() -> Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            codex_self_exe: Some(std::env::current_exe()?),
            ..ConfigOverrides::default()
        })
        .build()
        .await?;
    config.user_instructions = Some("Project-specific test instructions".to_string());

    let runtime_paths = codex_exec_server_api::ExecServerRuntimePaths::new(
        std::env::current_exe()?,
        /*codex_linux_sandbox_exe*/ None,
    )?;
    let environment_provider = Arc::new(
        codex_exec_server::EnvironmentManager::from_codex_home(
            config.codex_home.clone(),
            runtime_paths,
        )
        .await?,
    );

    let state_db: Option<StateDbHandle> = None;
    let thread_store = thread_store_from_config(&config, state_db.clone());
    let auth_manager =
        codex_core::test_support::auth_manager_from_auth(CodexAuth::from_api_key("test"));
    let auth_runtimes = ThreadAuthRuntimes::from_auth_runtime(
        auth_manager.clone(),
        codex_login::model_provider_auth_manager(Some(auth_manager)),
    );
    let input = build_prompt_input(
        config,
        vec![UserInput::Text {
            text: "hello from debug prompt".to_string(),
            text_elements: Vec::new(),
        }],
        state_db,
        environment_provider,
        thread_store,
        Arc::new(codex_thread_store::DefaultLiveThreadFactory),
        auth_runtimes,
        codex_core::test_support::model_provider_factory_for_tests(),
        Arc::new(TestToolRouterFactory),
        Arc::new(codex_mcp::DefaultMcpAuthRuntime),
        Arc::new(codex_mcp::DefaultMcpConnectionRuntimeFactory),
    )
    .await?;

    let expected_user_message = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "hello from debug prompt".to_string(),
        }],
        phase: None,
    };
    assert_eq!(input.last(), Some(&expected_user_message));
    assert!(input.iter().any(|item| {
        let ResponseItem::Message { content, .. } = item else {
            return false;
        };

        content.iter().any(|content_item| {
            let (ContentItem::InputText { text } | ContentItem::OutputText { text }) = content_item
            else {
                return false;
            };
            text.contains("Project-specific test instructions")
        })
    }));

    Ok(())
}
