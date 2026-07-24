use std::time::Duration;

use anyhow::Result;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::RequestId;
use app_server_protocol::ThreadProviderKind;
use app_server_protocol::ThreadProviderListResponse;
use app_server_protocol::ThreadProviderModelSelectionMode;
use app_test_support::McpProcess;
use app_test_support::to_response;
use app_test_support::write_models_cache;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn thread_provider_list_scopes_native_roles_and_external_capabilities() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_raw_request(
            "threadProvider/list",
            Some(serde_json::json!({ "cwd": codex_home.path() })),
        )
        .await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let response: ThreadProviderListResponse = to_response(response)?;

    let native = response
        .data
        .iter()
        .find(|provider| provider.id == "native")
        .expect("native provider descriptor should be present");
    assert_eq!(native.display_name, "Morpheus");
    assert_eq!(native.kind, ThreadProviderKind::Native);
    assert_eq!(
        native.model_selection.mode,
        ThreadProviderModelSelectionMode::Catalog
    );
    assert!(
        native
            .model_selection
            .model_providers
            .iter()
            .any(|provider| provider == "openai")
    );
    assert!(
        native
            .agent_types
            .iter()
            .any(|agent_type| agent_type.name == "default")
    );
    assert!(native.capabilities.start_thread);
    assert!(native.capabilities.compact);
    assert!(native.capabilities.workflow);
    assert!(native.capabilities.poll_event);
    assert!(
        native.capabilities.restore_thread,
        "native should advertise live restore support"
    );
    assert!(
        native.capabilities.restore_snapshot,
        "native should advertise persisted snapshot restore support"
    );

    for external_id in ["claude_cli", "opencode", "codex_cli"] {
        let external = response
            .data
            .iter()
            .find(|provider| provider.id == external_id)
            .unwrap_or_else(|| panic!("{external_id} descriptor should be present"));
        assert_eq!(external.kind, ThreadProviderKind::ExternalCli);
        assert!(external.agent_types.is_empty());
        assert_eq!(
            external.model_selection.mode,
            ThreadProviderModelSelectionMode::ProviderDefault
        );
        assert!(external.model_selection.model_providers.is_empty());
        assert!(
            external.capabilities.start_thread,
            "{external_id} should advertise root thread/start because the provider has a backend session transport"
        );
        assert!(
            external.capabilities.send_input,
            "{external_id} should accept text input for a live external root thread"
        );
        assert!(
            external.capabilities.close_thread,
            "{external_id} should support closing live external root threads"
        );
        assert!(
            external.capabilities.list_children,
            "{external_id} should list external collaboration children"
        );
        assert!(
            !external.capabilities.restore_thread,
            "{external_id} should not advertise live restore/reconnect support"
        );
        assert!(
            external.capabilities.restore_snapshot,
            "{external_id} should advertise read-only persisted snapshot restore support"
        );
        assert!(external.capabilities.event_stream);
        assert!(!external.capabilities.compact);
        assert!(!external.capabilities.workflow);
        assert!(external.capabilities.poll_event);
        assert!(!external.capabilities.command_session);
        assert!(!external.capabilities.permissions);
        assert!(!external.capabilities.dynamic_tools);
    }

    Ok(())
}
