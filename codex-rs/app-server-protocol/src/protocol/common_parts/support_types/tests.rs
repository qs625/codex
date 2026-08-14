use super::*;
use anyhow::Result;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::test_support::PathBufExt;
use codex_utils_absolute_path::test_support::test_path_buf;
use pretty_assertions::assert_eq;
use protocol::ThreadId;
use protocol::account::PlanType;
use protocol::protocol::RealtimeConversationVersion;
use protocol::protocol::RealtimeOutputModality;
use protocol::protocol::RealtimeVoice;
use serde_json::json;
use std::path::PathBuf;

fn absolute_path_string(path: &str) -> String {
    let path = format!("/{}", path.trim_start_matches('/'));
    test_path_buf(&path).display().to_string()
}

fn absolute_path(path: &str) -> AbsolutePathBuf {
    let path = format!("/{}", path.trim_start_matches('/'));
    test_path_buf(&path).abs()
}

fn request_id() -> RequestId {
    const REQUEST_ID: i64 = 1;
    RequestId::Integer(REQUEST_ID)
}

#[test]
fn client_request_serialization_scope_covers_keyed_families() {
    let thread_id = "thread-1".to_string();
    let thread_resume = ClientRequest::ThreadResume {
        request_id: request_id(),
        params: v2::ThreadResumeParams {
            thread_id: thread_id.clone(),
            ..Default::default()
        },
    };
    assert_eq!(
        thread_resume.serialization_scope(),
        Some(ClientRequestSerializationScope::Thread {
            thread_id: thread_id.clone()
        })
    );

    let thread_resume_with_path = ClientRequest::ThreadResume {
        request_id: request_id(),
        params: v2::ThreadResumeParams {
            thread_id: thread_id.clone(),
            path: Some(PathBuf::from("/tmp/resume-thread.jsonl")),
            ..Default::default()
        },
    };
    assert_eq!(
        thread_resume_with_path.serialization_scope(),
        Some(ClientRequestSerializationScope::Thread {
            thread_id: thread_id.clone()
        })
    );

    let thread_fork = ClientRequest::ThreadFork {
        request_id: request_id(),
        params: v2::ThreadForkParams {
            thread_id: thread_id.clone(),
            path: Some(PathBuf::from("/tmp/source-thread.jsonl")),
            ..Default::default()
        },
    };
    assert_eq!(
        thread_fork.serialization_scope(),
        Some(ClientRequestSerializationScope::Thread { thread_id })
    );

    let command_exec = ClientRequest::OneOffCommandExec {
        request_id: request_id(),
        params: v2::CommandExecParams {
            command: vec!["sleep".to_string(), "10".to_string()],
            process_id: Some("proc-1".to_string()),
            tty: false,
            stream_stdin: false,
            stream_stdout_stderr: false,
            output_bytes_cap: None,
            disable_output_cap: false,
            disable_timeout: false,
            timeout_ms: None,
            cwd: None,
            env: None,
            size: None,
            sandbox_policy: None,
            permission_profile: None,
        },
    };
    assert_eq!(
        command_exec.serialization_scope(),
        Some(ClientRequestSerializationScope::CommandExecProcess {
            process_id: "proc-1".to_string()
        })
    );

    let fuzzy_update = ClientRequest::FuzzyFileSearchSessionUpdate {
        request_id: request_id(),
        params: FuzzyFileSearchSessionUpdateParams {
            session_id: "search-1".to_string(),
            query: "lib".to_string(),
        },
    };
    assert_eq!(
        fuzzy_update.serialization_scope(),
        Some(ClientRequestSerializationScope::FuzzyFileSearchSession {
            session_id: "search-1".to_string()
        })
    );

    let fs_watch = ClientRequest::FsWatch {
        request_id: request_id(),
        params: v2::FsWatchParams {
            watch_id: "watch-1".to_string(),
            path: absolute_path("/tmp/repo"),
        },
    };
    assert_eq!(
        fs_watch.serialization_scope(),
        Some(ClientRequestSerializationScope::FsWatch {
            watch_id: "watch-1".to_string()
        })
    );

    let plugin_install = ClientRequest::PluginInstall {
        request_id: request_id(),
        params: v2::PluginInstallParams {
            marketplace_path: Some(absolute_path("/tmp/marketplace")),
            remote_marketplace_name: None,
            plugin_name: "plugin-a".to_string(),
        },
    };
    assert_eq!(
        plugin_install.serialization_scope(),
        Some(ClientRequestSerializationScope::Global("config"))
    );

    let skills_list = ClientRequest::SkillsList {
        request_id: request_id(),
        params: v2::SkillsListParams {
            cwds: Vec::new(),
            force_reload: false,
        },
    };
    assert_eq!(
        skills_list.serialization_scope(),
        Some(ClientRequestSerializationScope::GlobalSharedRead("config"))
    );

    let plugin_list = ClientRequest::PluginList {
        request_id: request_id(),
        params: v2::PluginListParams {
            cwds: None,
            marketplace_kinds: None,
        },
    };
    assert_eq!(plugin_list.serialization_scope(), None);

    let plugin_read = ClientRequest::PluginRead {
        request_id: request_id(),
        params: v2::PluginReadParams {
            marketplace_path: Some(absolute_path("/tmp/marketplace")),
            remote_marketplace_name: None,
            plugin_name: "plugin-a".to_string(),
        },
    };
    assert_eq!(plugin_read.serialization_scope(), None);

    let plugin_uninstall = ClientRequest::PluginUninstall {
        request_id: request_id(),
        params: v2::PluginUninstallParams {
            plugin_id: "plugin-a".to_string(),
        },
    };
    assert_eq!(
        plugin_uninstall.serialization_scope(),
        Some(ClientRequestSerializationScope::Global("config"))
    );

    let mcp_oauth = ClientRequest::McpServerOauthLogin {
        request_id: request_id(),
        params: v2::McpServerOauthLoginParams {
            name: "server-a".to_string(),
            scopes: None,
            timeout_secs: None,
        },
    };
    assert_eq!(
        mcp_oauth.serialization_scope(),
        Some(ClientRequestSerializationScope::McpOauth {
            server_name: "server-a".to_string()
        })
    );

    let mcp_resource_read = ClientRequest::McpResourceRead {
        request_id: request_id(),
        params: v2::McpResourceReadParams {
            thread_id: Some("thread-1".to_string()),
            server: "server-a".to_string(),
            uri: "file:///tmp/resource".to_string(),
        },
    };
    assert_eq!(
        mcp_resource_read.serialization_scope(),
        Some(ClientRequestSerializationScope::Thread {
            thread_id: "thread-1".to_string()
        })
    );

    let config_read = ClientRequest::ConfigRead {
        request_id: request_id(),
        params: v2::ConfigReadParams {
            include_layers: false,
            cwd: None,
        },
    };
    assert_eq!(
        config_read.serialization_scope(),
        Some(ClientRequestSerializationScope::GlobalSharedRead("config"))
    );

    let account_read = ClientRequest::GetAccount {
        request_id: request_id(),
        params: v2::GetAccountParams {
            refresh_token: false,
        },
    };
    assert_eq!(
        account_read.serialization_scope(),
        Some(ClientRequestSerializationScope::Global("account-auth"))
    );

    let thread_goal_set = ClientRequest::ThreadGoalSet {
        request_id: request_id(),
        params: v2::ThreadGoalSetParams {
            thread_id: "goal-thread".to_string(),
            objective: Some("ship it".to_string()),
            status: None,
            token_budget: None,
        },
    };
    assert_eq!(
        thread_goal_set.serialization_scope(),
        Some(ClientRequestSerializationScope::Thread {
            thread_id: "goal-thread".to_string()
        })
    );

    let guardian_approval = ClientRequest::ThreadApproveGuardianDeniedAction {
        request_id: request_id(),
        params: v2::ThreadApproveGuardianDeniedActionParams {
            thread_id: "guardian-thread".to_string(),
            event: json!({ "type": "guardian" }),
        },
    };
    assert_eq!(
        guardian_approval.serialization_scope(),
        Some(ClientRequestSerializationScope::Thread {
            thread_id: "guardian-thread".to_string()
        })
    );

    let marketplace_remove = ClientRequest::MarketplaceRemove {
        request_id: request_id(),
        params: v2::MarketplaceRemoveParams {
            marketplace_name: "marketplace".to_string(),
        },
    };
    assert_eq!(
        marketplace_remove.serialization_scope(),
        Some(ClientRequestSerializationScope::Global("config"))
    );

    let add_credits_nudge = ClientRequest::SendAddCreditsNudgeEmail {
        request_id: request_id(),
        params: v2::SendAddCreditsNudgeEmailParams {
            credit_type: v2::AddCreditsNudgeCreditType::Credits,
        },
    };
    assert_eq!(
        add_credits_nudge.serialization_scope(),
        Some(ClientRequestSerializationScope::Global("account-auth"))
    );

    let environment_add = ClientRequest::EnvironmentAdd {
        request_id: request_id(),
        params: v2::EnvironmentAddParams {
            environment_id: "remote-a".to_string(),
            exec_server_url: "ws://127.0.0.1:8765".to_string(),
        },
    };
    assert_eq!(
        environment_add.serialization_scope(),
        Some(ClientRequestSerializationScope::Global("environment"))
    );
}

