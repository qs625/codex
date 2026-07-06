use super::*;

#[test]
fn sandbox_policy_round_trips_external_sandbox_network_access() {
    let v2_policy = SandboxPolicy::ExternalSandbox {
        network_access: NetworkAccess::Enabled,
    };

    let core_policy = v2_policy.to_core();
    assert_eq!(
        core_policy,
        protocol::protocol::SandboxPolicy::ExternalSandbox {
            network_access: CoreNetworkAccess::Enabled,
        }
    );

    let back_to_v2 = SandboxPolicy::from(core_policy);
    assert_eq!(back_to_v2, v2_policy);
}

#[test]
fn sandbox_policy_round_trips_read_only_network_access() {
    let v2_policy = SandboxPolicy::ReadOnly {
        network_access: true,
    };

    let core_policy = v2_policy.to_core();
    assert_eq!(
        core_policy,
        protocol::protocol::SandboxPolicy::ReadOnly {
            network_access: true,
        }
    );

    let back_to_v2 = SandboxPolicy::from(core_policy);
    assert_eq!(back_to_v2, v2_policy);
}

#[test]
fn ask_for_approval_granular_round_trips_request_permissions_flag() {
    let v2_policy = AskForApproval::Granular {
        sandbox_approval: true,
        rules: false,
        skill_approval: false,
        request_permissions: true,
        mcp_elicitations: false,
    };

    let core_policy = v2_policy.to_core();
    assert_eq!(
        core_policy,
        CoreAskForApproval::Granular(CoreGranularApprovalConfig {
            sandbox_approval: true,
            rules: false,
            skill_approval: false,
            request_permissions: true,
            mcp_elicitations: false,
        })
    );

    let back_to_v2 = AskForApproval::from(core_policy);
    assert_eq!(back_to_v2, v2_policy);
}

#[test]
fn ask_for_approval_granular_defaults_missing_optional_flags_to_false() {
    let decoded = serde_json::from_value::<AskForApproval>(serde_json::json!({
        "granular": {
            "sandbox_approval": true,
            "rules": false,
            "mcp_elicitations": true,
        }
    }))
    .expect("granular approval policy should deserialize");

    assert_eq!(
        decoded,
        AskForApproval::Granular {
            sandbox_approval: true,
            rules: false,
            skill_approval: false,
            request_permissions: false,
            mcp_elicitations: true,
        }
    );
}

#[test]
fn ask_for_approval_granular_is_marked_experimental() {
    let reason =
        crate::experimental_api::ExperimentalApi::experimental_reason(&AskForApproval::Granular {
            sandbox_approval: true,
            rules: false,
            skill_approval: false,
            request_permissions: false,
            mcp_elicitations: true,
        });

    assert_eq!(reason, Some("askForApproval.granular"));
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&AskForApproval::OnRequest,),
        None
    );
}

