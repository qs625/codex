use super::*;
use anyhow::Result;
use permissions_service_api::Policy;
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn response_input_message_conversion_preserves_phase() {
    let item = ResponseItem::from(ResponseInputItem::Message {
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "still working".to_string(),
        }],
        phase: Some(MessagePhase::Commentary),
    });

    assert_eq!(
        item,
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: "still working".to_string(),
            }],
            phase: Some(MessagePhase::Commentary),
        }
    );
}

#[test]
fn sandbox_permissions_helpers_match_documented_semantics() {
    let cases = [
        (SandboxPermissions::UseDefault, false, false, false),
        (SandboxPermissions::RequireEscalated, true, true, false),
        (
            SandboxPermissions::WithAdditionalPermissions,
            false,
            true,
            true,
        ),
    ];

    for (
        sandbox_permissions,
        requires_escalated_permissions,
        requests_sandbox_override,
        uses_additional_permissions,
    ) in cases
    {
        assert_eq!(
            sandbox_permissions.requires_escalated_permissions(),
            requires_escalated_permissions
        );
        assert_eq!(
            sandbox_permissions.requests_sandbox_override(),
            requests_sandbox_override
        );
        assert_eq!(
            sandbox_permissions.uses_additional_permissions(),
            uses_additional_permissions
        );
    }
}

#[test]
fn convert_mcp_content_to_items_preserves_data_urls() {
    let contents = vec![serde_json::json!({
        "type": "image",
        "data": "data:image/png;base64,Zm9v",
        "mimeType": "image/png",
    })];

    let items = convert_mcp_content_to_items(&contents).expect("expected image items");
    assert_eq!(
        items,
        vec![FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,Zm9v".to_string(),
            detail: Some(DEFAULT_IMAGE_DETAIL),
        }]
    );
}

#[test]
fn response_item_parses_image_generation_call() {
    let item = serde_json::from_value::<ResponseItem>(serde_json::json!({
        "id": "ig_123",
        "type": "image_generation_call",
        "status": "completed",
        "revised_prompt": "A small blue square",
        "result": "Zm9v",
    }))
    .expect("image generation item should deserialize");

    assert_eq!(
        item,
        ResponseItem::ImageGenerationCall {
            id: "ig_123".to_string(),
            status: "completed".to_string(),
            revised_prompt: Some("A small blue square".to_string()),
            result: "Zm9v".to_string(),
        }
    );
}

#[test]
fn response_item_parses_image_generation_call_without_revised_prompt() {
    let item = serde_json::from_value::<ResponseItem>(serde_json::json!({
        "id": "ig_123",
        "type": "image_generation_call",
        "status": "completed",
        "result": "Zm9v",
    }))
    .expect("image generation item should deserialize");

    assert_eq!(
        item,
        ResponseItem::ImageGenerationCall {
            id: "ig_123".to_string(),
            status: "completed".to_string(),
            revised_prompt: None,
            result: "Zm9v".to_string(),
        }
    );
}

#[test]
fn additional_permission_profile_is_empty_when_all_fields_are_none() {
    assert_eq!(AdditionalPermissionProfile::default().is_empty(), true);
}

#[test]
fn additional_permission_profile_is_not_empty_when_field_is_present_but_nested_empty() {
    let permission_profile = AdditionalPermissionProfile {
        network: Some(NetworkPermissions { enabled: None }),
        file_system: None,
    };
    assert_eq!(permission_profile.is_empty(), false);
}

#[test]
fn permission_profile_round_trip_preserves_glob_scan_max_depth() {
    let mut file_system_sandbox_policy =
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern {
                pattern: "**/*.env".to_string(),
            },
            access: FileSystemAccessMode::None,
        }]);
    file_system_sandbox_policy.glob_scan_max_depth = Some(2);

    let permission_profile = PermissionProfile::from_runtime_permissions(
        &file_system_sandbox_policy,
        NetworkSandboxPolicy::Restricted,
    );

    assert_eq!(
        permission_profile.file_system_sandbox_policy(),
        file_system_sandbox_policy
    );
}

