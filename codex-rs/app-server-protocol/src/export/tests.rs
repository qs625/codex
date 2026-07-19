use super::*;
use crate::protocol;
use crate::schema_fixtures::read_schema_fixture_subtree;
use anyhow::Context;
use anyhow::Result;
use pretty_assertions::assert_eq;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

#[test]
fn generated_ts_optional_nullable_fields_only_in_params() -> Result<()> {
    // Assert that "?: T | null" only appears in generated *Params types.
    let fixture_tree = read_schema_fixture_subtree(&schema_root()?, "typescript")?;

    let client_request_ts = std::str::from_utf8(
        fixture_tree
            .get(Path::new("ClientRequest.ts"))
            .ok_or_else(|| anyhow::anyhow!("missing ClientRequest.ts fixture"))?,
    )?;
    assert_eq!(client_request_ts.contains("mock/experimentalMethod"), false);
    assert_eq!(
        client_request_ts.contains("MockExperimentalMethodParams"),
        false
    );
    let typescript_index = std::str::from_utf8(
        fixture_tree
            .get(Path::new("index.ts"))
            .ok_or_else(|| anyhow::anyhow!("missing index.ts fixture"))?,
    )?;
    assert_eq!(typescript_index.contains("export type { EventMsg }"), false);
    let thread_start_ts = std::str::from_utf8(
        fixture_tree
            .get(Path::new("ThreadStartParams.ts"))
            .ok_or_else(|| anyhow::anyhow!("missing ThreadStartParams.ts fixture"))?,
    )?;
    assert_eq!(thread_start_ts.contains("mockExperimentalField"), false);
    assert_eq!(
        fixture_tree.contains_key(Path::new("MockExperimentalMethodParams.ts")),
        false
    );
    assert_eq!(
        fixture_tree.contains_key(Path::new("MockExperimentalMethodResponse.ts")),
        false
    );

    let mut undefined_offenders = Vec::new();
    let mut optional_nullable_offenders = BTreeSet::new();
    for (path, contents) in &fixture_tree {
        if !matches!(path.extension().and_then(|ext| ext.to_str()), Some("ts")) {
            continue;
        }

        let allow_optional_nullable = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| {
                stem.ends_with("Params")
                    || stem == "InitializeCapabilities"
                    || matches!(
                        stem,
                        "CollabAgentRef"
                            | "CollabAgentLifecycleEntry"
                            | "CollabAgentSpawnEndEvent"
                            | "CollabAgentInteractionEndEvent"
                            | "CollabCloseEndEvent"
                            | "CollabResumeBeginEvent"
                            | "CollabResumeEndEvent"
                            | "ThreadLifecycleFinalStatus"
                            | "ThreadLifecycleStatus"
                    )
            });

        let contents = std::str::from_utf8(contents)?;
        if contents.contains("| undefined") {
            undefined_offenders.push(path.clone());
        }

        const SKIP_PREFIXES: &[&str] = &[
            "const ",
            "let ",
            "var ",
            "export const ",
            "export let ",
            "export var ",
        ];

        let mut search_start = 0;
        while let Some(idx) = contents[search_start..].find("| null") {
            let abs_idx = search_start + idx;
            let line_start_idx = contents[..abs_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);

            let mut segment_start_idx = line_start_idx;
            if let Some(rel_idx) = contents[line_start_idx..abs_idx].rfind(',') {
                segment_start_idx = segment_start_idx.max(line_start_idx + rel_idx + 1);
            }
            if let Some(rel_idx) = contents[line_start_idx..abs_idx].rfind('{') {
                segment_start_idx = segment_start_idx.max(line_start_idx + rel_idx + 1);
            }
            if let Some(rel_idx) = contents[line_start_idx..abs_idx].rfind('}') {
                segment_start_idx = segment_start_idx.max(line_start_idx + rel_idx + 1);
            }

            let mut level_brace = 0_i32;
            let mut level_brack = 0_i32;
            let mut level_paren = 0_i32;
            let mut in_single = false;
            let mut in_double = false;
            let mut escape = false;
            let mut prop_colon_idx = None;
            for (i, ch) in contents[segment_start_idx..abs_idx].char_indices() {
                let idx_abs = segment_start_idx + i;
                if escape {
                    escape = false;
                    continue;
                }
                match ch {
                    '\\' => {
                        if in_single || in_double {
                            escape = true;
                        }
                    }
                    '\'' => {
                        if !in_double {
                            in_single = !in_single;
                        }
                    }
                    '"' => {
                        if !in_single {
                            in_double = !in_double;
                        }
                    }
                    '{' if !in_single && !in_double => level_brace += 1,
                    '}' if !in_single && !in_double => level_brace -= 1,
                    '[' if !in_single && !in_double => level_brack += 1,
                    ']' if !in_single && !in_double => level_brack -= 1,
                    '(' if !in_single && !in_double => level_paren += 1,
                    ')' if !in_single && !in_double => level_paren -= 1,
                    ':' if !in_single
                        && !in_double
                        && level_brace == 0
                        && level_brack == 0
                        && level_paren == 0 =>
                    {
                        prop_colon_idx = Some(idx_abs);
                        break;
                    }
                    _ => {}
                }
            }

            let Some(colon_idx) = prop_colon_idx else {
                search_start = abs_idx + 5;
                continue;
            };

            let mut field_prefix = contents[segment_start_idx..colon_idx].trim();
            if field_prefix.is_empty() {
                search_start = abs_idx + 5;
                continue;
            }

            if let Some(comment_idx) = field_prefix.rfind("*/") {
                field_prefix = field_prefix[comment_idx + 2..].trim_start();
            }

            if field_prefix.is_empty() {
                search_start = abs_idx + 5;
                continue;
            }

            if SKIP_PREFIXES
                .iter()
                .any(|prefix| field_prefix.starts_with(prefix))
                || field_prefix.contains('(')
            {
                search_start = abs_idx + 5;
                continue;
            }

            if field_prefix.chars().rev().find(|c| !c.is_whitespace()) == Some('?')
                && !allow_optional_nullable
            {
                let line_number = contents[..abs_idx].chars().filter(|c| *c == '\n').count() + 1;
                let offending_line_end = contents[line_start_idx..]
                    .find('\n')
                    .map(|i| line_start_idx + i)
                    .unwrap_or(contents.len());
                let offending_snippet = contents[line_start_idx..offending_line_end].trim();

                optional_nullable_offenders.insert(format!(
                    "{}:{}: {offending_snippet}",
                    path.display(),
                    line_number
                ));
            }

            search_start = abs_idx + 5;
        }
    }

    assert!(
        undefined_offenders.is_empty(),
        "Generated TypeScript still includes unions with `undefined` in {undefined_offenders:?}"
    );
    assert!(
        optional_nullable_offenders.is_empty(),
        "Generated TypeScript has optional nullable fields outside *Params types (disallowed '?: T | null'):\n{optional_nullable_offenders:?}"
    );

    Ok(())
}