#[test]
fn profile_v2_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&ProfileV2 {
        model: None,
        model_provider: None,
        approval_policy: Some(AskForApproval::Granular {
            sandbox_approval: true,
            rules: false,
            skill_approval: false,
            request_permissions: true,
            mcp_elicitations: false,
        }),
        approvals_reviewer: None,
        service_tier: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        web_search: None,
        tools: None,
        chatgpt_base_url: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn config_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&Config {
        model: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_provider: None,
        approval_policy: Some(AskForApproval::Granular {
            sandbox_approval: false,
            rules: true,
            skill_approval: false,
            request_permissions: false,
            mcp_elicitations: true,
        }),
        approvals_reviewer: None,
        sandbox_mode: None,
        sandbox_workspace_write: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search: None,
        tools: None,
        profile: None,
        profiles: HashMap::new(),
        instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        service_tier: None,
        analytics: None,
        apps: None,
        desktop: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn config_approvals_reviewer_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&Config {
        model: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_provider: None,
        approval_policy: None,
        approvals_reviewer: Some(ApprovalsReviewer::AutoReview),
        sandbox_mode: None,
        sandbox_workspace_write: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search: None,
        tools: None,
        profile: None,
        profiles: HashMap::new(),
        instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        service_tier: None,
        analytics: None,
        apps: None,
        desktop: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("config/read.approvalsReviewer"));
}

#[test]
fn config_nested_profile_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&Config {
        model: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_provider: None,
        approval_policy: None,
        approvals_reviewer: None,
        sandbox_mode: None,
        sandbox_workspace_write: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search: None,
        tools: None,
        profile: None,
        profiles: HashMap::from([(
            "default".to_string(),
            ProfileV2 {
                model: None,
                model_provider: None,
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: true,
                    rules: false,
                    skill_approval: false,
                    request_permissions: false,
                    mcp_elicitations: true,
                }),
                approvals_reviewer: None,
                service_tier: None,
                model_reasoning_effort: None,
                model_reasoning_summary: None,
                model_verbosity: None,
                web_search: None,
                tools: None,
                chatgpt_base_url: None,
                additional: HashMap::new(),
            },
        )]),
        instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        service_tier: None,
        analytics: None,
        apps: None,
        desktop: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn config_nested_profile_approvals_reviewer_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&Config {
        model: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_provider: None,
        approval_policy: None,
        approvals_reviewer: None,
        sandbox_mode: None,
        sandbox_workspace_write: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search: None,
        tools: None,
        profile: None,
        profiles: HashMap::from([(
            "default".to_string(),
            ProfileV2 {
                model: None,
                model_provider: None,
                approval_policy: None,
                approvals_reviewer: Some(ApprovalsReviewer::AutoReview),
                service_tier: None,
                model_reasoning_effort: None,
                model_reasoning_summary: None,
                model_verbosity: None,
                web_search: None,
                tools: None,
                chatgpt_base_url: None,
                additional: HashMap::new(),
            },
        )]),
        instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        service_tier: None,
        analytics: None,
        apps: None,
        desktop: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("config/read.approvalsReviewer"));
}

#[test]
fn config_requirements_granular_allowed_approval_policy_is_marked_experimental() {
    let reason =
        crate::experimental_api::ExperimentalApi::experimental_reason(&ConfigRequirements {
            allowed_approval_policies: Some(vec![AskForApproval::Granular {
                sandbox_approval: true,
                rules: true,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: false,
            }]),
            allowed_approvals_reviewers: None,
            allowed_sandbox_modes: None,
            allowed_web_search_modes: None,
            allow_managed_hooks_only: None,
            feature_requirements: None,
            hooks: None,
            enforce_residency: None,
            network: None,
        });

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn client_request_thread_start_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(
        &crate::ClientRequest::ThreadStart {
            request_id: crate::RequestId::Integer(1),
            params: ThreadStartParams {
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: true,
                    rules: false,
                    skill_approval: false,
                    request_permissions: true,
                    mcp_elicitations: false,
                }),
                ..Default::default()
            },
        },
    );

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn client_request_thread_resume_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(
        &crate::ClientRequest::ThreadResume {
            request_id: crate::RequestId::Integer(2),
            params: ThreadResumeParams {
                thread_id: "thr_123".to_string(),
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: false,
                    rules: true,
                    skill_approval: false,
                    request_permissions: false,
                    mcp_elicitations: true,
                }),
                ..Default::default()
            },
        },
    );

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn client_request_thread_fork_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(
        &crate::ClientRequest::ThreadFork {
            request_id: crate::RequestId::Integer(3),
            params: ThreadForkParams {
                thread_id: "thr_456".to_string(),
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: true,
                    rules: false,
                    skill_approval: false,
                    request_permissions: false,
                    mcp_elicitations: true,
                }),
                ..Default::default()
            },
        },
    );

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn client_request_turn_start_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(
        &crate::ClientRequest::TurnStart {
            request_id: crate::RequestId::Integer(4),
            params: TurnStartParams {
                thread_id: "thr_123".to_string(),
                input: Vec::new(),
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: false,
                    rules: true,
                    skill_approval: false,
                    request_permissions: false,
                    mcp_elicitations: true,
                }),
                ..Default::default()
            },
        },
    );

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn mcp_server_elicitation_response_round_trips_mcp_types() {
    let response = mcp_types::ElicitationResponse {
        action: mcp_types::ElicitationAction::Accept,
        content: Some(json!({
            "confirmed": true,
        })),
        meta: None,
    };

    let v2_response = McpServerElicitationRequestResponse::from(response.clone());
    assert_eq!(
        v2_response,
        McpServerElicitationRequestResponse {
            action: McpServerElicitationAction::Accept,
            content: Some(json!({
                "confirmed": true,
            })),
            meta: None,
        }
    );
    assert_eq!(
        mcp_types::ElicitationResponse::from(v2_response),
        response
    );
}