#[test]
fn permission_profile_deserializes_legacy_rollout_shape() -> Result<()> {
    let legacy = serde_json::json!({
        "network": {
            "enabled": true,
        },
        "file_system": {
            "entries": [{
                "path": {
                    "type": "special",
                    "value": {
                        "kind": "root",
                    },
                },
                "access": "write",
            }],
            "glob_scan_max_depth": 2,
        },
    });

    let permission_profile: PermissionProfile = serde_json::from_value(legacy)?;

    assert_eq!(
        permission_profile,
        PermissionProfile::Managed {
            file_system: ManagedFileSystemPermissions::Restricted {
                entries: vec![FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::Root,
                    },
                    access: FileSystemAccessMode::Write,
                }],
                glob_scan_max_depth: NonZeroUsize::new(2),
            },
            network: NetworkSandboxPolicy::Enabled,
        }
    );
    Ok(())
}

#[test]
fn permission_profile_presets_match_legacy_defaults() {
    assert_eq!(
        PermissionProfile::read_only(),
        PermissionProfile::from_legacy_sandbox_policy(&SandboxPolicy::new_read_only_policy())
    );
    assert_eq!(
        PermissionProfile::workspace_write(),
        PermissionProfile::from_legacy_sandbox_policy(&SandboxPolicy::new_workspace_write_policy())
    );
}

#[test]
fn permission_profile_round_trip_preserves_disabled_sandbox() -> Result<()> {
    let cwd = tempdir()?;
    let permission_profile =
        PermissionProfile::from_legacy_sandbox_policy(&SandboxPolicy::DangerFullAccess);

    assert_eq!(permission_profile, PermissionProfile::Disabled);
    assert_eq!(
        permission_profile.to_legacy_sandbox_policy(cwd.path())?,
        SandboxPolicy::DangerFullAccess
    );
    assert_eq!(
        permission_profile.to_runtime_permissions(),
        (
            FileSystemSandboxPolicy::unrestricted(),
            NetworkSandboxPolicy::Enabled
        )
    );
    Ok(())
}

#[test]
fn disabled_permission_profile_ignores_runtime_network_policy() {
    let permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
        SandboxEnforcement::Disabled,
        &FileSystemSandboxPolicy::unrestricted(),
        NetworkSandboxPolicy::Restricted,
    );

    assert_eq!(permission_profile, PermissionProfile::Disabled);
}

#[test]
fn permission_profile_from_runtime_permissions_preserves_external_sandbox() {
    let permission_profile = PermissionProfile::from_runtime_permissions(
        &FileSystemSandboxPolicy::external_sandbox(),
        NetworkSandboxPolicy::Restricted,
    );

    assert_eq!(
        permission_profile,
        PermissionProfile::External {
            network: NetworkSandboxPolicy::Restricted,
        }
    );
    assert_eq!(
        PermissionProfile::from_runtime_permissions_with_enforcement(
            SandboxEnforcement::Managed,
            &FileSystemSandboxPolicy::external_sandbox(),
            NetworkSandboxPolicy::Restricted,
        ),
        permission_profile,
    );
}

#[test]
fn permission_profile_from_runtime_permissions_preserves_unrestricted_managed_network() {
    let permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
        SandboxEnforcement::External,
        &FileSystemSandboxPolicy::unrestricted(),
        NetworkSandboxPolicy::Restricted,
    );

    assert_eq!(
        permission_profile,
        PermissionProfile::Managed {
            file_system: ManagedFileSystemPermissions::Unrestricted,
            network: NetworkSandboxPolicy::Restricted,
        },
        "the legacy ExternalSandbox projection must not hide a split unrestricted filesystem policy"
    );
    assert_eq!(
        permission_profile.to_runtime_permissions(),
        (
            FileSystemSandboxPolicy::unrestricted(),
            NetworkSandboxPolicy::Restricted,
        )
    );
}