fn schema_root() -> Result<PathBuf> {
    let typescript_index = codex_utils_cargo_bin::find_resource!("schema/typescript/index.ts")
        .context("resolve TypeScript schema index.ts")?;
    let schema_root = typescript_index
        .parent()
        .and_then(|parent| parent.parent())
        .context("derive schema root from schema/typescript/index.ts")?
        .to_path_buf();
    Ok(schema_root)
}

#[test]
fn generate_ts_with_experimental_api_retains_experimental_entries() -> Result<()> {
    let client_request_ts = ClientRequest::export_to_string()?;
    assert_eq!(client_request_ts.contains("mock/experimentalMethod"), true);
    assert_eq!(
        client_request_ts.contains("MockExperimentalMethodParams"),
        true
    );
    assert_eq!(
        protocol::MockExperimentalMethodParams::export_to_string()?
            .contains("MockExperimentalMethodParams"),
        true
    );
    assert_eq!(
        protocol::MockExperimentalMethodResponse::export_to_string()?
            .contains("MockExperimentalMethodResponse"),
        true
    );

    let thread_start_ts = protocol::ThreadStartParams::export_to_string()?;
    assert_eq!(thread_start_ts.contains("mockExperimentalField"), true);
    let command_execution_request_approval_ts =
        protocol::CommandExecutionRequestApprovalParams::export_to_string()?;
    assert_eq!(
        command_execution_request_approval_ts.contains("additionalPermissions"),
        true
    );

    Ok(())
}

