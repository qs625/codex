use std::time::Duration;

use anyhow::Result;
use app_server_protocol::JSONRPCError;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::Model;
use app_server_protocol::ModelListParams;
use app_server_protocol::ModelListResponse;
use app_server_protocol::ModelServiceTier;
use app_server_protocol::ModelUpgradeInfo;
use app_server_protocol::ReasoningEffortOption;
use app_server_protocol::RequestId;
use app_test_support::ChatGptAuthFixture;
use app_test_support::McpProcess;
use app_test_support::to_response;
use app_test_support::write_chatgpt_auth;
use app_test_support::write_models_cache;
use config_service::types::AuthCredentialsStoreMode;
use core_test_support::responses::mount_models_once;
use pretty_assertions::assert_eq;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ModelPreset;
use protocol::openai_models::ModelsResponse;
use protocol::openai_models::ReasoningEffort;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::MockServer;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const INVALID_REQUEST_ERROR_CODE: i64 = -32600;

fn model_from_preset(preset: &ModelPreset, model_provider_id: &str) -> Model {
    Model {
        id: preset.id.clone(),
        model: preset.model.clone(),
        model_provider: Some(model_provider_id.to_string()),
        upgrade: preset.upgrade.as_ref().map(|upgrade| upgrade.id.clone()),
        upgrade_info: preset.upgrade.as_ref().map(|upgrade| ModelUpgradeInfo {
            model: upgrade.id.clone(),
            upgrade_copy: upgrade.upgrade_copy.clone(),
            model_link: upgrade.model_link.clone(),
            migration_markdown: upgrade.migration_markdown.clone(),
        }),
        availability_nux: preset.availability_nux.clone().map(Into::into),
        display_name: preset.display_name.clone(),
        description: preset.description.clone(),
        hidden: !preset.show_in_picker,
        supported_reasoning_efforts: preset
            .supported_reasoning_efforts
            .iter()
            .map(|preset| ReasoningEffortOption {
                reasoning_effort: preset.effort,
                description: preset.description.clone(),
            })
            .collect(),
        default_reasoning_effort: preset.default_reasoning_effort,
        input_modalities: preset.input_modalities.clone(),
        context_window: preset.context_window,
        max_context_window: preset.max_context_window,
        auto_compact_token_limit: preset.auto_compact_token_limit,
        // `write_models_cache()` round-trips through a simplified ModelInfo fixture that does not
        // preserve personality placeholders in base instructions, so app-server list results from
        // cache report `supports_personality = false`.
        // todo(sayan): fix, maybe make roundtrip use ModelInfo only
        supports_personality: false,
        additional_speed_tiers: preset.additional_speed_tiers.clone(),
        service_tiers: preset
            .service_tiers
            .iter()
            .map(|service_tier| ModelServiceTier {
                id: service_tier.id.clone(),
                name: service_tier.name.clone(),
                description: service_tier.description.clone(),
            })
            .collect(),
        is_default: preset.is_default,
    }
}

fn expected_visible_models(model_provider_id: &str) -> Vec<Model> {
    // Filter by supported_in_api to support testing with both ChatGPT and non-ChatGPT auth modes.
    let mut presets = ModelPreset::filter_by_auth(
        thread_service::test_support::all_model_presets().clone(),
        /*chatgpt_mode*/ false,
    );

    // Mirror `ModelsManager::build_available_models()` default selection after auth filtering.
    ModelPreset::mark_default_by_picker_visibility(&mut presets);

    presets
        .iter()
        .filter(|preset| preset.show_in_picker)
        .map(|preset| {
            let mut model = model_from_preset(preset, model_provider_id);
            model.max_context_window = None;
            model
        })
        .collect()
}