#[test]
fn permission_profile_round_trip_preserves_external_sandbox() -> Result<()> {
    let cwd = tempdir()?;
    let sandbox_policy = SandboxPolicy::ExternalSandbox {
        network_access: crate::protocol::NetworkAccess::Restricted,
    };
    let permission_profile = PermissionProfile::from_legacy_sandbox_policy(&sandbox_policy);

    assert_eq!(
        permission_profile,
        PermissionProfile::External {
            network: NetworkSandboxPolicy::Restricted,
        }
    );
    assert_eq!(
        permission_profile.to_legacy_sandbox_policy(cwd.path())?,
        sandbox_policy
    );
    assert_eq!(
        permission_profile.to_runtime_permissions(),
        (
            FileSystemSandboxPolicy::external_sandbox(),
            NetworkSandboxPolicy::Restricted
        )
    );
    Ok(())
}

#[test]
fn file_system_permissions_with_glob_scan_depth_uses_canonical_json() -> Result<()> {
    let path = AbsolutePathBuf::try_from(PathBuf::from(if cfg!(windows) {
        r"C:\tmp\allowed"
    } else {
        "/tmp/allowed"
    }))
    .expect("absolute path");
    let file_system_permissions = FileSystemPermissions {
        entries: vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path { path },
            access: FileSystemAccessMode::Read,
        }],
        glob_scan_max_depth: NonZeroUsize::new(2),
    };

    let serialized = serde_json::to_value(&file_system_permissions)?;

    assert_eq!(serialized.get("read"), None);
    assert_eq!(serialized.get("write"), None);
    assert_eq!(
        serialized.get("glob_scan_max_depth"),
        Some(&serde_json::json!(2))
    );
    assert!(serialized.get("entries").is_some());
    assert_eq!(
        serde_json::from_value::<FileSystemPermissions>(serialized)?,
        file_system_permissions
    );
    Ok(())
}

#[test]
fn file_system_permissions_rejects_zero_glob_scan_depth() {
    serde_json::from_value::<FileSystemPermissions>(serde_json::json!({
        "entries": [],
        "glob_scan_max_depth": 0,
    }))
    .expect_err("zero glob scan depth should fail deserialization");
}

#[test]
fn convert_mcp_content_to_items_builds_data_urls_when_missing_prefix() {
    let contents = vec![serde_json::json!({
        "type": "image",
        "data": "Zm9v",
        "mimeType": "image/png",
    })];

    let items = convert_mcp_content_to_items(&contents).expect("expected image items");
    assert_eq!(
        items,
        vec![FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,Zm9v".to_string(),
            detail: Some(DEFAULT_IMAGE_DETAIL),
        }]
    );
}

#[test]
fn convert_mcp_content_to_items_returns_none_without_images() {
    let contents = vec![serde_json::json!({
        "type": "text",
        "text": "hello",
    })];

    assert_eq!(convert_mcp_content_to_items(&contents), None);
}

#[test]
fn function_call_output_content_items_to_text_joins_text_segments() {
    let content_items = vec![
        FunctionCallOutputContentItem::InputText {
            text: "line 1".to_string(),
        },
        FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,AAA".to_string(),
            detail: Some(DEFAULT_IMAGE_DETAIL),
        },
        FunctionCallOutputContentItem::InputText {
            text: "line 2".to_string(),
        },
    ];

    let text = function_call_output_content_items_to_text(&content_items);
    assert_eq!(text, Some("line 1\nline 2".to_string()));
}

#[test]
fn function_call_output_content_items_to_text_ignores_blank_text_and_images() {
    let content_items = vec![
        FunctionCallOutputContentItem::InputText {
            text: "   ".to_string(),
        },
        FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,AAA".to_string(),
            detail: Some(DEFAULT_IMAGE_DETAIL),
        },
    ];

    let text = function_call_output_content_items_to_text(&content_items);
    assert_eq!(text, None);
}

#[test]
fn function_call_output_body_to_text_returns_plain_text_content() {
    let body = FunctionCallOutputBody::Text("ok".to_string());
    let text = body.to_text();
    assert_eq!(text, Some("ok".to_string()));
}

#[test]
fn function_call_output_body_to_text_uses_content_item_fallback() {
    let body = FunctionCallOutputBody::ContentItems(vec![
        FunctionCallOutputContentItem::InputText {
            text: "line 1".to_string(),
        },
        FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,AAA".to_string(),
            detail: Some(DEFAULT_IMAGE_DETAIL),
        },
    ]);

    let text = body.to_text();
    assert_eq!(text, Some("line 1".to_string()));
}