#[test]
fn stable_schema_filter_removes_mock_thread_start_field() -> Result<()> {
    let output_dir = std::env::temp_dir().join(format!("codex_schema_{}", Uuid::now_v7()));
    fs::create_dir(&output_dir)?;
    let schema =
        write_json_schema_with_return::<protocol::ThreadStartParams>(&output_dir, "ThreadStartParams")?;
    let mut bundle = build_schema_bundle(vec![schema])?;
    filter_experimental_schema(&mut bundle)?;

    let definitions = bundle["definitions"]
        .as_object()
        .expect("schema bundle should include definitions");
    let (_, def_schema) = definitions
        .iter()
        .find(|(name, _)| definition_matches_type(name, "ThreadStartParams"))
        .expect("ThreadStartParams definition should exist");
    let properties = def_schema["properties"]
        .as_object()
        .expect("ThreadStartParams should have properties");
    assert_eq!(properties.contains_key("mockExperimentalField"), false);
    let _cleanup = fs::remove_dir_all(&output_dir);
    Ok(())
}

#[test]
fn build_schema_bundle_keeps_root_definition_refs() -> Result<()> {
    let bundle = build_schema_bundle(vec![
        GeneratedSchema {
            logical_name: "LegacyEnvelope".to_string(),
            value: serde_json::json!({
                "title": "LegacyEnvelope",
                "type": "object",
                "properties": {
                    "current_thread": { "$ref": "#/definitions/ThreadId" },
                    "turn_item": { "$ref": "#/definitions/TurnItem" }
                },
                "definitions": {
                    "TurnItem": {
                        "type": "object",
                        "properties": {
                            "thread_id": { "$ref": "#/definitions/ThreadId" },
                            "phase": { "$ref": "#/definitions/MessagePhase" },
                            "content": {
                                "type": "array",
                                "items": { "$ref": "#/definitions/UserInput" }
                            }
                        }
                    }
                }
            }),
        },
        GeneratedSchema {
            logical_name: "ThreadId".to_string(),
            value: serde_json::json!({
                "title": "ThreadId",
                "type": "string"
            }),
        },
        GeneratedSchema {
            logical_name: "MessagePhase".to_string(),
            value: serde_json::json!({
                "title": "MessagePhase",
                "type": "string"
            }),
        },
        GeneratedSchema {
            logical_name: "UserInput".to_string(),
            value: serde_json::json!({
                "title": "UserInput",
                "type": "string"
            }),
        },
    ])?;

    assert_eq!(
        bundle["definitions"]["LegacyEnvelope"]["properties"]["current_thread"]["$ref"],
        serde_json::json!("#/definitions/ThreadId")
    );
    assert_eq!(
        bundle["definitions"]["LegacyEnvelope"]["properties"]["turn_item"]["$ref"],
        serde_json::json!("#/definitions/TurnItem")
    );
    assert_eq!(
        bundle["definitions"]["TurnItem"]["properties"]["thread_id"]["$ref"],
        serde_json::json!("#/definitions/ThreadId")
    );
    assert_eq!(
        bundle["definitions"]["TurnItem"]["properties"]["phase"]["$ref"],
        serde_json::json!("#/definitions/MessagePhase")
    );
    assert_eq!(
        bundle["definitions"]["TurnItem"]["properties"]["content"]["items"]["$ref"],
        serde_json::json!("#/definitions/UserInput")
    );

    Ok(())
}

#[test]
fn experimental_type_fields_ts_filter_handles_interface_shape() -> Result<()> {
    let output_dir = std::env::temp_dir().join(format!("codex_ts_filter_{}", Uuid::now_v7()));
    fs::create_dir_all(&output_dir)?;

    struct TempDirGuard(PathBuf);

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let _guard = TempDirGuard(output_dir.clone());
    let path = output_dir.join("CustomParams.ts");
    let content = r#"export interface CustomParams {
  stableField: string | null;
  unstableField: string | null;
  otherStableField: boolean;
}
"#;
    fs::write(&path, content)?;

    static CUSTOM_FIELD: crate::experimental_api::ExperimentalField =
        crate::experimental_api::ExperimentalField {
            type_name: "CustomParams",
            field_name: "unstableField",
            reason: "custom/unstableField",
        };
    filter_experimental_type_fields_ts(&output_dir, &[&CUSTOM_FIELD])?;

    let filtered = fs::read_to_string(&path)?;
    assert_eq!(filtered.contains("unstableField"), false);
    assert_eq!(filtered.contains("stableField"), true);
    assert_eq!(filtered.contains("otherStableField"), true);
    Ok(())
}