#[test]
fn client_request_serialization_scope_covers_unkeyed_representatives() {
    let thread_start = ClientRequest::ThreadStart {
        request_id: request_id(),
        params: v2::ThreadStartParams::default(),
    };
    assert_eq!(thread_start.serialization_scope(), None);

    let command_exec = ClientRequest::OneOffCommandExec {
        request_id: request_id(),
        params: v2::CommandExecParams {
            command: vec!["true".to_string()],
            process_id: None,
            tty: false,
            stream_stdin: false,
            stream_stdout_stderr: false,
            output_bytes_cap: None,
            disable_output_cap: false,
            disable_timeout: false,
            timeout_ms: None,
            cwd: None,
            env: None,
            size: None,
            sandbox_policy: None,
            permission_profile: None,
        },
    };
    assert_eq!(command_exec.serialization_scope(), None);

    let fs_read = ClientRequest::FsReadFile {
        request_id: request_id(),
        params: v2::FsReadFileParams {
            path: absolute_path("/tmp/file.txt"),
        },
    };
    assert_eq!(fs_read.serialization_scope(), None);

    let thread_turns_list = ClientRequest::ThreadTurnsList {
        request_id: request_id(),
        params: v2::ThreadTurnsListParams {
            thread_id: "thread-1".to_string(),
            cursor: None,
            limit: None,
            sort_direction: None,
            items_view: None,
        },
    };
    assert_eq!(thread_turns_list.serialization_scope(), None);

    let thread_turns_items_list = ClientRequest::ThreadTurnsItemsList {
        request_id: request_id(),
        params: v2::ThreadTurnsItemsListParams {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            cursor: None,
            limit: None,
            sort_direction: None,
        },
    };
    assert_eq!(thread_turns_items_list.serialization_scope(), None);

    let mcp_resource_read = ClientRequest::McpResourceRead {
        request_id: request_id(),
        params: v2::McpResourceReadParams {
            thread_id: None,
            server: "server-a".to_string(),
            uri: "file:///tmp/resource".to_string(),
        },
    };
    assert_eq!(mcp_resource_read.serialization_scope(), None);
}