#[test]
fn function_call_deserializes_optional_namespace() {
    let item: ResponseItem = serde_json::from_value(serde_json::json!({
        "type": "function_call",
        "name": "mcp__codex_apps__gmail_get_recent_emails",
        "namespace": "mcp__codex_apps__gmail",
        "arguments": "{\"top_k\":5}",
        "call_id": "call-1",
    }))
    .expect("function_call should deserialize");

    assert_eq!(
        item,
        ResponseItem::FunctionCall {
            id: None,
            name: "mcp__codex_apps__gmail_get_recent_emails".to_string(),
            namespace: Some("mcp__codex_apps__gmail".to_string()),
            arguments: "{\"top_k\":5}".to_string(),
            call_id: "call-1".to_string(),
        }
    );
}

#[test]
fn render_command_prefix_list_sorts_by_len_then_total_len_then_alphabetical() {
    let prefixes = vec![
        vec!["b".to_string(), "zz".to_string()],
        vec!["aa".to_string()],
        vec!["b".to_string()],
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
        vec!["a".to_string()],
        vec!["b".to_string(), "a".to_string()],
    ];

    let output = format_allow_prefixes(prefixes).expect("rendered list");
    assert_eq!(
        output,
        r#"- ["a"]
- ["b"]
- ["aa"]
- ["b", "a"]
- ["b", "zz"]
- ["a", "b", "c"]"#
            .to_string(),
    );
}

#[test]
fn render_command_prefix_list_limits_output_to_max_prefixes() {
    let prefixes = (0..(MAX_RENDERED_PREFIXES + 5))
        .map(|i| vec![format!("{i:03}")])
        .collect::<Vec<_>>();

    let output = format_allow_prefixes(prefixes).expect("rendered list");
    assert_eq!(output.ends_with(TRUNCATED_MARKER), true);
    eprintln!("output: {output}");
    assert_eq!(output.lines().count(), MAX_RENDERED_PREFIXES + 1);
}

#[test]
fn format_allow_prefixes_limits_output() {
    let mut exec_policy = Policy::empty();
    for i in 0..200 {
        exec_policy
            .add_prefix_rule(
                &[format!("tool-{i:03}"), "x".repeat(500)],
                permissions_service_api::Decision::Allow,
            )
            .expect("add rule");
    }

    let output =
        format_allow_prefixes(exec_policy.get_allowed_prefixes()).expect("formatted prefixes");
    assert!(
        output.len() <= MAX_ALLOW_PREFIX_TEXT_BYTES + TRUNCATED_MARKER.len(),
        "output length exceeds expected limit: {output}",
    );
}

#[test]
fn serializes_success_as_plain_string() -> Result<()> {
    let item = ResponseInputItem::FunctionCallOutput {
        call_id: "call1".into(),
        output: FunctionCallOutputPayload::from_text("ok".into()),
    };

    let json = serde_json::to_string(&item)?;
    let v: serde_json::Value = serde_json::from_str(&json)?;

    // Success case -> output should be a plain string
    assert_eq!(v.get("output").unwrap().as_str().unwrap(), "ok");
    Ok(())
}

#[test]
fn serializes_failure_as_string() -> Result<()> {
    let item = ResponseInputItem::FunctionCallOutput {
        call_id: "call1".into(),
        output: FunctionCallOutputPayload {
            body: FunctionCallOutputBody::Text("bad".into()),
            success: Some(false),
        },
    };

    let json = serde_json::to_string(&item)?;
    let v: serde_json::Value = serde_json::from_str(&json)?;

    assert_eq!(v.get("output").unwrap().as_str().unwrap(), "bad");
    Ok(())
}