#[tokio::test]
async fn list_models_returns_all_models_with_large_limit() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;

    let expected_models = expected_visible_models("openai");

    assert_eq!(&items[..expected_models.len()], expected_models.as_slice());
    assert!(
        items
            .iter()
            .all(|item| item.model_provider.as_deref() != Some("amazon-bedrock"))
    );
    assert!(next_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_includes_configured_custom_model() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        r#"
model = "corp-model"
model_provider = "corp"
model_reasoning_effort = "high"

[model_providers.corp]
name = "Corp Gateway"
base_url = "https://example.invalid/v1"
env_key = "CORP_API_KEY"
"#,
    )?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;

    let configured_model = items
        .iter()
        .find(|item| item.model == "corp-model")
        .expect("configured model is present");
    assert_eq!(
        configured_model,
        &Model {
            id: "configured:corp:corp-model".to_string(),
            model: "corp-model".to_string(),
            model_provider: Some("corp".to_string()),
            upgrade: None,
            upgrade_info: None,
            availability_nux: None,
            display_name: "corp-model".to_string(),
            description: "当前配置中的模型 · Corp Gateway".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![ReasoningEffortOption {
                reasoning_effort: ReasoningEffort::High,
                description: "当前配置的默认 reasoning".to_string(),
            }],
            default_reasoning_effort: ReasoningEffort::High,
            input_modalities: protocol::openai_models::default_input_modalities(),
            context_window: None,
            max_context_window: None,
            auto_compact_token_limit: None,
            supports_personality: false,
            additional_speed_tiers: Vec::new(),
            service_tiers: Vec::new(),
            is_default: false,
        }
    );
    assert!(next_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_does_not_synthesize_models_for_extra_providers() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        r#"
model = "mock-model"
model_provider = "openai"

[model_providers.corp]
name = "Corp Gateway"
base_url = "https://example.invalid/v1"
env_key = "CORP_API_KEY"
"#,
    )?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;

    assert!(
        items
            .iter()
            .all(|item| item.id != "configured:corp:mock-model")
    );
    assert!(next_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_includes_inline_model_options() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        r#"
[[model_options]]
model = "gpt-5.5-2026-04-24"
provider = "modelhub-gpt"
base_url = "https://example.invalid/api/modelhub/online/v2/crawl"
wire_api = "azure_chat_completions"
max_tokens = 500
context_window = 128000
max_context_window = 256000
auto_compact_token_limit = 90000

[model_options.query_params]
ak = "test-key"
"#,
    )?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;

    let configured_model = items
        .iter()
        .find(|item| item.id == "configured:modelhub-gpt:gpt-5.5-2026-04-24")
        .expect("inline model option is present");
    assert_eq!(
        configured_model,
        &Model {
            id: "configured:modelhub-gpt:gpt-5.5-2026-04-24".to_string(),
            model: "gpt-5.5-2026-04-24".to_string(),
            model_provider: Some("modelhub-gpt".to_string()),
            upgrade: None,
            upgrade_info: None,
            availability_nux: None,
            display_name: "gpt-5.5-2026-04-24".to_string(),
            description: "配置文件中的模型 · modelhub-gpt".to_string(),
            hidden: false,
            supported_reasoning_efforts: Vec::new(),
            default_reasoning_effort: ReasoningEffort::None,
            input_modalities: protocol::openai_models::default_input_modalities(),
            context_window: Some(128_000),
            max_context_window: Some(256_000),
            auto_compact_token_limit: Some(90_000),
            supports_personality: false,
            additional_speed_tiers: Vec::new(),
            service_tiers: Vec::new(),
            is_default: false,
        }
    );
    assert!(next_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_includes_configured_bedrock_provider() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        r#"
[model_providers.amazon-bedrock.aws]
profile = "codex-bedrock"
"#,
    )?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse { data: items, .. } = to_response::<ModelListResponse>(response)?;

    assert!(items.iter().any(|item| {
        item.model_provider.as_deref() == Some("amazon-bedrock")
            && item.model == "openai.gpt-5.4"
            && item.context_window == Some(272_000)
            && item.max_context_window == Some(1_000_000)
    }));
    assert!(items.iter().any(|item| {
        item.model_provider.as_deref() == Some("amazon-bedrock")
            && item.model == "openai.gpt-oss-120b"
            && item.context_window == Some(128_000)
            && item.max_context_window == Some(128_000)
    }));
    Ok(())
}