#[test]
fn conversation_id_serializes_as_plain_string() -> Result<()> {
    let id = ThreadId::from_string("67e55044-10b1-426f-9247-bb680e5fe0c8")?;
    assert_eq!(
        json!("67e55044-10b1-426f-9247-bb680e5fe0c8"),
        serde_json::to_value(id)?
    );
    Ok(())
}

#[test]
fn conversation_id_deserializes_from_plain_string() -> Result<()> {
    let id: ThreadId = serde_json::from_value(json!("67e55044-10b1-426f-9247-bb680e5fe0c8"))?;
    assert_eq!(
        ThreadId::from_string("67e55044-10b1-426f-9247-bb680e5fe0c8")?,
        id,
    );
    Ok(())
}

#[test]
fn serialize_client_notification() -> Result<()> {
    let notification = ClientNotification::Initialized;
    assert_eq!(
        json!({
            "method": "initialized",
        }),
        serde_json::to_value(&notification)?,
    );
    Ok(())
}

#[test]
fn serialize_chatgpt_auth_tokens_refresh_request() -> Result<()> {
    let request = ServerRequest::ChatgptAuthTokensRefresh {
        request_id: RequestId::Integer(8),
        params: v2::ChatgptAuthTokensRefreshParams {
            reason: v2::ChatgptAuthTokensRefreshReason::Unauthorized,
            previous_account_id: Some("org-123".to_string()),
        },
    };
    assert_eq!(
        json!({
            "method": "account/chatgptAuthTokens/refresh",
            "id": 8,
            "params": {
                "reason": "unauthorized",
                "previousAccountId": "org-123"
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_attestation_generate_request() -> Result<()> {
    let params = v2::AttestationGenerateParams {};
    let request = ServerRequest::AttestationGenerate {
        request_id: RequestId::Integer(9),
        params: params.clone(),
    };
    assert_eq!(
        json!({
            "method": "attestation/generate",
            "id": 9,
            "params": {}
        }),
        serde_json::to_value(&request)?,
    );

    let payload = ServerRequestPayload::AttestationGenerate(params);
    assert_eq!(request.id(), &RequestId::Integer(9));
    assert_eq!(payload.request_with_id(RequestId::Integer(9)), request);
    Ok(())
}

#[test]
fn serialize_server_response() -> Result<()> {
    let response = ServerResponse::CommandExecutionRequestApproval {
        request_id: RequestId::Integer(8),
        response: v2::CommandExecutionRequestApprovalResponse {
            decision: v2::CommandExecutionApprovalDecision::AcceptForSession,
        },
    };

    assert_eq!(response.id(), &RequestId::Integer(8));
    assert_eq!(response.method(), "item/commandExecution/requestApproval");
    assert_eq!(
        json!({
            "method": "item/commandExecution/requestApproval",
            "id": 8,
            "response": {
                "decision": "acceptForSession"
            }
        }),
        serde_json::to_value(&response)?,
    );
    Ok(())
}

#[test]
fn serialize_mcp_server_elicitation_request() -> Result<()> {
    let requested_schema: v2::McpElicitationSchema = serde_json::from_value(json!({
        "type": "object",
        "properties": {
            "confirmed": {
                "type": "boolean"
            }
        },
        "required": ["confirmed"]
    }))?;
    let params = v2::McpServerElicitationRequestParams {
        thread_id: "thr_123".to_string(),
        turn_id: Some("turn_123".to_string()),
        server_name: "codex_apps".to_string(),
        request: v2::McpServerElicitationRequest::Form {
            meta: None,
            message: "Allow this request?".to_string(),
            requested_schema,
        },
    };
    let request = ServerRequest::McpServerElicitationRequest {
        request_id: RequestId::Integer(9),
        params: params.clone(),
    };

    assert_eq!(
        json!({
            "method": "mcpServer/elicitation/request",
            "id": 9,
            "params": {
                "threadId": "thr_123",
                "turnId": "turn_123",
                "serverName": "codex_apps",
                "mode": "form",
                "_meta": null,
                "message": "Allow this request?",
                "requestedSchema": {
                    "type": "object",
                    "properties": {
                        "confirmed": {
                            "type": "boolean"
                        }
                    },
                    "required": ["confirmed"]
                }
            }
        }),
        serde_json::to_value(&request)?,
    );

    let payload = ServerRequestPayload::McpServerElicitationRequest(params);
    assert_eq!(request.id(), &RequestId::Integer(9));
    assert_eq!(payload.request_with_id(RequestId::Integer(9)), request);
    Ok(())
}

#[test]
fn serialize_get_account_rate_limits() -> Result<()> {
    let request = ClientRequest::GetAccountRateLimits {
        request_id: RequestId::Integer(1),
        params: None,
    };
    assert_eq!(request.id(), &RequestId::Integer(1));
    assert_eq!(request.method(), "account/rateLimits/read");
    assert_eq!(
        json!({
            "method": "account/rateLimits/read",
            "id": 1,
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_client_response() -> Result<()> {
    let cwd = absolute_path("/tmp");
    let response = ClientResponse::ThreadStart {
        request_id: RequestId::Integer(7),
        response: v2::ThreadStartResponse {
            thread: v2::Thread {
                id: "67e55044-10b1-426f-9247-bb680e5fe0c8".to_string(),
                session_id: "67e55044-10b1-426f-9247-bb680e5fe0c7".to_string(),
                forked_from_id: None,
                preview: "first prompt".to_string(),
                ephemeral: true,
                model_provider: "openai".to_string(),
                created_at: 1,
                updated_at: 2,
                lifecycle_status: v2::ThreadLifecycleStatus::completed(None),
                path: None,
                cwd: cwd.clone(),
                cli_version: "0.0.0".to_string(),
                source: v2::SessionSource::Exec,
                thread_source: None,
                agent_nickname: None,
                agent_role: None,
                agent_path: None,
                git_info: None,
                name: None,
                skills: Vec::new(),
                token_usage: None,
                context_usage: None,
                turns: Vec::new(),
                active_subscription_items: None,
            },
            model: "gpt-5".to_string(),
            model_provider: "openai".to_string(),
            service_tier: None,
            cwd,
            runtime_workspace_roots: Vec::new(),
            instruction_sources: vec![absolute_path("/tmp/AGENTS.md")],
            approval_policy: v2::AskForApproval::OnFailure,
            approvals_reviewer: v2::ApprovalsReviewer::User,
            sandbox: v2::SandboxPolicy::DangerFullAccess,
            permission_profile: None,
            active_permission_profile: None,
            reasoning_effort: None,
        },
    };

    assert_eq!(response.id(), &RequestId::Integer(7));
    assert_eq!(response.method(), "thread/start");
    assert_eq!(
        json!({
            "method": "thread/start",
            "id": 7,
            "response": {
                "thread": {
                    "id": "67e55044-10b1-426f-9247-bb680e5fe0c8",
                    "sessionId": "67e55044-10b1-426f-9247-bb680e5fe0c7",
                    "forkedFromId": null,
                    "preview": "first prompt",
                    "ephemeral": true,
                    "modelProvider": "openai",
                    "createdAt": 1,
                    "updatedAt": 2,
                    "lifecycleStatus": {
                        "type": "final",
                        "result": {
                            "type": "completed"
                        }
                    },
                    "path": null,
                    "cwd": absolute_path_string("tmp"),
                    "cliVersion": "0.0.0",
                    "source": "exec",
                    "threadSource": null,
                    "agentNickname": null,
                    "agentRole": null,
                    "agentPath": null,
                    "gitInfo": null,
                    "name": null,
                    "skills": [],
                    "tokenUsage": null,
                    "contextUsage": null,
                    "turns": []
                },
                "model": "gpt-5",
                "modelProvider": "openai",
                "serviceTier": null,
                "cwd": absolute_path_string("tmp"),
                "runtimeWorkspaceRoots": [],
                "instructionSources": [absolute_path_string("tmp/AGENTS.md")],
                "approvalPolicy": "on-failure",
                "approvalsReviewer": "user",
                "sandbox": {
                    "type": "dangerFullAccess"
                },
                "permissionProfile": null,
                "activePermissionProfile": null,
                "reasoningEffort": null
            }
        }),
        serde_json::to_value(&response)?,
    );
    Ok(())
}

#[test]
fn serialize_config_requirements_read() -> Result<()> {
    let request = ClientRequest::ConfigRequirementsRead {
        request_id: RequestId::Integer(1),
        params: None,
    };
    assert_eq!(
        json!({
            "method": "configRequirements/read",
            "id": 1,
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_account_login_api_key() -> Result<()> {
    let request = ClientRequest::LoginAccount {
        request_id: RequestId::Integer(2),
        params: v2::LoginAccountParams::ApiKey {
            api_key: "secret".to_string(),
        },
    };
    assert_eq!(
        json!({
            "method": "account/login/start",
            "id": 2,
            "params": {
                "type": "apiKey",
                "apiKey": "secret"
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_account_login_chatgpt() -> Result<()> {
    let request = ClientRequest::LoginAccount {
        request_id: RequestId::Integer(3),
        params: v2::LoginAccountParams::Chatgpt {
            codex_streamlined_login: false,
        },
    };
    assert_eq!(
        json!({
            "method": "account/login/start",
            "id": 3,
            "params": {
                "type": "chatgpt"
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_account_login_chatgpt_streamlined() -> Result<()> {
    let request = ClientRequest::LoginAccount {
        request_id: RequestId::Integer(3),
        params: v2::LoginAccountParams::Chatgpt {
            codex_streamlined_login: true,
        },
    };
    assert_eq!(
        json!({
            "method": "account/login/start",
            "id": 3,
            "params": {
                "type": "chatgpt",
                "codexStreamlinedLogin": true
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_account_login_chatgpt_device_code() -> Result<()> {
    let request = ClientRequest::LoginAccount {
        request_id: RequestId::Integer(4),
        params: v2::LoginAccountParams::ChatgptDeviceCode,
    };
    assert_eq!(
        json!({
            "method": "account/login/start",
            "id": 4,
            "params": {
                "type": "chatgptDeviceCode"
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_account_logout() -> Result<()> {
    let request = ClientRequest::LogoutAccount {
        request_id: RequestId::Integer(5),
        params: None,
    };
    assert_eq!(
        json!({
            "method": "account/logout",
            "id": 5,
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_account_login_chatgpt_auth_tokens() -> Result<()> {
    let request = ClientRequest::LoginAccount {
        request_id: RequestId::Integer(6),
        params: v2::LoginAccountParams::ChatgptAuthTokens {
            access_token: "access-token".to_string(),
            chatgpt_account_id: "org-123".to_string(),
            chatgpt_plan_type: Some("business".to_string()),
        },
    };
    assert_eq!(
        json!({
            "method": "account/login/start",
            "id": 6,
            "params": {
                "type": "chatgptAuthTokens",
                "accessToken": "access-token",
                "chatgptAccountId": "org-123",
                "chatgptPlanType": "business"
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_get_account() -> Result<()> {
    let request = ClientRequest::GetAccount {
        request_id: RequestId::Integer(6),
        params: v2::GetAccountParams {
            refresh_token: false,
        },
    };
    assert_eq!(
        json!({
            "method": "account/read",
            "id": 6,
            "params": {
                "refreshToken": false
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn account_serializes_fields_in_camel_case() -> Result<()> {
    let api_key = v2::Account::ApiKey {};
    assert_eq!(
        json!({
            "type": "apiKey",
        }),
        serde_json::to_value(&api_key)?,
    );

    let chatgpt = v2::Account::Chatgpt {
        email: "user@example.com".to_string(),
        plan_type: PlanType::Plus,
    };
    assert_eq!(
        json!({
            "type": "chatgpt",
            "email": "user@example.com",
            "planType": "plus",
        }),
        serde_json::to_value(&chatgpt)?,
    );

    Ok(())
}

#[test]
fn serialize_list_models() -> Result<()> {
    let request = ClientRequest::ModelList {
        request_id: RequestId::Integer(6),
        params: v2::ModelListParams::default(),
    };
    assert_eq!(
        json!({
            "method": "model/list",
            "id": 6,
            "params": {
                "limit": null,
                "cursor": null,
                "includeHidden": null
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_list_agent_types() -> Result<()> {
    let request = ClientRequest::AgentTypeList {
        request_id: RequestId::Integer(7),
        params: v2::AgentTypeListParams::default(),
    };
    assert_eq!(
        json!({
            "method": "agentType/list",
            "id": 7,
            "params": {
                "cwd": null
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_list_thread_providers() -> Result<()> {
    let request = ClientRequest::ThreadProviderList {
        request_id: RequestId::Integer(8),
        params: v2::ThreadProviderListParams::default(),
    };
    assert_eq!(
        json!({
            "method": "threadProvider/list",
            "id": 8,
            "params": {
                "cwd": null
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_thread_start_with_thread_provider() -> Result<()> {
    let params = v2::ThreadStartParams {
        thread_provider: Some("native".to_string()),
        ..v2::ThreadStartParams::default()
    };
    assert_eq!(
        serde_json::to_value(&params)?["threadProvider"],
        json!("native")
    );
    Ok(())
}

#[test]
fn serialize_model_provider_capabilities_read() -> Result<()> {
    let request = ClientRequest::ModelProviderCapabilitiesRead {
        request_id: RequestId::Integer(7),
        params: v2::ModelProviderCapabilitiesReadParams {},
    };
    assert_eq!(
        json!({
            "method": "modelProvider/capabilities/read",
            "id": 7,
            "params": {}
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_list_collaboration_modes() -> Result<()> {
    let request = ClientRequest::CollaborationModeList {
        request_id: RequestId::Integer(7),
        params: v2::CollaborationModeListParams::default(),
    };
    assert_eq!(
        json!({
            "method": "collaborationMode/list",
            "id": 7,
            "params": {}
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_list_apps() -> Result<()> {
    let request = ClientRequest::AppsList {
        request_id: RequestId::Integer(8),
        params: v2::AppsListParams::default(),
    };
    assert_eq!(
        json!({
            "method": "app/list",
            "id": 8,
            "params": {
                "cursor": null,
                "limit": null,
                "threadId": null
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_environment_add() -> Result<()> {
    let request = ClientRequest::EnvironmentAdd {
        request_id: RequestId::Integer(9),
        params: v2::EnvironmentAddParams {
            environment_id: "remote-a".to_string(),
            exec_server_url: "ws://127.0.0.1:8765".to_string(),
        },
    };
    assert_eq!(
        json!({
            "method": "environment/add",
            "id": 9,
            "params": {
                "environmentId": "remote-a",
                "execServerUrl": "ws://127.0.0.1:8765"
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_fs_get_metadata() -> Result<()> {
    let request = ClientRequest::FsGetMetadata {
        request_id: RequestId::Integer(10),
        params: v2::FsGetMetadataParams {
            path: absolute_path("tmp/example"),
        },
    };
    assert_eq!(
        json!({
            "method": "fs/getMetadata",
            "id": 10,
            "params": {
                "path": absolute_path_string("tmp/example")
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_fs_watch() -> Result<()> {
    let request = ClientRequest::FsWatch {
        request_id: RequestId::Integer(10),
        params: v2::FsWatchParams {
            watch_id: "watch-git".to_string(),
            path: absolute_path("tmp/repo/.git"),
        },
    };
    assert_eq!(
        json!({
            "method": "fs/watch",
            "id": 10,
            "params": {
                "watchId": "watch-git",
                "path": absolute_path_string("tmp/repo/.git")
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_list_experimental_features() -> Result<()> {
    let request = ClientRequest::ExperimentalFeatureList {
        request_id: RequestId::Integer(8),
        params: v2::ExperimentalFeatureListParams::default(),
    };
    assert_eq!(
        json!({
            "method": "experimentalFeature/list",
            "id": 8,
            "params": {
                "cursor": null,
                "limit": null
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_thread_background_terminals_clean() -> Result<()> {
    let request = ClientRequest::ThreadBackgroundTerminalsClean {
        request_id: RequestId::Integer(8),
        params: v2::ThreadBackgroundTerminalsCleanParams {
            thread_id: "thr_123".to_string(),
        },
    };
    assert_eq!(
        json!({
            "method": "thread/backgroundTerminals/clean",
            "id": 8,
            "params": {
                "threadId": "thr_123"
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_thread_realtime_start() -> Result<()> {
    let request = ClientRequest::ThreadRealtimeStart {
        request_id: RequestId::Integer(9),
        params: v2::ThreadRealtimeStartParams {
            thread_id: "thr_123".to_string(),
            output_modality: RealtimeOutputModality::Audio,
            prompt: Some(Some("You are on a call".to_string())),
            realtime_session_id: Some("sess_456".to_string()),
            transport: None,
            voice: Some(RealtimeVoice::Marin),
        },
    };
    assert_eq!(
        json!({
            "method": "thread/realtime/start",
            "id": 9,
            "params": {
                "threadId": "thr_123",
                "outputModality": "audio",
                "prompt": "You are on a call",
                "realtimeSessionId": "sess_456",
                "transport": null,
                "voice": "marin"
            }
        }),
        serde_json::to_value(&request)?,
    );
    Ok(())
}

#[test]
fn serialize_thread_realtime_start_prompt_default_and_null() -> Result<()> {
    let default_prompt_request = ClientRequest::ThreadRealtimeStart {
        request_id: RequestId::Integer(9),
        params: v2::ThreadRealtimeStartParams {
            thread_id: "thr_123".to_string(),
            output_modality: RealtimeOutputModality::Audio,
            prompt: None,
            realtime_session_id: None,
            transport: None,
            voice: None,
        },
    };
    assert_eq!(
        json!({
            "method": "thread/realtime/start",
            "id": 9,
            "params": {
                "threadId": "thr_123",
                "outputModality": "audio",
                "realtimeSessionId": null,
                "transport": null,
                "voice": null
            }
        }),
        serde_json::to_value(&default_prompt_request)?,
    );

    let null_prompt_request = ClientRequest::ThreadRealtimeStart {
        request_id: RequestId::Integer(9),
        params: v2::ThreadRealtimeStartParams {
            thread_id: "thr_123".to_string(),
            output_modality: RealtimeOutputModality::Audio,
            prompt: Some(None),
            realtime_session_id: None,
            transport: None,
            voice: None,
        },
    };
    assert_eq!(
        json!({
            "method": "thread/realtime/start",
            "id": 9,
            "params": {
                "threadId": "thr_123",
                "outputModality": "audio",
                "prompt": null,
                "realtimeSessionId": null,
                "transport": null,
                "voice": null
            }
        }),
        serde_json::to_value(&null_prompt_request)?,
    );

    let default_prompt_value = json!({
        "method": "thread/realtime/start",
        "id": 9,
        "params": {
            "threadId": "thr_123",
            "outputModality": "audio",
            "realtimeSessionId": null,
            "transport": null,
            "voice": null
        }
    });
    assert_eq!(
        serde_json::from_value::<ClientRequest>(default_prompt_value)?,
        default_prompt_request,
    );

    let null_prompt_value = json!({
        "method": "thread/realtime/start",
        "id": 9,
        "params": {
            "threadId": "thr_123",
            "outputModality": "audio",
            "prompt": null,
            "realtimeSessionId": null,
            "transport": null,
            "voice": null
        }
    });
    assert_eq!(
        serde_json::from_value::<ClientRequest>(null_prompt_value)?,
        null_prompt_request,
    );

    Ok(())
}

#[test]
fn serialize_thread_status_changed_notification() -> Result<()> {
    let notification =
        ServerNotification::ThreadStatusChanged(v2::ThreadStatusChangedNotification {
            thread_id: "thr_123".to_string(),
            lifecycle_status: v2::ThreadLifecycleStatus::completed(None),
        });
    assert_eq!(
        json!({
            "method": "thread/status/changed",
            "params": {
                "threadId": "thr_123",
                "lifecycleStatus": {
                    "type": "final",
                    "result": {
                        "type": "completed"
                    }
                },
            }
        }),
        serde_json::to_value(&notification)?,
    );
    Ok(())
}

#[test]
fn serialize_thread_realtime_output_audio_delta_notification() -> Result<()> {
    let notification = ServerNotification::ThreadRealtimeOutputAudioDelta(
        v2::ThreadRealtimeOutputAudioDeltaNotification {
            thread_id: "thr_123".to_string(),
            audio: v2::ThreadRealtimeAudioChunk {
                data: "AQID".to_string(),
                sample_rate: 24_000,
                num_channels: 1,
                samples_per_channel: Some(512),
                item_id: None,
            },
        },
    );
    assert_eq!(
        json!({
            "method": "thread/realtime/outputAudio/delta",
            "params": {
                "threadId": "thr_123",
                "audio": {
                    "data": "AQID",
                    "sampleRate": 24000,
                    "numChannels": 1,
                    "samplesPerChannel": 512,
                    "itemId": null
                }
            }
        }),
        serde_json::to_value(&notification)?,
    );
    Ok(())
}

#[test]
fn mock_experimental_method_is_marked_experimental() {
    let request = ClientRequest::MockExperimentalMethod {
        request_id: RequestId::Integer(1),
        params: v2::MockExperimentalMethodParams::default(),
    };
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&request);
    assert_eq!(reason, Some("mock/experimentalMethod"));
}

#[test]
fn environment_add_is_marked_experimental() {
    let request = ClientRequest::EnvironmentAdd {
        request_id: RequestId::Integer(1),
        params: v2::EnvironmentAddParams {
            environment_id: "remote-a".to_string(),
            exec_server_url: "ws://127.0.0.1:8765".to_string(),
        },
    };
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&request);
    assert_eq!(reason, Some("environment/add"));
}

#[test]
fn command_exec_permission_profile_is_marked_experimental() {
    let request = ClientRequest::OneOffCommandExec {
        request_id: RequestId::Integer(1),
        params: v2::CommandExecParams {
            command: vec!["pwd".to_string()],
            process_id: None,
            tty: false,
            stream_stdin: false,
            stream_stdout_stderr: false,
            output_bytes_cap: None,
            disable_output_cap: false,
            disable_timeout: false,
            timeout_ms: None,
            cwd: None,
            env: None,
            size: None,
            sandbox_policy: None,
            permission_profile: Some(v2::PermissionProfile::Disabled),
        },
    };

    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&request);
    assert_eq!(reason, Some("command/exec.permissionProfile"));
}

#[test]
fn thread_realtime_start_is_marked_experimental() {
    let request = ClientRequest::ThreadRealtimeStart {
        request_id: RequestId::Integer(1),
        params: v2::ThreadRealtimeStartParams {
            thread_id: "thr_123".to_string(),
            output_modality: RealtimeOutputModality::Audio,
            prompt: Some(Some("You are on a call".to_string())),
            realtime_session_id: None,
            transport: None,
            voice: None,
        },
    };
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&request);
    assert_eq!(reason, Some("thread/realtime/start"));
}

#[test]
fn thread_goal_methods_are_marked_experimental() {
    let set_request = ClientRequest::ThreadGoalSet {
        request_id: RequestId::Integer(1),
        params: v2::ThreadGoalSetParams {
            thread_id: "thr_123".to_string(),
            objective: Some("ship goal mode".to_string()),
            status: Some(v2::ThreadGoalStatus::Active),
            token_budget: Some(Some(10_000)),
        },
    };
    let get_request = ClientRequest::ThreadGoalGet {
        request_id: RequestId::Integer(2),
        params: v2::ThreadGoalGetParams {
            thread_id: "thr_123".to_string(),
        },
    };
    let clear_request = ClientRequest::ThreadGoalClear {
        request_id: RequestId::Integer(3),
        params: v2::ThreadGoalClearParams {
            thread_id: "thr_123".to_string(),
        },
    };

    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&set_request),
        Some("thread/goal/set")
    );
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&get_request),
        Some("thread/goal/get")
    );
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&clear_request),
        Some("thread/goal/clear")
    );
}

#[test]
fn thread_goal_notifications_are_marked_experimental() {
    let goal = v2::ThreadGoal {
        thread_id: "thr_123".to_string(),
        objective: "ship goal mode".to_string(),
        status: v2::ThreadGoalStatus::Active,
        token_budget: Some(10_000),
        tokens_used: 123,
        time_used_seconds: 45,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_123,
    };
    let updated = ServerNotification::ThreadGoalUpdated(v2::ThreadGoalUpdatedNotification {
        thread_id: "thr_123".to_string(),
        turn_id: None,
        goal,
    });
    let cleared = ServerNotification::ThreadGoalCleared(v2::ThreadGoalClearedNotification {
        thread_id: "thr_123".to_string(),
    });

    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&updated),
        Some("thread/goal/updated")
    );
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&cleared),
        Some("thread/goal/cleared")
    );
}

#[test]
fn thread_realtime_started_notification_is_marked_experimental() {
    let notification =
        ServerNotification::ThreadRealtimeStarted(v2::ThreadRealtimeStartedNotification {
            thread_id: "thr_123".to_string(),
            realtime_session_id: Some("sess_456".to_string()),
            version: RealtimeConversationVersion::V1,
        });
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&notification);
    assert_eq!(reason, Some("thread/realtime/started"));
}

#[test]
fn thread_realtime_output_audio_delta_notification_is_marked_experimental() {
    let notification = ServerNotification::ThreadRealtimeOutputAudioDelta(
        v2::ThreadRealtimeOutputAudioDeltaNotification {
            thread_id: "thr_123".to_string(),
            audio: v2::ThreadRealtimeAudioChunk {
                data: "AQID".to_string(),
                sample_rate: 24_000,
                num_channels: 1,
                samples_per_channel: Some(512),
                item_id: None,
            },
        },
    );
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&notification);
    assert_eq!(reason, Some("thread/realtime/outputAudio/delta"));
}

#[test]
fn command_execution_request_approval_additional_permissions_is_marked_experimental() {
    let params = v2::CommandExecutionRequestApprovalParams {
        thread_id: "thr_123".to_string(),
        turn_id: "turn_123".to_string(),
        item_id: "call_123".to_string(),
        started_at_ms: 0,
        approval_id: None,
        reason: None,
        network_approval_context: None,
        command: Some("cat file".to_string()),
        cwd: None,
        command_actions: None,
        additional_permissions: Some(v2::AdditionalPermissionProfile {
            network: None,
            file_system: Some(v2::AdditionalFileSystemPermissions {
                read: Some(vec![absolute_path("/tmp/allowed")]),
                write: None,
                glob_scan_max_depth: None,
                entries: None,
            }),
        }),
        proposed_execpolicy_amendment: None,
        proposed_network_policy_amendments: None,
        available_decisions: None,
    };
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&params);
    assert_eq!(
        reason,
        Some("item/commandExecution/requestApproval.additionalPermissions")
    );
}