#[test]
fn mcp_server_elicitation_request_from_core_url_request() {
    let request = McpServerElicitationRequest::try_from(CoreElicitationRequest::Url {
        meta: None,
        message: "Finish sign-in".to_string(),
        url: "https://example.com/complete".to_string(),
        elicitation_id: "elicitation-123".to_string(),
    })
    .expect("URL request should convert");

    assert_eq!(
        request,
        McpServerElicitationRequest::Url {
            meta: None,
            message: "Finish sign-in".to_string(),
            url: "https://example.com/complete".to_string(),
            elicitation_id: "elicitation-123".to_string(),
        }
    );
}

#[test]
fn mcp_server_elicitation_request_from_core_form_request() {
    let request = McpServerElicitationRequest::try_from(CoreElicitationRequest::Form {
        meta: None,
        message: "Allow this request?".to_string(),
        requested_schema: json!({
            "type": "object",
            "properties": {
                "confirmed": {
                    "type": "boolean",
                }
            },
            "required": ["confirmed"],
        }),
    })
    .expect("form request should convert");

    let expected_schema: McpElicitationSchema = serde_json::from_value(json!({
        "type": "object",
        "properties": {
            "confirmed": {
                "type": "boolean",
            }
        },
        "required": ["confirmed"],
    }))
    .expect("expected schema should deserialize");

    assert_eq!(
        request,
        McpServerElicitationRequest::Form {
            meta: None,
            message: "Allow this request?".to_string(),
            requested_schema: expected_schema,
        }
    );
}

#[test]
fn mcp_elicitation_schema_matches_mcp_2025_11_25_primitives() {
    let schema: McpElicitationSchema = serde_json::from_value(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "email": {
                "type": "string",
                "title": "Email",
                "description": "Work email address",
                "format": "email",
                "default": "dev@example.com",
            },
            "count": {
                "type": "integer",
                "title": "Count",
                "description": "How many items to create",
                "minimum": 1,
                "maximum": 5,
                "default": 3,
            },
            "confirmed": {
                "type": "boolean",
                "title": "Confirm",
                "description": "Approve the pending action",
                "default": true,
            },
            "legacyChoice": {
                "type": "string",
                "title": "Action",
                "description": "Legacy titled enum form",
                "enum": ["allow", "deny"],
                "enumNames": ["Allow", "Deny"],
                "default": "allow",
            },
        },
        "required": ["email", "confirmed"],
    }))
    .expect("schema should deserialize");

    assert_eq!(
        schema,
        McpElicitationSchema {
            schema_uri: Some("https://json-schema.org/draft/2020-12/schema".to_string()),
            type_: McpElicitationObjectType::Object,
            properties: BTreeMap::from([
                (
                    "confirmed".to_string(),
                    McpElicitationPrimitiveSchema::Boolean(McpElicitationBooleanSchema {
                        type_: McpElicitationBooleanType::Boolean,
                        title: Some("Confirm".to_string()),
                        description: Some("Approve the pending action".to_string()),
                        default: Some(true),
                    }),
                ),
                (
                    "count".to_string(),
                    McpElicitationPrimitiveSchema::Number(McpElicitationNumberSchema {
                        type_: McpElicitationNumberType::Integer,
                        title: Some("Count".to_string()),
                        description: Some("How many items to create".to_string()),
                        minimum: Some(1.0),
                        maximum: Some(5.0),
                        default: Some(3.0),
                    }),
                ),
                (
                    "email".to_string(),
                    McpElicitationPrimitiveSchema::String(McpElicitationStringSchema {
                        type_: McpElicitationStringType::String,
                        title: Some("Email".to_string()),
                        description: Some("Work email address".to_string()),
                        min_length: None,
                        max_length: None,
                        format: Some(McpElicitationStringFormat::Email),
                        default: Some("dev@example.com".to_string()),
                    }),
                ),
                (
                    "legacyChoice".to_string(),
                    McpElicitationPrimitiveSchema::Enum(McpElicitationEnumSchema::Legacy(
                        McpElicitationLegacyTitledEnumSchema {
                            type_: McpElicitationStringType::String,
                            title: Some("Action".to_string()),
                            description: Some("Legacy titled enum form".to_string()),
                            enum_: vec!["allow".to_string(), "deny".to_string()],
                            enum_names: Some(vec!["Allow".to_string(), "Deny".to_string(),]),
                            default: Some("allow".to_string()),
                        },
                    )),
                ),
            ]),
            required: Some(vec!["email".to_string(), "confirmed".to_string()]),
        }
    );
}

