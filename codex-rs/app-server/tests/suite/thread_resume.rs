use anyhow::Result;
use app_server_protocol::AskForApproval;
use app_server_protocol::ClientInfo;
use app_server_protocol::CommandExecutionApprovalDecision;
use app_server_protocol::CommandExecutionRequestApprovalResponse;
use app_server_protocol::FileChangeApprovalDecision;
use app_server_protocol::FileChangeRequestApprovalResponse;
use app_server_protocol::ItemStartedNotification;
use app_server_protocol::JSONRPCError;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::PatchApplyStatus;
use app_server_protocol::PatchChangeKind;
use app_server_protocol::RequestId;
use app_server_protocol::ServerNotification;
use app_server_protocol::ServerRequest;
use app_server_protocol::SessionSource;
use app_server_protocol::ThreadActiveFlag;
use app_server_protocol::ThreadGoalClearResponse;
use app_server_protocol::ThreadGoalSetResponse;
use app_server_protocol::ThreadGoalStatus;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadMetadataGitInfoUpdateParams;
use app_server_protocol::ThreadMetadataUpdateParams;
use app_server_protocol::ThreadReadParams;
use app_server_protocol::ThreadReadResponse;
use app_server_protocol::ThreadResumeParams;
use app_server_protocol::ThreadResumeResponse;
use app_server_protocol::ThreadSource;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_server_protocol::ThreadStatus;
use app_server_protocol::TurnItemsView;
use app_server_protocol::TurnStartParams;
use app_server_protocol::TurnStartResponse;
use app_server_protocol::TurnStatus;
use app_server_protocol::UserInput;
use app_test_support::ChatGptAuthFixture;
use app_test_support::McpProcess;
use app_test_support::create_apply_patch_sse_response;
use app_test_support::create_fake_rollout;
use app_test_support::create_fake_rollout_with_text_elements;
use app_test_support::create_fake_rollout_with_token_usage;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::create_shell_command_sse_response;
use app_test_support::rollout_path;
use app_test_support::test_absolute_path;
use app_test_support::to_response;
use app_test_support::write_chatgpt_auth;
use chrono::Utc;
use config_service::types::AuthCredentialsStoreMode;
use codex_login::REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR;
use codex_utils_absolute_path::AbsolutePathBuf;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use protocol::ThreadId;
use protocol::config_types::Personality;
use protocol::mcp::CallToolResult;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::AgentMessageEvent;
use protocol::protocol::EventMsg;
use protocol::protocol::ImageGenerationEndEvent;
use protocol::protocol::McpInvocation;
use protocol::protocol::McpToolCallEndEvent;
use protocol::protocol::SessionMeta;
use protocol::protocol::SessionMetaLine;
use protocol::protocol::SessionSource as RolloutSessionSource;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::ThreadContextUsageCategoryBreakdown;
use protocol::protocol::ThreadContextUsageLoadedSkills;
use protocol::protocol::ThreadContextUsageToolBreakdown;
use protocol::protocol::ThreadContextUsageToolBucket;
use protocol::protocol::ThreadContextUsageUpdatedEvent;
use protocol::protocol::TokenCountEvent;
use protocol::protocol::TokenUsage;
use protocol::protocol::TokenUsageInfo;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::TurnAbortedEvent;
use protocol::protocol::TurnStartedEvent;
use protocol::user_input::ByteRange;
use protocol::user_input::TextElement;
use serde_json::json;
use state::StateRuntime;
use std::fs::FileTimes;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;
use uuid::Uuid;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

use super::analytics::assert_basic_thread_initialized_event;
use super::analytics::mount_analytics_capture;
use super::analytics::thread_initialized_event;
use super::analytics::wait_for_analytics_payload;

#[cfg(windows)]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
#[cfg(not(windows))]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const CODEX_5_2_INSTRUCTIONS_TEMPLATE_DEFAULT: &str = "You are Codex, a coding agent based on GPT-5. You and the user share the same workspace and collaborate to achieve the user's goals.";

fn normalized_existing_path(path: impl AsRef<Path>) -> Result<PathBuf> {
    Ok(AbsolutePathBuf::from_absolute_path(path.as_ref().canonicalize()?)?.into_path_buf())
}

async fn wait_for_responses_request_count(
    server: &wiremock::MockServer,
    expected_count: usize,
) -> Result<()> {
    timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let Some(requests) = server.received_requests().await else {
                anyhow::bail!("wiremock did not record requests");
            };
            let responses_request_count = requests
                .iter()
                .filter(|request| {
                    request.method == "POST" && request.url.path().ends_with("/responses")
                })
                .count();
            if responses_request_count == expected_count {
                return Ok::<(), anyhow::Error>(());
            }
            if responses_request_count > expected_count {
                anyhow::bail!(
                    "expected exactly {expected_count} /responses requests, got {responses_request_count}"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}


mod basic_resume;
mod goals_and_usage;
mod runtime_replays;
mod runtime_running;

// Helper to create a config.toml pointing at the mock model server.
fn create_config_toml(codex_home: &std::path::Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "gpt-5.3-codex"
approval_policy = "never"
sandbox_mode = "read-only"

model_provider = "mock_provider"

[features]
personality = true

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

fn create_config_toml_with_chatgpt_base_url(
    codex_home: &std::path::Path,
    server_uri: &str,
    chatgpt_base_url: &str,
) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "gpt-5.3-codex"
approval_policy = "never"
sandbox_mode = "read-only"
chatgpt_base_url = "{chatgpt_base_url}"

model_provider = "mock_provider"

[features]
personality = true

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

fn create_config_toml_with_required_broken_mcp(
    codex_home: &std::path::Path,
    server_uri: &str,
) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "gpt-5.3-codex"
approval_policy = "never"
sandbox_mode = "read-only"

model_provider = "mock_provider"

[features]
personality = true

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0

[mcp_servers.required_broken]
command = "codex-definitely-not-a-real-binary"
required = true
"#
        ),
    )
}

#[allow(dead_code)]
fn set_rollout_mtime(path: &Path, updated_at_rfc3339: &str) -> Result<()> {
    let parsed = chrono::DateTime::parse_from_rfc3339(updated_at_rfc3339)?.with_timezone(&Utc);
    let times = FileTimes::new().set_modified(parsed.into());
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)?
        .set_times(times)?;
    Ok(())
}

struct RolloutFixture {
    conversation_id: String,
    rollout_file_path: PathBuf,
    before_modified: std::time::SystemTime,
}

fn setup_rollout_fixture(codex_home: &Path, server_uri: &str) -> Result<RolloutFixture> {
    create_config_toml(codex_home, server_uri)?;

    let preview = "Saved user message";
    let filename_ts = "2025-01-05T12-00-00";
    let meta_rfc3339 = "2025-01-05T12:00:00Z";
    let expected_updated_at_rfc3339 = "2025-01-07T00:00:00Z";
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home,
        filename_ts,
        meta_rfc3339,
        preview,
        Vec::new(),
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let rollout_file_path = rollout_path(codex_home, filename_ts, &conversation_id);
    set_rollout_mtime(rollout_file_path.as_path(), expected_updated_at_rfc3339)?;
    let before_modified = std::fs::metadata(&rollout_file_path)?.modified()?;
    Ok(RolloutFixture {
        conversation_id,
        rollout_file_path,
        before_modified,
    })
}