#[test]
fn serializes_image_outputs_as_array() -> Result<()> {
    let call_tool_result = CallToolResult {
        content: vec![
            serde_json::json!({"type":"text","text":"caption"}),
            serde_json::json!({"type":"image","data":"BASE64","mimeType":"image/png"}),
        ],
        structured_content: None,
        is_error: Some(false),
        meta: None,
    };

    let payload = call_tool_result.into_function_call_output_payload();
    assert_eq!(payload.success, Some(true));
    let Some(items) = payload.content_items() else {
        panic!("expected content items");
    };
    let items = items.to_vec();
    assert_eq!(
        items,
        vec![
            FunctionCallOutputContentItem::InputText {
                text: "caption".into(),
            },
            FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,BASE64".into(),
                detail: Some(DEFAULT_IMAGE_DETAIL),
            },
        ]
    );

    let item = ResponseInputItem::FunctionCallOutput {
        call_id: "call1".into(),
        output: payload,
    };

    let json = serde_json::to_string(&item)?;
    let v: serde_json::Value = serde_json::from_str(&json)?;

    let output = v.get("output").expect("output field");
    assert!(output.is_array(), "expected array output");

    Ok(())
}

#[test]
fn serializes_custom_tool_image_outputs_as_array() -> Result<()> {
    let item = ResponseInputItem::CustomToolCallOutput {
        call_id: "call1".into(),
        name: None,
        output: FunctionCallOutputPayload::from_content_items(vec![
            FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,BASE64".into(),
                detail: Some(DEFAULT_IMAGE_DETAIL),
            },
        ]),
    };

    let json = serde_json::to_string(&item)?;
    let v: serde_json::Value = serde_json::from_str(&json)?;

    let output = v.get("output").expect("output field");
    assert!(output.is_array(), "expected array output");

    Ok(())
}

#[test]
fn preserves_existing_image_data_urls() -> Result<()> {
    let call_tool_result = CallToolResult {
        content: vec![serde_json::json!({
            "type": "image",
            "data": "data:image/png;base64,BASE64",
            "mimeType": "image/png"
        })],
        structured_content: None,
        is_error: Some(false),
        meta: None,
    };

    let payload = call_tool_result.into_function_call_output_payload();
    let Some(items) = payload.content_items() else {
        panic!("expected content items");
    };
    let items = items.to_vec();
    assert_eq!(
        items,
        vec![FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,BASE64".into(),
            detail: Some(DEFAULT_IMAGE_DETAIL),
        }]
    );

    Ok(())
}

#[test]
fn preserves_original_detail_metadata_on_mcp_images() -> Result<()> {
    let call_tool_result = CallToolResult {
        content: vec![serde_json::json!({
            "type": "image",
            "data": "BASE64",
            "mimeType": "image/png",
            "_meta": {
                "codex/imageDetail": "original",
            },
        })],
        structured_content: None,
        is_error: Some(false),
        meta: None,
    };

    let payload = call_tool_result.into_function_call_output_payload();
    let Some(items) = payload.content_items() else {
        panic!("expected content items");
    };
    let items = items.to_vec();
    assert_eq!(
        items,
        vec![FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,BASE64".into(),
            detail: Some(ImageDetail::Original),
        }]
    );

    Ok(())
}

#[test]
fn preserves_standard_detail_metadata_on_mcp_images() -> Result<()> {
    let call_tool_result = CallToolResult {
        content: vec![serde_json::json!({
            "type": "image",
            "data": "BASE64",
            "mimeType": "image/png",
            "_meta": {
                "codex/imageDetail": "high",
            },
        })],
        structured_content: None,
        is_error: Some(false),
        meta: None,
    };

    let payload = call_tool_result.into_function_call_output_payload();
    let Some(items) = payload.content_items() else {
        panic!("expected content items");
    };
    let items = items.to_vec();
    assert_eq!(
        items,
        vec![FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,BASE64".into(),
            detail: Some(ImageDetail::High),
        }]
    );

    Ok(())
}

#[test]
fn deserializes_array_payload_into_items() -> Result<()> {
    let json = r#"[
            {"type": "input_text", "text": "note"},
            {"type": "input_image", "image_url": "data:image/png;base64,XYZ"}
        ]"#;

    let payload: FunctionCallOutputPayload = serde_json::from_str(json)?;

    assert_eq!(payload.success, None);
    let expected_items = vec![
        FunctionCallOutputContentItem::InputText {
            text: "note".into(),
        },
        FunctionCallOutputContentItem::InputImage {
            image_url: "data:image/png;base64,XYZ".into(),
            detail: None,
        },
    ];
    assert_eq!(
        payload.body,
        FunctionCallOutputBody::ContentItems(expected_items.clone())
    );
    assert_eq!(
        serde_json::to_string(&payload)?,
        serde_json::to_string(&expected_items)?
    );

    Ok(())
}