#[test]
fn mcp_server_elicitation_request_rejects_null_core_form_schema() {
    let result = McpServerElicitationRequest::try_from(CoreElicitationRequest::Form {
        meta: Some(json!({
            "persist": "session",
        })),
        message: "Allow this request?".to_string(),
        requested_schema: JsonValue::Null,
    });

    assert!(result.is_err());
}

#[test]
fn mcp_server_elicitation_request_rejects_invalid_core_form_schema() {
    let result = McpServerElicitationRequest::try_from(CoreElicitationRequest::Form {
        meta: None,
        message: "Allow this request?".to_string(),
        requested_schema: json!({
            "type": "object",
            "properties": {
                "confirmed": {
                    "type": "object",
                }
            },
        }),
    });

    assert!(result.is_err());
}

#[test]
fn mcp_server_elicitation_response_serializes_nullable_content() {
    let response = McpServerElicitationRequestResponse {
        action: McpServerElicitationAction::Decline,
        content: None,
        meta: None,
    };

    assert_eq!(
        serde_json::to_value(response).expect("response should serialize"),
        json!({
            "action": "decline",
            "content": null,
            "_meta": null,
        })
    );
}

#[test]
fn sandbox_policy_round_trips_workspace_write_access() {
    let v2_policy = SandboxPolicy::WorkspaceWrite {
        writable_roots: vec![],
        network_access: true,
        exclude_tmpdir_env_var: false,
        exclude_slash_tmp: false,
    };

    let core_policy = v2_policy.to_core();
    assert_eq!(
        core_policy,
        protocol::protocol::SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: true,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        }
    );

    let back_to_v2 = SandboxPolicy::from(core_policy);
    assert_eq!(back_to_v2, v2_policy);
}

#[test]
fn sandbox_policy_deserializes_legacy_read_only_full_access_field() {
    let policy = serde_json::from_value::<SandboxPolicy>(json!({
        "type": "readOnly",
        "access": {
            "type": "fullAccess"
        },
        "networkAccess": true
    }))
    .expect("read-only policy should ignore legacy fullAccess field");
    assert_eq!(
        policy,
        SandboxPolicy::ReadOnly {
            network_access: true
        }
    );
}

#[test]
fn sandbox_policy_deserializes_legacy_workspace_write_full_access_field() {
    let writable_root = absolute_path("/workspace");
    let policy = serde_json::from_value::<SandboxPolicy>(json!({
        "type": "workspaceWrite",
        "writableRoots": [writable_root],
        "readOnlyAccess": {
            "type": "fullAccess"
        },
        "networkAccess": true,
        "excludeTmpdirEnvVar": true,
        "excludeSlashTmp": true
    }))
    .expect("workspace-write policy should ignore legacy fullAccess field");
    assert_eq!(
        policy,
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![absolute_path("/workspace")],
            network_access: true,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        }
    );
}

#[test]
fn sandbox_policy_rejects_legacy_read_only_restricted_access_field() {
    let err = serde_json::from_value::<SandboxPolicy>(json!({
        "type": "readOnly",
        "access": {
            "type": "restricted",
            "includePlatformDefaults": false,
            "readableRoots": []
        }
    }))
    .expect_err("read-only policy should reject removed restricted access field");
    assert!(err.to_string().contains("readOnly.access"));
}

#[test]
fn sandbox_policy_rejects_legacy_workspace_write_restricted_read_access_field() {
    let err = serde_json::from_value::<SandboxPolicy>(json!({
        "type": "workspaceWrite",
        "writableRoots": [],
        "readOnlyAccess": {
            "type": "restricted",
            "includePlatformDefaults": false,
            "readableRoots": []
        },
        "networkAccess": false,
        "excludeTmpdirEnvVar": false,
        "excludeSlashTmp": false
    }))
    .expect_err("workspace-write policy should reject removed restricted readOnlyAccess field");
    assert!(err.to_string().contains("workspaceWrite.readOnlyAccess"));
}