#[tokio::test]
async fn list_models_does_not_label_openai_catalog_as_custom_provider() -> Result<()> {
    let openai_model = expected_visible_models("openai")
        .into_iter()
        .find(|model| !model.is_default)
        .expect("non-default visible model");
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"
model = "{}"
model_provider = "corp"

[model_providers.corp]
name = "Corp Gateway"
base_url = "https://example.invalid/v1"
env_key = "CORP_API_KEY"
"#,
            openai_model.model
        ),
    )?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;

    assert!(items.iter().any(|item| item == &openai_model));
    assert!(!items.iter().any(|item| {
        item.id == openai_model.id && item.model_provider.as_deref() == Some("corp")
    }));
    let matches = items
        .iter()
        .filter(|item| {
            item.model == openai_model.model && item.model_provider.as_deref() == Some("corp")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matches,
        vec![&Model {
            id: format!("configured:corp:{}", openai_model.model),
            model: openai_model.model.clone(),
            model_provider: Some("corp".to_string()),
            upgrade: None,
            upgrade_info: None,
            availability_nux: None,
            display_name: openai_model.model.clone(),
            description: "当前配置中的模型 · Corp Gateway".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![ReasoningEffortOption {
                reasoning_effort: ReasoningEffort::Medium,
                description: "当前配置的默认 reasoning".to_string(),
            }],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: protocol::openai_models::default_input_modalities(),
            context_window: None,
            max_context_window: None,
            auto_compact_token_limit: None,
            supports_personality: false,
            additional_speed_tiers: Vec::new(),
            service_tiers: Vec::new(),
            is_default: false,
        }]
    );
    assert!(next_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_marks_configured_catalog_model_without_duplication() -> Result<()> {
    let configured_model = expected_visible_models("openai")
        .into_iter()
        .find(|model| !model.is_default)
        .expect("non-default visible model");
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"
model = "{}"
model_provider = "openai"
"#,
            configured_model.model
        ),
    )?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse { data: items, .. } = to_response::<ModelListResponse>(response)?;
    let matches = items
        .iter()
        .filter(|item| item.model == configured_model.model)
        .collect::<Vec<_>>();

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, configured_model.id);
    assert!(matches[0].description.ends_with(" · 当前配置: OpenAI"));
    Ok(())
}

#[tokio::test]
async fn list_models_includes_hidden_models() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: Some(true),
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;

    assert!(items.iter().any(|item| item.hidden));
    assert!(next_cursor.is_none());
    Ok(())
}