#[test]
fn deserializes_compaction_alias() -> Result<()> {
    let json = r#"{"type":"compaction_summary","encrypted_content":"abc"}"#;

    let item: ResponseItem = serde_json::from_str(json)?;

    assert_eq!(
        item,
        ResponseItem::Compaction {
            encrypted_content: "abc".into(),
        }
    );
    Ok(())
}

#[test]
fn deserializes_context_compaction() -> Result<()> {
    let json = r#"{"type":"context_compaction","encrypted_content":"abc"}"#;

    let item: ResponseItem = serde_json::from_str(json)?;

    assert_eq!(
        item,
        ResponseItem::ContextCompaction {
            encrypted_content: Some("abc".into()),
        }
    );
    Ok(())
}

#[test]
fn serializes_context_compaction_trigger_without_payload() -> Result<()> {
    let item = ResponseItem::ContextCompaction {
        encrypted_content: None,
    };

    assert_eq!(
        serde_json::to_value(item)?,
        serde_json::json!({
            "type": "context_compaction",
        })
    );
    Ok(())
}

#[test]
fn deserializes_legacy_ghost_snapshot_as_other() -> Result<()> {
    let json = r#"{
            "type":"ghost_snapshot",
            "ghost_commit":{
                "id":"ghost-1",
                "parent":null,
                "preexisting_untracked_files":[],
                "preexisting_untracked_dirs":[]
            }
        }"#;

    let item: ResponseItem = serde_json::from_str(json)?;

    assert_eq!(item, ResponseItem::Other);
    Ok(())
}

#[test]
fn roundtrips_web_search_call_actions() -> Result<()> {
    let cases = vec![
        (
            r#"{
                    "type": "web_search_call",
                    "status": "completed",
                    "action": {
                        "type": "search",
                        "query": "weather seattle",
                        "queries": ["weather seattle", "seattle weather now"]
                    }
                }"#,
            None,
            Some(WebSearchAction::Search {
                query: Some("weather seattle".into()),
                queries: Some(vec!["weather seattle".into(), "seattle weather now".into()]),
            }),
            Some("completed".into()),
            true,
        ),
        (
            r#"{
                    "type": "web_search_call",
                    "status": "open",
                    "action": {
                        "type": "open_page",
                        "url": "https://example.com"
                    }
                }"#,
            None,
            Some(WebSearchAction::OpenPage {
                url: Some("https://example.com".into()),
            }),
            Some("open".into()),
            true,
        ),
        (
            r#"{
                    "type": "web_search_call",
                    "status": "in_progress",
                    "action": {
                        "type": "find_in_page",
                        "url": "https://example.com/docs",
                        "pattern": "installation"
                    }
                }"#,
            None,
            Some(WebSearchAction::FindInPage {
                url: Some("https://example.com/docs".into()),
                pattern: Some("installation".into()),
            }),
            Some("in_progress".into()),
            true,
        ),
        (
            r#"{
                    "type": "web_search_call",
                    "status": "in_progress",
                    "id": "ws_partial"
                }"#,
            Some("ws_partial".into()),
            None,
            Some("in_progress".into()),
            false,
        ),
    ];

    for (json_literal, expected_id, expected_action, expected_status, expect_roundtrip) in cases {
        let parsed: ResponseItem = serde_json::from_str(json_literal)?;
        let expected = ResponseItem::WebSearchCall {
            id: expected_id.clone(),
            status: expected_status.clone(),
            action: expected_action.clone(),
        };
        assert_eq!(parsed, expected);

        let serialized = serde_json::to_value(&parsed)?;
        let mut expected_serialized: serde_json::Value = serde_json::from_str(json_literal)?;
        if !expect_roundtrip && let Some(obj) = expected_serialized.as_object_mut() {
            obj.remove("id");
        }
        assert_eq!(serialized, expected_serialized);
    }

    Ok(())
}

