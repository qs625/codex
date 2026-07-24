use anyhow::Result;
use app_server::INPUT_TOO_LARGE_ERROR_CODE;
use app_server::INVALID_PARAMS_ERROR_CODE;
use app_server_protocol::ByteRange;
use app_server_protocol::ClientInfo;
use app_server_protocol::CollabAgentTool;
use app_server_protocol::CollabAgentToolCallStatus;
use app_server_protocol::CommandExecutionApprovalDecision;
use app_server_protocol::CommandExecutionRequestApprovalResponse;
use app_server_protocol::CommandExecutionStatus;
use app_server_protocol::FileChangeApprovalDecision;
use app_server_protocol::FileChangePatchUpdatedNotification;
use app_server_protocol::FileChangeRequestApprovalResponse;
use app_server_protocol::ItemCompletedNotification;
use app_server_protocol::ItemStartedNotification;
use app_server_protocol::JSONRPCError;
use app_server_protocol::JSONRPCMessage;
use app_server_protocol::JSONRPCNotification;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::PatchApplyStatus;
use app_server_protocol::PatchChangeKind;
use app_server_protocol::RequestId;
use app_server_protocol::ServerRequest;
use app_server_protocol::ServerRequestResolvedNotification;
use app_server_protocol::TextElement;
use app_server_protocol::ThreadApproveGuardianDeniedActionParams;
use app_server_protocol::ThreadBackgroundTerminalsCleanParams;
use app_server_protocol::ThreadCompactStartParams;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadLifecycleStatus;
use app_server_protocol::ThreadRollbackParams;
use app_server_protocol::ThreadShellCommandParams;
use app_server_protocol::ThreadSource;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_server_protocol::ThreadStartedNotification;
use app_server_protocol::TurnCompletedNotification;
use app_server_protocol::TurnEnvironmentParams;
use app_server_protocol::TurnItemsView;
use app_server_protocol::TurnStartParams;
use app_server_protocol::TurnStartResponse;
use app_server_protocol::TurnStartedNotification;
use app_server_protocol::TurnStatus;
use app_server_protocol::UserInput as V2UserInput;
use app_server_protocol::WarningNotification;
use app_test_support::DEFAULT_CLIENT_NAME;
use app_test_support::McpProcess;
use app_test_support::create_apply_patch_sse_response;
use app_test_support::create_exec_command_sse_response;
use app_test_support::create_fake_rollout;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::create_mock_responses_server_sequence;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::create_shell_command_sse_response;
use app_test_support::format_with_current_shell_display;
use app_test_support::to_response;
use app_test_support::write_mock_responses_config_toml_with_chatgpt_base_url;
use app_test_support::write_models_cache;
use config_service::config_toml::ConfigToml;
use codex_features::FEATURES;
use codex_features::Feature;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use protocol::config_types::CollaborationMode;
use protocol::config_types::ModeKind;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary;
use protocol::config_types::Settings;
use protocol::models::BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS;
use protocol::openai_models::ReasoningEffort;
use protocol::user_input::MAX_USER_INPUT_TEXT_CHARS;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use tempfile::TempDir;
use thread_service::personality_migration::PERSONALITY_MIGRATION_FILENAME;
use thread_service::test_support::all_model_presets;
use tokio::time::timeout;

use super::analytics::mount_analytics_capture;
use super::analytics::wait_for_analytics_event;

#[cfg(windows)]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
#[cfg(not(windows))]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const TEST_ORIGINATOR: &str = "codex_vscode";
const LOCAL_PRAGMATIC_TEMPLATE: &str = "You are a deeply pragmatic, effective software engineer.";
const INVALID_REQUEST_ERROR_CODE: i64 = -32600;

fn body_contains(req: &wiremock::Request, text: &str) -> bool {
    String::from_utf8(req.body.clone())
        .ok()
        .is_some_and(|body| body.contains(text))
}


mod approvals;
mod basics;
mod environments;
mod external_root;
mod overrides;
mod tool_items;

// Helper to create a config.toml pointing at the mock model server.
fn create_config_toml(
    codex_home: &Path,
    server_uri: &str,
    approval_policy: &str,
    feature_flags: &BTreeMap<Feature, bool>,
) -> std::io::Result<()> {
    create_config_toml_with_sandbox(
        codex_home,
        server_uri,
        approval_policy,
        feature_flags,
        "read-only",
    )
}

fn create_config_toml_with_sandbox(
    codex_home: &Path,
    server_uri: &str,
    approval_policy: &str,
    feature_flags: &BTreeMap<Feature, bool>,
    sandbox_mode: &str,
) -> std::io::Result<()> {
    let mut features = BTreeMap::new();
    for (feature, enabled) in feature_flags {
        features.insert(*feature, *enabled);
    }
    let feature_entries = features
        .into_iter()
        .map(|(feature, enabled)| {
            let key = FEATURES
                .iter()
                .find(|spec| spec.id == feature)
                .map(|spec| spec.key)
                .unwrap_or_else(|| panic!("missing feature key for {feature:?}"));
            format!("{key} = {enabled}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "{approval_policy}"
sandbox_mode = "{sandbox_mode}"

model_provider = "mock_provider"

[features]
{feature_entries}

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}

fn write_test_skill(codex_home: &Path, name: &str) -> std::io::Result<()> {
    let skill_dir = codex_home.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {name} description\n---\n\n# Body\n"),
    )
}

fn write_fake_claude_cli(bin_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(bin_dir)?;
    let fake_claude = bin_dir.join("claude");
    std::fs::write(
        &fake_claude,
        "#!/bin/sh\n# Test double for hidden external root turn/start wiring.\nsleep 30\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fake_claude)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_claude, permissions)?;
    }
    Ok(())
}

fn prepend_path_env(path: &Path) -> Result<String> {
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(path.to_path_buf()).chain(std::env::split_paths(&original_path));
    Ok(std::env::join_paths(paths)?.to_string_lossy().into_owned())
}
