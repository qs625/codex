use super::*;
use codex_config_types::AppToolApproval;
use codex_config_types::Constrained;
use codex_config_types::McpServerConfig;
use codex_config_types::McpServerTransportConfig;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use codex_mcp_types::CodexAppsAuthContext;
use codex_mcp_types::McpClientElicitationSupport;
use codex_mcp_types::McpConfig;
use codex_mcp_types::McpPermissionPromptAutoApproveContext;
use codex_mcp_types::effective_mcp_servers;
use codex_mcp_types::mcp_permission_prompt_is_auto_approved;
use codex_mcp_types::with_codex_apps_mcp;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::models::ManagedFileSystemPermissions;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::GranularApprovalConfig;
use pretty_assertions::assert_eq;
use std::collections::HashMap;
use std::path::PathBuf;

fn test_mcp_config(codex_home: PathBuf) -> McpConfig {
    McpConfig {
        chatgpt_base_url: "https://chatgpt.com".to_string(),
        apps_mcp_path_override: None,
        codex_home,
        mcp_oauth_credentials_store_mode: OAuthCredentialsStoreMode::default(),
        mcp_oauth_callback_port: None,
        mcp_oauth_callback_url: None,
        skill_mcp_dependency_install_enabled: true,
        approval_policy: Constrained::allow_any(AskForApproval::OnFailure),
        codex_linux_sandbox_exe: None,
        use_legacy_landlock: false,
        apps_enabled: false,
        client_elicitation_support: McpClientElicitationSupport::Disabled,
        configured_mcp_servers: HashMap::new(),
        plugin_capability_summaries: Vec::new(),
    }
}

fn test_codex_apps_auth_context() -> CodexAppsAuthContext {
    CodexAppsAuthContext {
        uses_codex_backend: true,
        account_id: Some("acct_test".to_string()),
        chatgpt_user_id: Some("user_test".to_string()),
        is_workspace_account: false,
    }
}

#[test]
fn qualified_mcp_tool_name_prefix_sanitizes_server_names_without_lowercasing() {
    assert_eq!(
        qualified_mcp_tool_name_prefix("Some-Server"),
        "mcp__Some_Server__".to_string()
    );
}

#[test]
fn mcp_prompt_auto_approval_honors_unrestricted_managed_profiles() {
    assert!(mcp_permission_prompt_is_auto_approved(
        AskForApproval::Never,
        &PermissionProfile::Managed {
            file_system: ManagedFileSystemPermissions::Unrestricted,
            network: NetworkSandboxPolicy::Enabled,
        },
        McpPermissionPromptAutoApproveContext::default(),
    ));
    assert!(mcp_permission_prompt_is_auto_approved(
        AskForApproval::Never,
        &PermissionProfile::Managed {
            file_system: ManagedFileSystemPermissions::Unrestricted,
            network: NetworkSandboxPolicy::Restricted,
        },
        McpPermissionPromptAutoApproveContext::default(),
    ));
    assert!(!mcp_permission_prompt_is_auto_approved(
        AskForApproval::Never,
        &PermissionProfile::read_only(),
        McpPermissionPromptAutoApproveContext::default(),
    ));
    assert!(!mcp_permission_prompt_is_auto_approved(
        AskForApproval::OnRequest,
        &PermissionProfile::Managed {
            file_system: ManagedFileSystemPermissions::Unrestricted,
            network: NetworkSandboxPolicy::Enabled,
        },
        McpPermissionPromptAutoApproveContext::default(),
    ));
}

#[test]
fn mcp_prompt_auto_approval_honors_approved_tools_in_all_permission_modes() {
    for approval_policy in [
        AskForApproval::UnlessTrusted,
        AskForApproval::OnFailure,
        AskForApproval::OnRequest,
        AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: true,
            rules: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        }),
        AskForApproval::Never,
    ] {
        assert!(mcp_permission_prompt_is_auto_approved(
            approval_policy,
            &PermissionProfile::read_only(),
            McpPermissionPromptAutoApproveContext {
                approvals_reviewer: Some(ApprovalsReviewer::User),
                tool_approval_mode: Some(AppToolApproval::Approve),
            },
        ));
    }

    assert!(!mcp_permission_prompt_is_auto_approved(
        AskForApproval::OnRequest,
        &PermissionProfile::read_only(),
        McpPermissionPromptAutoApproveContext {
            approvals_reviewer: Some(ApprovalsReviewer::AutoReview),
            tool_approval_mode: Some(AppToolApproval::Auto),
        },
    ));
}