#[test]
fn experimental_type_fields_ts_filter_keeps_imports_used_in_intersection_suffix() -> Result<()> {
    let output_dir = std::env::temp_dir().join(format!("codex_ts_filter_{}", Uuid::now_v7()));
    fs::create_dir_all(&output_dir)?;

    struct TempDirGuard(PathBuf);

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let _guard = TempDirGuard(output_dir.clone());
    let path = output_dir.join("Config.ts");
    let content = r#"import type { JsonValue } from "../serde_json/JsonValue";
import type { Keep } from "./Keep";

export type Config = { stableField: Keep, unstableField: string | null } & ({ [key in string]?: number | string | boolean | Array<JsonValue> | { [key in string]?: JsonValue } | null });
"#;
    fs::write(&path, content)?;

    static CUSTOM_FIELD: crate::experimental_api::ExperimentalField =
        crate::experimental_api::ExperimentalField {
            type_name: "Config",
            field_name: "unstableField",
            reason: "custom/unstableField",
        };
    filter_experimental_type_fields_ts(&output_dir, &[&CUSTOM_FIELD])?;

    let filtered = fs::read_to_string(&path)?;
    assert_eq!(filtered.contains("unstableField"), false);
    assert_eq!(
        filtered.contains(r#"import type { JsonValue } from "../serde_json/JsonValue";"#),
        true
    );
    assert_eq!(filtered.contains(r#"import type { Keep } from "./Keep";"#), true);
    Ok(())
}

#[test]
fn experimental_type_fields_ts_filter_handles_generated_command_params_shape() -> Result<()> {
    let output_dir = std::env::temp_dir().join(format!("codex_ts_filter_{}", Uuid::now_v7()));
    fs::create_dir_all(&output_dir)?;

    struct TempDirGuard(PathBuf);

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let _guard = TempDirGuard(output_dir.clone());
    let path = output_dir.join("CommandExecParams.ts");
    let content = r#"import type { CommandExecTerminalSize } from "./CommandExecTerminalSize";
import type { PermissionProfile } from "./PermissionProfile";
import type { SandboxPolicy } from "./SandboxPolicy";

export type CommandExecParams = {/**
 * Command argv vector. Empty arrays are rejected.
 */
command: Array<string>, /**
 * Optional environment overrides merged into the server-computed
 * environment.
 */
env?: { [key in string]?: string | null } | null, /**
 * Optional initial PTY size in character cells. Only valid when `tty` is
 * true.
 */
size?: CommandExecTerminalSize | null, /**
 * Optional sandbox policy for this command.
 *
 * Uses the same shape as thread/turn execution sandbox configuration and
 * defaults to the user's configured policy when omitted. Cannot be
 * combined with `permissionProfile`.
 */
sandboxPolicy?: SandboxPolicy | null,
/**
 * Optional full permissions profile for this command.
 *
 * Defaults to the user's configured permissions when omitted. Cannot be
 * combined with `sandboxPolicy`.
 */
permissionProfile?: PermissionProfile | null};
"#;
    fs::write(&path, content)?;

    static CUSTOM_FIELD: crate::experimental_api::ExperimentalField =
        crate::experimental_api::ExperimentalField {
            type_name: "CommandExecParams",
            field_name: "permissionProfile",
            reason: "command/exec.permissionProfile",
        };
    filter_experimental_type_fields_ts(&output_dir, &[&CUSTOM_FIELD])?;

    let filtered = fs::read_to_string(&path)?;
    assert_eq!(
        filtered.contains("permissionProfile?: PermissionProfile"),
        false
    );
    assert_eq!(
        filtered.contains(r#"import type { PermissionProfile } from "./PermissionProfile";"#),
        false
    );
    assert_eq!(filtered.contains("sandboxPolicy?: SandboxPolicy"), true);
    assert_eq!(
        filtered.contains(r#"import type { SandboxPolicy } from "./SandboxPolicy";"#),
        true
    );
    Ok(())
}

#[test]
fn stable_schema_filter_removes_mock_experimental_method() -> Result<()> {
    let output_dir = std::env::temp_dir().join(format!("codex_schema_{}", Uuid::now_v7()));
    fs::create_dir(&output_dir)?;
    let schema =
        write_json_schema_with_return::<crate::ClientRequest>(&output_dir, "ClientRequest")?;
    let mut bundle = build_schema_bundle(vec![schema])?;
    filter_experimental_schema(&mut bundle)?;

    let bundle_str = serde_json::to_string(&bundle)?;
    assert_eq!(bundle_str.contains("mock/experimentalMethod"), false);
    let _cleanup = fs::remove_dir_all(&output_dir);
    Ok(())
}

#[test]
fn generate_json_filters_experimental_fields_and_methods() -> Result<()> {
    let output_dir = std::env::temp_dir().join(format!("codex_schema_{}", Uuid::now_v7()));
    fs::create_dir(&output_dir)?;
    generate_json_with_experimental(&output_dir, /*experimental_api*/ false)?;

    let thread_start_json = fs::read_to_string(output_dir.join("ThreadStartParams.json"))?;
    assert_eq!(thread_start_json.contains("mockExperimentalField"), false);
    let command_execution_request_approval_json =
        fs::read_to_string(output_dir.join("CommandExecutionRequestApprovalParams.json"))?;
    assert_eq!(
        command_execution_request_approval_json.contains("additionalPermissions"),
        false
    );

    let client_request_json = fs::read_to_string(output_dir.join("ClientRequest.json"))?;
    assert_eq!(
        client_request_json.contains("mock/experimentalMethod"),
        false
    );
    assert_eq!(output_dir.join("EventMsg.json").exists(), false);

    let bundle_json = fs::read_to_string(output_dir.join("app_server_protocol.schemas.json"))?;
    assert_eq!(bundle_json.contains("mockExperimentalField"), false);
    assert_eq!(bundle_json.contains("additionalPermissions"), false);
    assert_eq!(bundle_json.contains("MockExperimentalMethodParams"), false);
    assert_eq!(bundle_json.contains("MockExperimentalMethodResponse"), false);
    let bundle = read_json_value(&output_dir.join("app_server_protocol.schemas.json"))?;
    let definitions = bundle["definitions"]
        .as_object()
        .expect("schema bundle should include definitions");
    let client_request_methods: BTreeSet<String> = definitions["ClientRequest"]["oneOf"]
        .as_array()
        .expect("ClientRequest should remain a oneOf")
        .iter()
        .filter_map(|variant| {
            variant["properties"]["method"]["enum"]
                .as_array()
                .and_then(|values| values.first())
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let missing_client_request_methods: Vec<String> = [
        "account/logout",
        "account/rateLimits/read",
        "config/mcpServer/reload",
        "configRequirements/read",
        "fuzzyFileSearch",
        "initialize",
    ]
    .into_iter()
    .filter(|method| !client_request_methods.contains(*method))
    .map(str::to_string)
    .collect();
    assert_eq!(missing_client_request_methods, Vec::<String>::new());
    let server_notification_methods: BTreeSet<String> = definitions["ServerNotification"]["oneOf"]
        .as_array()
        .expect("ServerNotification should remain a oneOf")
        .iter()
        .filter_map(|variant| {
            variant["properties"]["method"]["enum"]
                .as_array()
                .and_then(|values| values.first())
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let missing_server_notification_methods: Vec<String> = [
        "fuzzyFileSearch/sessionCompleted",
        "fuzzyFileSearch/sessionUpdated",
        "serverRequest/resolved",
    ]
    .into_iter()
    .filter(|method| !server_notification_methods.contains(*method))
    .map(str::to_string)
    .collect();
    assert_eq!(missing_server_notification_methods, Vec::<String>::new());
    assert_eq!(definitions.contains_key("EventMsg"), false);
    assert_eq!(output_dir.join("MockExperimentalMethodParams.json").exists(), false);
    assert_eq!(
        output_dir.join("MockExperimentalMethodResponse.json").exists(),
        false
    );

    let _cleanup = fs::remove_dir_all(&output_dir);
    Ok(())
}