#[test]
fn wraps_image_user_input_with_tags() -> Result<()> {
    let image_url = "data:image/png;base64,abc".to_string();

    let item = ResponseInputItem::from(vec![UserInput::Image {
        image_url: image_url.clone(),
    }]);

    match item {
        ResponseInputItem::Message { content, .. } => {
            let expected = vec![
                ContentItem::InputText {
                    text: image_open_tag_text_with_attachment_id(&image_attachment_id(1)),
                },
                ContentItem::InputImage {
                    image_url,
                    detail: Some(DEFAULT_IMAGE_DETAIL),
                },
                ContentItem::InputText {
                    text: image_close_tag_text(),
                },
            ];
            assert_eq!(content, expected);
        }
        other => panic!("expected message response but got {other:?}"),
    }

    Ok(())
}

#[test]
fn tool_search_call_roundtrips() -> Result<()> {
    let parsed: ResponseItem = serde_json::from_str(
        r#"{
                "type": "tool_search_call",
                "call_id": "search-1",
                "execution": "client",
                "arguments": {
                    "query": "calendar create",
                    "limit": 1
                }
            }"#,
    )?;

    assert_eq!(
        parsed,
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({
                "query": "calendar create",
                "limit": 1,
            }),
        }
    );

    assert_eq!(
        serde_json::to_value(&parsed)?,
        serde_json::json!({
            "type": "tool_search_call",
            "call_id": "search-1",
            "execution": "client",
            "arguments": {
                "query": "calendar create",
                "limit": 1,
            }
        })
    );

    Ok(())
}

#[test]
fn tool_search_output_roundtrips() -> Result<()> {
    let input = ResponseInputItem::ToolSearchOutput {
        call_id: "search-1".to_string(),
        status: "completed".to_string(),
        execution: "client".to_string(),
        tools: vec![serde_json::json!({
            "type": "function",
            "name": "mcp__codex_apps__calendar_create_event",
            "description": "Create a calendar event.",
            "defer_loading": true,
            "parameters": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"}
                },
                "required": ["title"],
                "additionalProperties": false,
            }
        })],
    };
    assert_eq!(
        ResponseItem::from(input.clone()),
        ResponseItem::ToolSearchOutput {
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "function",
                "name": "mcp__codex_apps__calendar_create_event",
                "description": "Create a calendar event.",
                "defer_loading": true,
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"}
                    },
                    "required": ["title"],
                    "additionalProperties": false,
                }
            })],
        }
    );

    assert_eq!(
        serde_json::to_value(input)?,
        serde_json::json!({
            "type": "tool_search_output",
            "call_id": "search-1",
            "status": "completed",
            "execution": "client",
            "tools": [{
                "type": "function",
                "name": "mcp__codex_apps__calendar_create_event",
                "description": "Create a calendar event.",
                "defer_loading": true,
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"}
                    },
                    "required": ["title"],
                    "additionalProperties": false,
                }
            }]
        })
    );

    Ok(())
}

#[test]
fn tool_search_server_items_allow_null_call_id() -> Result<()> {
    let parsed_call: ResponseItem = serde_json::from_str(
        r#"{
                "type": "tool_search_call",
                "execution": "server",
                "call_id": null,
                "status": "completed",
                "arguments": {
                    "paths": ["crm"]
                }
            }"#,
    )?;
    assert_eq!(
        parsed_call,
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: None,
            status: Some("completed".to_string()),
            execution: "server".to_string(),
            arguments: serde_json::json!({
                "paths": ["crm"],
            }),
        }
    );

    let parsed_output: ResponseItem = serde_json::from_str(
        r#"{
                "type": "tool_search_output",
                "execution": "server",
                "call_id": null,
                "status": "completed",
                "tools": []
            }"#,
    )?;
    assert_eq!(
        parsed_output,
        ResponseItem::ToolSearchOutput {
            call_id: None,
            status: "completed".to_string(),
            execution: "server".to_string(),
            tools: vec![],
        }
    );

    Ok(())
}