#[test]
fn mcp_prompt_auto_approval_rejects_auto_mode_in_default_permission_mode() {
    assert!(!mcp_permission_prompt_is_auto_approved(
        AskForApproval::OnRequest,
        &PermissionProfile::read_only(),
        McpPermissionPromptAutoApproveContext {
            approvals_reviewer: Some(ApprovalsReviewer::User),
            tool_approval_mode: Some(AppToolApproval::Auto),
        },
    ));
}

#[test]
fn codex_apps_server_config_uses_legacy_codex_apps_path() {
    let mut config = test_mcp_config(PathBuf::from("/tmp"));
    let auth_context = test_codex_apps_auth_context();

    let mut servers = with_codex_apps_mcp(HashMap::new(), /*auth_context*/ None, &config);
    assert!(!servers.contains_key(CODEX_APPS_MCP_SERVER_NAME));

    config.apps_enabled = true;

    servers = with_codex_apps_mcp(servers, Some(&auth_context), &config);
    let server = servers
        .get(CODEX_APPS_MCP_SERVER_NAME)
        .expect("codex apps should be present when apps is enabled");
    let config = server
        .configured_config()
        .expect("codex apps should use configured transport");
    let url = match &config.transport {
        McpServerTransportConfig::StreamableHttp { url, .. } => url,
        _ => panic!("expected streamable http transport for codex apps"),
    };

    assert_eq!(url, "https://chatgpt.com/backend-api/wham/apps");
}

#[test]
fn codex_apps_server_config_uses_configured_apps_mcp_path_override() {
    let mut config = test_mcp_config(PathBuf::from("/tmp"));
    config.apps_mcp_path_override = Some("/custom/mcp".to_string());
    config.apps_enabled = true;
    let auth_context = test_codex_apps_auth_context();

    let servers = with_codex_apps_mcp(HashMap::new(), Some(&auth_context), &config);
    let server = servers
        .get(CODEX_APPS_MCP_SERVER_NAME)
        .expect("codex apps should be present when apps is enabled");
    let config = server
        .configured_config()
        .expect("codex apps should use configured transport");
    let url = match &config.transport {
        McpServerTransportConfig::StreamableHttp { url, .. } => url,
        _ => panic!("expected streamable http transport for codex apps"),
    };

    assert_eq!(url, "https://chatgpt.com/backend-api/custom/mcp");
}

#[tokio::test]
async fn effective_mcp_servers_preserve_user_servers_and_add_codex_apps() {
    let codex_home = tempfile::tempdir().expect("tempdir");
    let mut config = test_mcp_config(codex_home.path().to_path_buf());
    config.apps_enabled = true;
    let auth_context = test_codex_apps_auth_context();

    config.configured_mcp_servers.insert(
        "sample".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://user.example/mcp".to_string(),
                bearer_token_env_var: None,
                http_headers: None,
                env_http_headers: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    );
    config.configured_mcp_servers.insert(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://docs.example/mcp".to_string(),
                bearer_token_env_var: None,
                http_headers: None,
                env_http_headers: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    );

    let effective = effective_mcp_servers(&config, Some(&auth_context));

    let sample = effective.get("sample").expect("user server should exist");
    let docs = effective
        .get("docs")
        .expect("configured server should exist");
    let codex_apps = effective
        .get(CODEX_APPS_MCP_SERVER_NAME)
        .expect("codex apps server should exist");

    let sample = sample
        .configured_config()
        .expect("configured server should retain transport");
    let docs = docs
        .configured_config()
        .expect("configured server should retain transport");
    let codex_apps = codex_apps
        .configured_config()
        .expect("codex apps should use configured transport");

    match &sample.transport {
        McpServerTransportConfig::StreamableHttp { url, .. } => {
            assert_eq!(url, "https://user.example/mcp");
        }
        other => panic!("expected streamable http transport, got {other:?}"),
    }
    match &docs.transport {
        McpServerTransportConfig::StreamableHttp { url, .. } => {
            assert_eq!(url, "https://docs.example/mcp");
        }
        other => panic!("expected streamable http transport, got {other:?}"),
    }
    match &codex_apps.transport {
        McpServerTransportConfig::StreamableHttp { url, .. } => {
            assert_eq!(url, "https://chatgpt.com/backend-api/wham/apps");
        }
        other => panic!("expected streamable http transport, got {other:?}"),
    }
}