#[test]
fn automatic_approval_review_deserializes_aborted_status() {
    let review: GuardianApprovalReview = serde_json::from_value(json!({
        "status": "aborted",
        "riskLevel": null,
        "userAuthorization": null,
        "rationale": null
    }))
    .expect("aborted automatic review should deserialize");
    assert_eq!(
        review,
        GuardianApprovalReview {
            status: GuardianApprovalReviewStatus::Aborted,
            risk_level: None,
            user_authorization: None,
            rationale: None,
        }
    );
}

#[test]
fn guardian_approval_review_action_round_trips_command_shape() {
    let value = json!({
        "type": "command",
        "source": "shell",
        "command": "rm -rf /tmp/example.sqlite",
        "cwd": absolute_path_string("tmp"),
    });
    let action: GuardianApprovalReviewAction =
        serde_json::from_value(value.clone()).expect("guardian review action");

    assert_eq!(
        action,
        GuardianApprovalReviewAction::Command {
            source: GuardianCommandSource::Shell,
            command: "rm -rf /tmp/example.sqlite".to_string(),
            cwd: absolute_path("tmp"),
        }
    );
    assert_eq!(
        serde_json::to_value(&action).expect("serialize guardian review action"),
        value
    );
}

#[test]
fn network_requirements_deserializes_legacy_fields() {
    let requirements: NetworkRequirements = serde_json::from_value(json!({
        "allowedDomains": ["api.openai.com"],
        "deniedDomains": ["blocked.example.com"],
        "allowUnixSockets": ["/tmp/proxy.sock"]
    }))
    .expect("legacy network requirements should deserialize");

    assert_eq!(
        requirements,
        NetworkRequirements {
            enabled: None,
            http_port: None,
            socks_port: None,
            allow_upstream_proxy: None,
            dangerously_allow_non_loopback_proxy: None,
            dangerously_allow_all_unix_sockets: None,
            domains: None,
            managed_allowed_domains_only: None,
            allowed_domains: Some(vec!["api.openai.com".to_string()]),
            denied_domains: Some(vec!["blocked.example.com".to_string()]),
            unix_sockets: None,
            allow_unix_sockets: Some(vec!["/tmp/proxy.sock".to_string()]),
            allow_local_binding: None,
        }
    );
}

#[test]
fn network_requirements_serializes_canonical_and_legacy_fields() {
    let requirements = NetworkRequirements {
        enabled: Some(true),
        http_port: Some(8080),
        socks_port: Some(1080),
        allow_upstream_proxy: Some(false),
        dangerously_allow_non_loopback_proxy: Some(false),
        dangerously_allow_all_unix_sockets: Some(true),
        domains: Some(BTreeMap::from([
            ("api.openai.com".to_string(), NetworkDomainPermission::Allow),
            (
                "blocked.example.com".to_string(),
                NetworkDomainPermission::Deny,
            ),
        ])),
        managed_allowed_domains_only: Some(true),
        allowed_domains: Some(vec!["api.openai.com".to_string()]),
        denied_domains: Some(vec!["blocked.example.com".to_string()]),
        unix_sockets: Some(BTreeMap::from([
            (
                "/tmp/proxy.sock".to_string(),
                NetworkUnixSocketPermission::Allow,
            ),
            (
                "/tmp/ignored.sock".to_string(),
                NetworkUnixSocketPermission::None,
            ),
        ])),
        allow_unix_sockets: Some(vec!["/tmp/proxy.sock".to_string()]),
        allow_local_binding: Some(true),
    };

    assert_eq!(
        serde_json::to_value(requirements).expect("network requirements should serialize"),
        json!({
            "enabled": true,
            "httpPort": 8080,
            "socksPort": 1080,
            "allowUpstreamProxy": false,
            "dangerouslyAllowNonLoopbackProxy": false,
            "dangerouslyAllowAllUnixSockets": true,
            "domains": {
                "api.openai.com": "allow",
                "blocked.example.com": "deny"
            },
            "managedAllowedDomainsOnly": true,
            "allowedDomains": ["api.openai.com"],
            "deniedDomains": ["blocked.example.com"],
            "unixSockets": {
                "/tmp/ignored.sock": "none",
                "/tmp/proxy.sock": "allow"
            },
            "allowUnixSockets": ["/tmp/proxy.sock"],
            "allowLocalBinding": true
        })
    );
}