#[tokio::test]
async fn list_models_uses_chatgpt_remote_catalog_as_source_of_truth() -> Result<()> {
    let server = MockServer::start().await;
    let remote_model: ModelInfo = serde_json::from_value(json!({
        "slug": "gpt-5.6-sol",
        "display_name": "GPT-5.6 Sol",
        "description": "Remote-only GPT-5.6 model for app-server model/list coverage",
        "default_reasoning_level": "max",
        "supported_reasoning_levels": [
            {"effort": "max", "description": "max"},
            {"effort": "ultra", "description": "ultra"}
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "minimal_client_version": [0, 1, 0],
        "supported_in_api": true,
        "priority": 0,
        "upgrade": null,
        "base_instructions": "base instructions",
        "supports_reasoning_summaries": false,
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "truncation_policy": {"mode": "bytes", "limit": 10_000},
        "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "context_window": 272_000,
        "max_context_window": 272_000,
        "experimental_supported_tools": [],
    }))?;
    let models_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: vec![remote_model.clone()],
        },
    )
    .await;

    let codex_home = TempDir::new()?;
    let server_uri = server.uri();
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
openai_base_url = "{server_uri}/v1"
"#
        ),
    )?;
    write_chatgpt_auth(
        codex_home.path(),
        ChatGptAuthFixture::new("chatgpt-access-token").plan_type("pro"),
        AuthCredentialsStoreMode::File,
    )?;

    let mut mcp = McpProcess::new_with_env(codex_home.path(), &[("OPENAI_API_KEY", None)]).await?;
    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let ModelListResponse {
        data: items,
        next_cursor,
    } = to_response::<ModelListResponse>(response)?;
    let mut expected_presets: Vec<ModelPreset> = vec![remote_model.into()];
    ModelPreset::mark_default_by_picker_visibility(&mut expected_presets);
    let expected_openai_items = expected_presets
        .iter()
        .map(|preset| model_from_preset(preset, "openai"))
        .collect::<Vec<_>>();

    assert_eq!(
        &items[..expected_openai_items.len()],
        expected_openai_items.as_slice()
    );
    assert!(items.iter().any(|item| {
        item.model == "gpt-5.6-sol"
            && item.supported_reasoning_efforts
                == vec![
                    ReasoningEffortOption {
                        reasoning_effort: ReasoningEffort::Max,
                        description: "max".to_string(),
                    },
                    ReasoningEffortOption {
                        reasoning_effort: ReasoningEffort::Ultra,
                        description: "ultra".to_string(),
                    },
                ]
            && item.default_reasoning_effort == ReasoningEffort::Max
    }));
    assert!(
        items
            .iter()
            .all(|item| item.model_provider.as_deref() != Some("amazon-bedrock"))
    );
    assert!(items.iter().any(|item| {
        item == &Model {
            id: "configured:openai:mock-model".to_string(),
            model: "mock-model".to_string(),
            model_provider: Some("openai".to_string()),
            upgrade: None,
            upgrade_info: None,
            availability_nux: None,
            display_name: "mock-model".to_string(),
            description: "当前配置中的模型 · OpenAI".to_string(),
            hidden: false,
            supported_reasoning_efforts: vec![ReasoningEffortOption {
                reasoning_effort: ReasoningEffort::Medium,
                description: "当前配置的默认 reasoning".to_string(),
            }],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: protocol::openai_models::default_input_modalities(),
            context_window: None,
            max_context_window: None,
            auto_compact_token_limit: None,
            supports_personality: false,
            additional_speed_tiers: Vec::new(),
            service_tiers: Vec::new(),
            is_default: false,
        }
    }));
    assert!(next_cursor.is_none());
    assert_eq!(
        models_mock.requests().len(),
        1,
        "expected a single /models request"
    );
    Ok(())
}

#[tokio::test]
async fn list_models_pagination_works() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let full_request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: Some(100),
            cursor: None,
            include_hidden: None,
        })
        .await?;

    let full_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(full_request_id)),
    )
    .await??;
    let ModelListResponse {
        data: expected_models,
        ..
    } = to_response::<ModelListResponse>(full_response)?;
    let mut cursor = None;
    let mut items = Vec::new();

    for _ in 0..expected_models.len() {
        let request_id = mcp
            .send_list_models_request(ModelListParams {
                limit: Some(1),
                cursor: cursor.clone(),
                include_hidden: None,
            })
            .await?;

        let response: JSONRPCResponse = timeout(
            DEFAULT_TIMEOUT,
            mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
        )
        .await??;

        let ModelListResponse {
            data: page_items,
            next_cursor,
        } = to_response::<ModelListResponse>(response)?;

        assert_eq!(page_items.len(), 1);
        items.extend(page_items);

        if let Some(next_cursor) = next_cursor {
            cursor = Some(next_cursor);
        } else {
            assert_eq!(items, expected_models);
            return Ok(());
        }
    }

    panic!(
        "model pagination did not terminate after {} pages",
        expected_models.len()
    );
}

#[tokio::test]
async fn list_models_rejects_invalid_cursor() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_models_cache(codex_home.path())?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;

    timeout(DEFAULT_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_list_models_request(ModelListParams {
            limit: None,
            cursor: Some("invalid".to_string()),
            include_hidden: None,
        })
        .await?;

    let error: JSONRPCError = timeout(
        DEFAULT_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(error.id, RequestId::Integer(request_id));
    assert_eq!(error.error.code, INVALID_REQUEST_ERROR_CODE);
    assert_eq!(error.error.message, "invalid cursor: invalid");
    Ok(())
}
