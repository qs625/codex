use crate::ClientNotification;
use crate::ClientRequest;
use crate::ServerNotification;
use crate::ServerRequest;
use crate::experimental_api::experimental_fields;
use crate::export_client_notification_schemas;
use crate::export_client_param_schemas;
use crate::export_client_response_schemas;
use crate::export_client_responses;
use crate::export_server_notification_schemas;
use crate::export_server_param_schemas;
use crate::export_server_response_schemas;
use crate::export_server_responses;
use crate::protocol::common::EXPERIMENTAL_CLIENT_METHOD_PARAM_TYPES;
use crate::protocol::common::EXPERIMENTAL_CLIENT_METHOD_RESPONSE_TYPES;
use crate::protocol::common::EXPERIMENTAL_CLIENT_METHODS;
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use protocol::protocol::RolloutLine;
use schemars::JsonSchema;
use schemars::schema_for;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use ts_rs::TS;

pub(crate) const GENERATED_TS_HEADER: &str = "// GENERATED CODE! DO NOT MODIFY BY HAND!\n\n";
const IGNORED_DEFINITIONS: &[&str] = &["Option<()>"];
const SPECIAL_DEFINITIONS: &[&str] = &[
    "ClientNotification",
    "ClientRequest",
    "ServerNotification",
    "ServerRequest",
];
#[derive(Clone)]
pub struct GeneratedSchema {
    logical_name: String,
    value: Value,
}

type JsonSchemaEmitter = fn(&Path) -> Result<GeneratedSchema>;
pub fn generate_types(out_dir: &Path, prettier: Option<&Path>) -> Result<()> {
    generate_ts(out_dir, prettier)?;
    generate_json(out_dir)?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct GenerateTsOptions {
    pub generate_indices: bool,
    pub ensure_headers: bool,
    pub run_prettier: bool,
    pub experimental_api: bool,
}

impl Default for GenerateTsOptions {
    fn default() -> Self {
        Self {
            generate_indices: true,
            ensure_headers: true,
            run_prettier: true,
            experimental_api: false,
        }
    }
}

pub fn generate_ts(out_dir: &Path, prettier: Option<&Path>) -> Result<()> {
    generate_ts_with_options(out_dir, prettier, GenerateTsOptions::default())
}

pub fn generate_ts_with_options(
    out_dir: &Path,
    prettier: Option<&Path>,
    options: GenerateTsOptions,
) -> Result<()> {
    ensure_dir(out_dir)?;

    ClientRequest::export_all_to(out_dir)?;
    export_client_responses(out_dir)?;
    ClientNotification::export_all_to(out_dir)?;

    ServerRequest::export_all_to(out_dir)?;
    export_server_responses(out_dir)?;
    ServerNotification::export_all_to(out_dir)?;

    if !options.experimental_api {
        filter_experimental_ts(out_dir)?;
    }

    if options.generate_indices {
        generate_index_ts(out_dir)?;
    }

    // Ensure our header is present on all generated TS files.
    let ts_files = ts_files_in_recursive(out_dir)?;

    if options.ensure_headers {
        let worker_count = thread::available_parallelism()
            .map_or(1, usize::from)
            .min(ts_files.len().max(1));
        let chunk_size = ts_files.len().div_ceil(worker_count);
        thread::scope(|scope| -> Result<()> {
            let mut workers = Vec::new();
            for chunk in ts_files.chunks(chunk_size.max(1)) {
                workers.push(scope.spawn(move || -> Result<()> {
                    for file in chunk {
                        prepend_header_if_missing(file)?;
                    }
                    Ok(())
                }));
            }

            for worker in workers {
                worker
                    .join()
                    .map_err(|_| anyhow!("TypeScript header worker panicked"))??;
            }

            Ok(())
        })?;
    }

    // Optionally run Prettier on all generated TS files.
    if options.run_prettier
        && let Some(prettier_bin) = prettier
        && !ts_files.is_empty()
    {
        let status = Command::new(prettier_bin)
            .arg("--write")
            .arg("--log-level")
            .arg("warn")
            .args(ts_files.iter().map(|p| p.as_os_str()))
            .status()
            .with_context(|| format!("Failed to invoke Prettier at {}", prettier_bin.display()))?;
        if !status.success() {
            return Err(anyhow!("Prettier failed with status {status}"));
        }
    }

    trim_trailing_whitespace_in_ts_files(&ts_files)?;

    Ok(())
}

pub fn generate_json(out_dir: &Path) -> Result<()> {
    generate_json_with_experimental(out_dir, /*experimental_api*/ false)
}

pub fn generate_internal_json_schema(out_dir: &Path) -> Result<()> {
    ensure_dir(out_dir)?;
    write_json_schema::<RolloutLine>(out_dir, "RolloutLine")?;
    Ok(())
}

pub fn generate_json_with_experimental(out_dir: &Path, experimental_api: bool) -> Result<()> {
    ensure_dir(out_dir)?;
    let envelope_emitters: Vec<JsonSchemaEmitter> = vec![
        |d| write_json_schema_with_return::<crate::RequestId>(d, "RequestId"),
        |d| write_json_schema_with_return::<crate::JSONRPCMessage>(d, "JSONRPCMessage"),
        |d| write_json_schema_with_return::<crate::JSONRPCRequest>(d, "JSONRPCRequest"),
        |d| write_json_schema_with_return::<crate::JSONRPCNotification>(d, "JSONRPCNotification"),
        |d| write_json_schema_with_return::<crate::JSONRPCResponse>(d, "JSONRPCResponse"),
        |d| write_json_schema_with_return::<crate::JSONRPCError>(d, "JSONRPCError"),
        |d| write_json_schema_with_return::<crate::JSONRPCErrorError>(d, "JSONRPCErrorError"),
        |d| write_json_schema_with_return::<crate::ClientRequest>(d, "ClientRequest"),
        |d| write_json_schema_with_return::<crate::ServerRequest>(d, "ServerRequest"),
        |d| write_json_schema_with_return::<crate::ClientNotification>(d, "ClientNotification"),
        |d| write_json_schema_with_return::<crate::ServerNotification>(d, "ServerNotification"),
    ];

    let mut schemas: Vec<GeneratedSchema> = Vec::new();
    for emit in &envelope_emitters {
        schemas.push(emit(out_dir)?);
    }

    schemas.extend(export_client_param_schemas(out_dir)?);
    schemas.extend(export_client_response_schemas(out_dir)?);
    schemas.extend(export_server_param_schemas(out_dir)?);
    schemas.extend(export_server_response_schemas(out_dir)?);
    schemas.extend(export_client_notification_schemas(out_dir)?);
    schemas.extend(export_server_notification_schemas(out_dir)?);
    let mut bundle = build_schema_bundle(schemas)?;
    if !experimental_api {
        filter_experimental_schema(&mut bundle)?;
    }
    write_pretty_json(out_dir.join("app_server_protocol.schemas.json"), &bundle)?;

    if !experimental_api {
        filter_experimental_json_files(out_dir)?;
    }

    Ok(())
}

fn filter_experimental_ts(out_dir: &Path) -> Result<()> {
    let registered_fields = experimental_fields();
    let experimental_method_types = experimental_method_types();
    // Most generated TS files are filtered by schema processing, but
    // `ClientRequest.ts` and any type with `#[experimental(...)]` fields need
    // direct post-processing because they encode method/field information in
    // file-local unions/interfaces.
    filter_client_request_ts(out_dir, EXPERIMENTAL_CLIENT_METHODS)?;
    filter_experimental_type_fields_ts(out_dir, &registered_fields)?;
    remove_generated_type_files(out_dir, &experimental_method_types, "ts")?;
    Ok(())
}

pub(crate) fn filter_experimental_ts_tree(tree: &mut BTreeMap<PathBuf, String>) -> Result<()> {
    let registered_fields = experimental_fields();
    let experimental_method_types = experimental_method_types();
    if let Some(content) = tree.get_mut(Path::new("ClientRequest.ts")) {
        let filtered =
            filter_client_request_ts_contents(std::mem::take(content), EXPERIMENTAL_CLIENT_METHODS);
        *content = filtered;
    }

    let mut fields_by_type_name: HashMap<String, HashSet<String>> = HashMap::new();
    for field in registered_fields {
        fields_by_type_name
            .entry(field.type_name.to_string())
            .or_default()
            .insert(field.field_name.to_string());
    }

    for (path, content) in tree.iter_mut() {
        let Some(type_name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(experimental_field_names) = fields_by_type_name.get(type_name) else {
            continue;
        };
        let filtered = filter_experimental_type_fields_ts_contents(
            std::mem::take(content),
            experimental_field_names,
        );
        *content = filtered;
    }

    remove_generated_type_entries(tree, &experimental_method_types, "ts");
    Ok(())
}

/// Removes union arms from `ClientRequest.ts` for methods marked experimental.
fn filter_client_request_ts(out_dir: &Path, experimental_methods: &[&str]) -> Result<()> {
    let path = out_dir.join("ClientRequest.ts");
    if !path.exists() {
        return Ok(());
    }
    let mut content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    content = filter_client_request_ts_contents(content, experimental_methods);

    fs::write(&path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

fn filter_client_request_ts_contents(mut content: String, experimental_methods: &[&str]) -> String {
    let Some((prefix, body, suffix)) = split_type_alias(&content) else {
        return content;
    };
    let experimental_methods: HashSet<&str> = experimental_methods
        .iter()
        .copied()
        .filter(|method| !method.is_empty())
        .collect();
    let arms = split_top_level(&body, '|');
    let filtered_arms: Vec<String> = arms
        .into_iter()
        .filter(|arm| {
            extract_method_from_arm(arm)
                .is_none_or(|method| !experimental_methods.contains(method.as_str()))
        })
        .collect();
    let new_body = filtered_arms.join(" | ");
    content = format!("{prefix}{new_body}{suffix}");
    let import_usage_scope = split_type_alias(&content)
        .map(|(_, filtered_body, _)| filtered_body)
        .unwrap_or_else(|| new_body.clone());
    prune_unused_type_imports(content, &import_usage_scope)
}

/// Removes experimental properties from generated TypeScript type files.
fn filter_experimental_type_fields_ts(
    out_dir: &Path,
    experimental_fields: &[&'static crate::experimental_api::ExperimentalField],
) -> Result<()> {
    let mut fields_by_type_name: HashMap<String, HashSet<String>> = HashMap::new();
    for field in experimental_fields {
        fields_by_type_name
            .entry(field.type_name.to_string())
            .or_default()
            .insert(field.field_name.to_string());
    }
    if fields_by_type_name.is_empty() {
        return Ok(());
    }

    for path in ts_files_in_recursive(out_dir)? {
        let Some(type_name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(experimental_field_names) = fields_by_type_name.get(type_name) else {
            continue;
        };
        filter_experimental_fields_in_ts_file(&path, experimental_field_names)?;
    }

    Ok(())
}

fn filter_experimental_fields_in_ts_file(
    path: &Path,
    experimental_field_names: &HashSet<String>,
) -> Result<()> {
    let mut content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    content = filter_experimental_type_fields_ts_contents(content, experimental_field_names);
    fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

fn filter_experimental_type_fields_ts_contents(
    mut content: String,
    experimental_field_names: &HashSet<String>,
) -> String {
    let Some((open_brace, close_brace)) = type_body_brace_span(&content) else {
        return content;
    };
    let inner = &content[open_brace + 1..close_brace];
    let fields = split_top_level_multi(inner, &[',', ';']);
    let filtered_fields: Vec<String> = fields
        .into_iter()
        .filter(|field| {
            let field = strip_leading_block_comments(field);
            parse_property_name(field)
                .is_none_or(|name| !experimental_field_names.contains(name.as_str()))
        })
        .collect();
    let new_inner = filtered_fields.join(", ");
    let prefix = &content[..open_brace + 1];
    let suffix = &content[close_brace..];
    content = format!("{prefix}{new_inner}{suffix}");
    let import_usage_scope = split_type_alias(&content)
        .map(|(_, body, _)| body)
        .unwrap_or_else(|| new_inner.clone());
    prune_unused_type_imports(content, &import_usage_scope)
}

fn filter_experimental_schema(bundle: &mut Value) -> Result<()> {
    let registered_fields = experimental_fields();
    filter_experimental_fields_in_root(bundle, &registered_fields);
    filter_experimental_fields_in_definitions(bundle, &registered_fields);
    prune_experimental_methods(bundle, EXPERIMENTAL_CLIENT_METHODS);
    remove_experimental_method_type_definitions(bundle);
    Ok(())
}

fn filter_experimental_fields_in_root(
    schema: &mut Value,
    experimental_fields: &[&'static crate::experimental_api::ExperimentalField],
) {
    let Some(title) = schema.get("title").and_then(Value::as_str) else {
        return;
    };
    let title = title.to_string();

    for field in experimental_fields {
        if title != field.type_name {
            continue;
        }
        remove_property_from_schema(schema, field.field_name);
    }
}

fn filter_experimental_fields_in_definitions(
    bundle: &mut Value,
    experimental_fields: &[&'static crate::experimental_api::ExperimentalField],
) {
    let Some(definitions) = bundle.get_mut("definitions").and_then(Value::as_object_mut) else {
        return;
    };

    filter_experimental_fields_in_definitions_map(definitions, experimental_fields);
}

fn filter_experimental_fields_in_definitions_map(
    definitions: &mut Map<String, Value>,
    experimental_fields: &[&'static crate::experimental_api::ExperimentalField],
) {
    for (def_name, def_schema) in definitions.iter_mut() {
        if is_namespace_map(def_schema) {
            if let Some(namespace_defs) = def_schema.as_object_mut() {
                filter_experimental_fields_in_definitions_map(namespace_defs, experimental_fields);
            }
            continue;
        }

        for field in experimental_fields {
            if !definition_matches_type(def_name, field.type_name) {
                continue;
            }
            remove_property_from_schema(def_schema, field.field_name);
        }
    }
}

fn is_namespace_map(value: &Value) -> bool {
    let Value::Object(map) = value else {
        return false;
    };

    if map.keys().any(|key| key.starts_with('$')) {
        return false;
    }

    let looks_like_schema = map.contains_key("type")
        || map.contains_key("properties")
        || map.contains_key("anyOf")
        || map.contains_key("oneOf")
        || map.contains_key("allOf");

    !looks_like_schema && map.values().all(Value::is_object)
}

fn definition_matches_type(def_name: &str, type_name: &str) -> bool {
    def_name == type_name || def_name.ends_with(&format!("::{type_name}"))
}

fn remove_property_from_schema(schema: &mut Value, field_name: &str) {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        properties.remove(field_name);
    }

    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|entry| entry.as_str() != Some(field_name));
    }

    if let Some(inner_schema) = schema.get_mut("schema") {
        remove_property_from_schema(inner_schema, field_name);
    }
}

fn prune_experimental_methods(bundle: &mut Value, experimental_methods: &[&str]) {
    let experimental_methods: HashSet<&str> = experimental_methods
        .iter()
        .copied()
        .filter(|method| !method.is_empty())
        .collect();
    prune_experimental_methods_inner(bundle, &experimental_methods);
}

fn prune_experimental_methods_inner(value: &mut Value, experimental_methods: &HashSet<&str>) {
    match value {
        Value::Array(items) => {
            items.retain(|item| !is_experimental_method_variant(item, experimental_methods));
            for item in items {
                prune_experimental_methods_inner(item, experimental_methods);
            }
        }
        Value::Object(map) => {
            for entry in map.values_mut() {
                prune_experimental_methods_inner(entry, experimental_methods);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_experimental_method_variant(value: &Value, experimental_methods: &HashSet<&str>) -> bool {
    let Value::Object(map) = value else {
        return false;
    };
    let Some(properties) = map.get("properties").and_then(Value::as_object) else {
        return false;
    };
    let Some(method_schema) = properties.get("method").and_then(Value::as_object) else {
        return false;
    };

    if let Some(method) = method_schema.get("const").and_then(Value::as_str) {
        return experimental_methods.contains(method);
    }

    if let Some(values) = method_schema.get("enum").and_then(Value::as_array)
        && values.len() == 1
        && let Some(method) = values[0].as_str()
    {
        return experimental_methods.contains(method);
    }

    false
}

fn filter_experimental_json_files(out_dir: &Path) -> Result<()> {
    for path in json_files_in_recursive(out_dir)? {
        let mut value = read_json_value(&path)?;
        filter_experimental_schema(&mut value)?;
        write_pretty_json(path, &value)?;
    }
    let experimental_method_types = experimental_method_types();
    remove_generated_type_files(out_dir, &experimental_method_types, "json")?;
    Ok(())
}

fn experimental_method_types() -> HashSet<String> {
    let mut type_names = HashSet::new();
    collect_experimental_type_names(EXPERIMENTAL_CLIENT_METHOD_PARAM_TYPES, &mut type_names);
    collect_experimental_type_names(EXPERIMENTAL_CLIENT_METHOD_RESPONSE_TYPES, &mut type_names);
    type_names
}

fn collect_experimental_type_names(entries: &[&str], out: &mut HashSet<String>) {
    for entry in entries {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let name = trimmed.rsplit("::").next().unwrap_or(trimmed);
        if !name.is_empty() {
            out.insert(name.to_string());
        }
    }
}

fn remove_generated_type_files(
    out_dir: &Path,
    type_names: &HashSet<String>,
    extension: &str,
) -> Result<()> {
    for type_name in type_names {
        let path = out_dir.join(format!("{type_name}.{extension}"));
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove {}", path.display()))?;
        }
    }
    Ok(())
}

fn remove_generated_type_entries(
    tree: &mut BTreeMap<PathBuf, String>,
    type_names: &HashSet<String>,
    extension: &str,
) {
    for type_name in type_names {
        let path = PathBuf::from(format!("{type_name}.{extension}"));
        tree.remove(&path);
    }
}

fn remove_experimental_method_type_definitions(bundle: &mut Value) {
    let type_names = experimental_method_types();
    let Some(definitions) = bundle.get_mut("definitions").and_then(Value::as_object_mut) else {
        return;
    };
    remove_experimental_method_type_definitions_map(definitions, &type_names);
}

fn remove_experimental_method_type_definitions_map(
    definitions: &mut Map<String, Value>,
    experimental_type_names: &HashSet<String>,
) {
    let keys_to_remove: Vec<String> = definitions
        .keys()
        .filter(|def_name| {
            experimental_type_names
                .iter()
                .any(|type_name| definition_matches_type(def_name, type_name))
        })
        .cloned()
        .collect();
    for key in keys_to_remove {
        definitions.remove(&key);
    }

    for value in definitions.values_mut() {
        if !is_namespace_map(value) {
            continue;
        }
        if let Some(namespace_defs) = value.as_object_mut() {
            remove_experimental_method_type_definitions_map(
                namespace_defs,
                experimental_type_names,
            );
        }
    }
}

fn prune_unused_type_imports(content: String, type_alias_body: &str) -> String {
    let trailing_newline = content.ends_with('\n');
    let mut lines = Vec::new();
    for line in content.lines() {
        if let Some(type_name) = parse_imported_type_name(line)
            && !type_alias_body.contains(type_name)
        {
            continue;
        }
        lines.push(line);
    }

    let mut rewritten = lines.join("\n");
    if trailing_newline {
        rewritten.push('\n');
    }
    rewritten
}

fn parse_imported_type_name(line: &str) -> Option<&str> {
    let line = line.trim();
    let rest = line.strip_prefix("import type {")?;
    let (type_name, _) = rest.split_once("} from ")?;
    let type_name = type_name.trim();
    if type_name.is_empty() || type_name.contains(',') || type_name.contains(" as ") {
        return None;
    }
    Some(type_name)
}

fn json_files_in_recursive(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if matches!(path.extension().and_then(|ext| ext.to_str()), Some("json")) {
                out.push(path);
            }
        }
    }
    Ok(out)
}

fn read_json_value(path: &Path) -> Result<Value> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

fn split_type_alias(content: &str) -> Option<(String, String, String)> {
    let eq_index = content.find('=')?;
    let semi_index = content.rfind(';')?;
    if semi_index <= eq_index {
        return None;
    }
    let prefix = content[..eq_index + 1].to_string();
    let body = content[eq_index + 1..semi_index].to_string();
    let suffix = content[semi_index..].to_string();
    Some((prefix, body, suffix))
}

fn type_body_brace_span(content: &str) -> Option<(usize, usize)> {
    if let Some(eq_index) = content.find('=') {
        let after_eq = &content[eq_index + 1..];
        let (open_rel, close_rel) = find_top_level_brace_span(after_eq)?;
        return Some((eq_index + 1 + open_rel, eq_index + 1 + close_rel));
    }

    const INTERFACE_MARKER: &str = "export interface";
    let interface_index = content.find(INTERFACE_MARKER)?;
    let after_interface = &content[interface_index + INTERFACE_MARKER.len()..];
    let (open_rel, close_rel) = find_top_level_brace_span(after_interface)?;
    Some((
        interface_index + INTERFACE_MARKER.len() + open_rel,
        interface_index + INTERFACE_MARKER.len() + close_rel,
    ))
}

fn find_top_level_brace_span(input: &str) -> Option<(usize, usize)> {
    let mut state = ScanState::default();
    let mut open_index = None;
    for (index, ch) in input.char_indices() {
        if !state.in_ignored_syntax() && ch == '{' && state.depth.is_top_level() {
            open_index = Some(index);
        }
        state.observe(ch);
        if !state.in_ignored_syntax()
            && ch == '}'
            && state.depth.is_top_level()
            && let Some(open) = open_index
        {
            return Some((open, index));
        }
    }
    None
}

fn split_top_level(input: &str, delimiter: char) -> Vec<String> {
    split_top_level_multi(input, &[delimiter])
}

fn split_top_level_multi(input: &str, delimiters: &[char]) -> Vec<String> {
    let mut state = ScanState::default();
    let mut start = 0usize;
    let mut parts = Vec::new();
    for (index, ch) in input.char_indices() {
        if !state.in_ignored_syntax() && state.depth.is_top_level() && delimiters.contains(&ch) {
            let part = input[start..index].trim();
            if !part.is_empty() {
                parts.push(part.to_string());
            }
            start = index + ch.len_utf8();
        }
        state.observe(ch);
    }
    let tail = input[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    parts
}

fn extract_method_from_arm(arm: &str) -> Option<String> {
    let (open, close) = find_top_level_brace_span(arm)?;
    let inner = &arm[open + 1..close];
    for field in split_top_level(inner, ',') {
        let Some((name, value)) = parse_property(field.as_str()) else {
            continue;
        };
        if name != "method" {
            continue;
        }
        let value = value.trim_start();
        let (literal, _) = parse_string_literal(value)?;
        return Some(literal);
    }
    None
}

fn parse_property(input: &str) -> Option<(String, &str)> {
    let name = parse_property_name(input)?;
    let colon_index = input.find(':')?;
    Some((name, input[colon_index + 1..].trim_start()))
}

fn strip_leading_block_comments(input: &str) -> &str {
    let mut rest = input.trim_start();
    loop {
        let Some(after_prefix) = rest.strip_prefix("/*") else {
            return rest;
        };
        let Some(end_rel) = after_prefix.find("*/") else {
            return rest;
        };
        rest = after_prefix[end_rel + 2..].trim_start();
    }
}

fn parse_property_name(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((literal, consumed)) = parse_string_literal(trimmed) {
        let rest = trimmed[consumed..].trim_start();
        if rest.starts_with(':') {
            return Some(literal);
        }
        return None;
    }

    let mut end = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if !is_ident_char(ch) {
            break;
        }
        end = index + ch.len_utf8();
    }
    if end == 0 {
        return None;
    }
    let name = &trimmed[..end];
    let rest = trimmed[end..].trim_start();
    let rest = if let Some(stripped) = rest.strip_prefix('?') {
        stripped.trim_start()
    } else {
        rest
    };
    if rest.starts_with(':') {
        return Some(name.to_string());
    }
    None
}

fn parse_string_literal(input: &str) -> Option<(String, usize)> {
    let mut chars = input.char_indices();
    let (start_index, quote) = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut escape = false;
    for (index, ch) in chars {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if ch == quote {
            let literal = input[start_index + 1..index].to_string();
            let consumed = index + ch.len_utf8();
            return Some((literal, consumed));
        }
    }
    None
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[derive(Default)]
struct ScanState {
    depth: Depth,
    string_delim: Option<char>,
    escape: bool,
    block_comment: bool,
    line_comment: bool,
    previous_char: Option<char>,
}

impl ScanState {
    fn observe(&mut self, ch: char) {
        if self.line_comment {
            if ch == '\n' {
                self.line_comment = false;
            }
            self.previous_char = Some(ch);
            return;
        }

        if self.block_comment {
            if self.previous_char == Some('*') && ch == '/' {
                self.block_comment = false;
                self.previous_char = None;
            } else {
                self.previous_char = Some(ch);
            }
            return;
        }

        if let Some(delim) = self.string_delim {
            if self.escape {
                self.escape = false;
                self.previous_char = Some(ch);
                return;
            }
            if ch == '\\' {
                self.escape = true;
                self.previous_char = Some(ch);
                return;
            }
            if ch == delim {
                self.string_delim = None;
            }
            self.previous_char = Some(ch);
            return;
        }

        if self.previous_char == Some('/') && ch == '/' {
            self.line_comment = true;
            self.previous_char = Some(ch);
            return;
        }

        if self.previous_char == Some('/') && ch == '*' {
            self.block_comment = true;
            self.previous_char = Some(ch);
            return;
        }

        match ch {
            '"' | '\'' => {
                self.string_delim = Some(ch);
            }
            '{' => self.depth.brace += 1,
            '}' => self.depth.brace = (self.depth.brace - 1).max(0),
            '[' => self.depth.bracket += 1,
            ']' => self.depth.bracket = (self.depth.bracket - 1).max(0),
            '(' => self.depth.paren += 1,
            ')' => self.depth.paren = (self.depth.paren - 1).max(0),
            '<' => self.depth.angle += 1,
            '>' => {
                if self.depth.angle > 0 {
                    self.depth.angle -= 1;
                }
            }
            _ => {}
        }
        self.previous_char = Some(ch);
    }

    fn in_ignored_syntax(&self) -> bool {
        self.string_delim.is_some() || self.block_comment || self.line_comment
    }
}

#[derive(Default)]
struct Depth {
    brace: i32,
    bracket: i32,
    paren: i32,
    angle: i32,
}

impl Depth {
    fn is_top_level(&self) -> bool {
        self.brace == 0 && self.bracket == 0 && self.paren == 0 && self.angle == 0
    }
}

fn build_schema_bundle(schemas: Vec<GeneratedSchema>) -> Result<Value> {
    let mut definitions = Map::new();

    for schema in schemas {
        let GeneratedSchema { logical_name, mut value } = schema;

        if IGNORED_DEFINITIONS.contains(&logical_name.as_str()) {
            continue;
        }

        let logical_prefix = logical_name
            .rsplit_once('/')
            .map(|(prefix, _)| prefix.to_string());

        if let Value::Object(ref mut obj) = value
            && let Some(defs) = obj.remove("definitions")
            && let Value::Object(defs_obj) = defs
        {
            for (def_name, mut def_schema) in defs_obj {
                if IGNORED_DEFINITIONS.contains(&def_name.as_str()) {
                    continue;
                }
                if SPECIAL_DEFINITIONS.contains(&def_name.as_str()) {
                    continue;
                }
                if let Some(prefix) = logical_prefix.as_deref() {
                    prefix_schema_definition_refs(&mut def_schema, prefix);
                }
                annotate_schema(&mut def_schema, Some(def_name.as_str()));
                let bundled_def_name = logical_prefix
                    .as_deref()
                    .map(|prefix| format!("{prefix}/{def_name}"))
                    .unwrap_or(def_name);
                insert_definition(
                    &mut definitions,
                    bundled_def_name,
                    def_schema,
                    "root definitions",
                )?;
            }
        }

        if let Some(prefix) = logical_prefix.as_deref() {
            prefix_schema_definition_refs(&mut value, prefix);
        }

        insert_definition(&mut definitions, logical_name, value, "root definitions")?;
    }

    let mut root = Map::new();
    root.insert(
        "$schema".to_string(),
        Value::String("http://json-schema.org/draft-07/schema#".into()),
    );
    root.insert(
        "title".to_string(),
        Value::String("CodexAppServerProtocol".into()),
    );
    root.insert("type".to_string(), Value::String("object".into()));
    root.insert("definitions".to_string(), Value::Object(definitions));

    Ok(Value::Object(root))
}

fn insert_definition(
    definitions: &mut Map<String, Value>,
    name: String,
    schema: Value,
    location: &str,
) -> Result<()> {
    if let Some(existing) = definitions.get(&name) {
        if existing == &schema || schemas_equal_ignoring_definition_metadata(existing, &schema) {
            return Ok(());
        }

        let existing_title = existing
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("<untitled>");
        let new_title = schema
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("<untitled>");
        return Err(anyhow!(
            "schema definition collision in {location}: {name} (existing title: {existing_title}, new title: {new_title}); use #[schemars(rename = \"...\")] to rename one of the conflicting schema definitions"
        ));
    }

    definitions.insert(name, schema);
    Ok(())
}

fn schemas_equal_ignoring_definition_metadata(left: &Value, right: &Value) -> bool {
    fn strip_metadata(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut stripped = Map::new();
                for (key, child) in map {
                    if matches!(key.as_str(), "$schema" | "title" | "description") {
                        continue;
                    }
                    stripped.insert(key.clone(), strip_metadata(child));
                }
                Value::Object(stripped)
            }
            Value::Array(items) => Value::Array(items.iter().map(strip_metadata).collect()),
            _ => value.clone(),
        }
    }

    strip_metadata(left) == strip_metadata(right)
}

fn prefix_schema_definition_refs(value: &mut Value, prefix: &str) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref")
                && let Some(definition_name) = reference.strip_prefix("#/definitions/")
                && !definition_name.contains('/')
            {
                *reference = format!("#/definitions/{prefix}/{definition_name}");
            }

            for child in map.values_mut() {
                prefix_schema_definition_refs(child, prefix);
            }
        }
        Value::Array(items) => {
            for item in items {
                prefix_schema_definition_refs(item, prefix);
            }
        }
        _ => {}
    }
}

fn write_json_schema_with_return<T>(out_dir: &Path, name: &str) -> Result<GeneratedSchema>
where
    T: JsonSchema,
{
    let file_stem = name.trim();
    let (_, logical_name) = split_namespace(file_stem);
    let include_in_json_codegen = true;
    let schema = schema_for!(T);
    let mut schema_value = serde_json::to_value(schema)?;
    if include_in_json_codegen {
        enforce_numbered_definition_collision_overrides(file_stem, &mut schema_value);
        normalize_inter_agent_communication_schema(&mut schema_value);
        annotate_schema(&mut schema_value, Some(file_stem));
    }
    let out_path = out_dir.join(format!("{logical_name}.json"));

    if include_in_json_codegen && !IGNORED_DEFINITIONS.contains(&logical_name) {
        write_pretty_json(out_path, &schema_value)
            .with_context(|| format!("Failed to write JSON schema for {file_stem}"))?;
    }

    Ok(GeneratedSchema {
        logical_name: logical_name.to_string(),
        value: schema_value,
    })
}

fn enforce_numbered_definition_collision_overrides(schema_name: &str, schema: &mut Value) {
    for defs_key in ["definitions", "$defs"] {
        let Some(defs) = schema.get(defs_key).and_then(Value::as_object) else {
            continue;
        };
        detect_numbered_definition_collisions(schema_name, defs_key, defs);
    }
}

fn normalize_inter_agent_communication_schema(schema: &mut Value) {
    add_required_fields_to_named_definitions(
        schema,
        "InterAgentCommunication",
        &[
            "author",
            "recipient",
            "other_recipients",
            "content",
            "operation",
            "trigger_turn",
            "sender_thread_id",
            "recipient_thread_id",
            "status",
        ],
    );
}

fn add_required_fields_to_named_definitions(
    value: &mut Value,
    definition_name: &str,
    required_fields: &[&str],
) {
    match value {
        Value::Object(map) => {
            if let Some(definition) = map.get_mut(definition_name) {
                add_required_fields(definition, required_fields);
            }
            for child in map.values_mut() {
                add_required_fields_to_named_definitions(child, definition_name, required_fields);
            }
        }
        Value::Array(items) => {
            for child in items {
                add_required_fields_to_named_definitions(child, definition_name, required_fields);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn add_required_fields(schema: &mut Value, required_fields: &[&str]) {
    let Some(schema_obj) = schema.as_object_mut() else {
        return;
    };
    let required = schema_obj
        .entry("required".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(required) = required.as_array_mut() else {
        return;
    };
    for field in required_fields {
        let field = Value::String((*field).to_string());
        if !required.contains(&field) {
            required.push(field);
        }
    }
    required.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
}

fn detect_numbered_definition_collisions(
    schema_name: &str,
    defs_key: &str,
    defs: &Map<String, Value>,
) {
    for generated_name in defs.keys() {
        let base_name = generated_name.trim_end_matches(|c: char| c.is_ascii_digit());
        if base_name == generated_name || !defs.contains_key(base_name) {
            continue;
        }

        panic!(
            "Numbered definition naming collision detected: schema={schema_name}|container={defs_key}|generated={generated_name}|base={base_name}"
        );
    }
}

pub(crate) fn write_json_schema<T>(out_dir: &Path, name: &str) -> Result<GeneratedSchema>
where
    T: JsonSchema,
{
    write_json_schema_with_return::<T>(out_dir, name)
}

fn write_pretty_json(path: PathBuf, value: &impl Serialize) -> Result<()> {
    let json = serde_json::to_vec_pretty(value)
        .with_context(|| format!("Failed to serialize JSON schema to {}", path.display()))?;
    fs::write(&path, json).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Split a fully-qualified type name like "protocol::Type" into its namespace and logical name.
fn split_namespace(name: &str) -> (Option<&str>, &str) {
    name.split_once("::")
        .map_or((None, name), |(ns, rest)| (Some(ns), rest))
}

fn variant_definition_name(base: &str, variant: &Value) -> Option<String> {
    if let Some(props) = variant.get("properties").and_then(Value::as_object) {
        if let Some(method_literal) = literal_from_property(props, "method") {
            let pascal = to_pascal_case(method_literal);
            return Some(match base {
                "ClientRequest" | "ServerRequest" => format!("{pascal}Request"),
                "ClientNotification" | "ServerNotification" => format!("{pascal}Notification"),
                _ => format!("{pascal}{base}"),
            });
        }

        if let Some(type_literal) = literal_from_property(props, "type") {
            let pascal = to_pascal_case(type_literal);
            return Some(match base {
                "EventMsg" => format!("{pascal}EventMsg"),
                _ => format!("{pascal}{base}"),
            });
        }

        if props.len() == 1
            && let Some(key) = props.keys().next()
        {
            let pascal = props
                .get(key)
                .and_then(string_literal)
                .map(to_pascal_case)
                .unwrap_or_else(|| to_pascal_case(key));
            return Some(format!("{pascal}{base}"));
        }
    }

    if let Some(required) = variant.get("required").and_then(Value::as_array)
        && required.len() == 1
        && let Some(key) = required[0].as_str()
    {
        let pascal = to_pascal_case(key);
        return Some(format!("{pascal}{base}"));
    }

    None
}

fn literal_from_property<'a>(props: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    props.get(key).and_then(string_literal)
}

fn string_literal(value: &Value) -> Option<&str> {
    value.get("const").and_then(Value::as_str).or_else(|| {
        value
            .get("enum")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(Value::as_str)
    })
}

fn annotate_schema(value: &mut Value, base: Option<&str>) {
    match value {
        Value::Object(map) => annotate_object(map, base),
        Value::Array(items) => {
            for item in items {
                annotate_schema(item, base);
            }
        }
        _ => {}
    }
}

fn annotate_object(map: &mut Map<String, Value>, base: Option<&str>) {
    let owner = map.get("title").and_then(Value::as_str).map(str::to_owned);
    if let Some(owner) = owner.as_deref()
        && let Some(Value::Object(props)) = map.get_mut("properties")
    {
        set_discriminator_titles(props, owner);
    }

    if let Some(Value::Array(variants)) = map.get_mut("oneOf") {
        annotate_variant_list(variants, base);
    }
    if let Some(Value::Array(variants)) = map.get_mut("anyOf") {
        annotate_variant_list(variants, base);
    }

    if let Some(Value::Object(defs)) = map.get_mut("definitions") {
        for (name, schema) in defs.iter_mut() {
            annotate_schema(schema, Some(name.as_str()));
        }
    }

    if let Some(Value::Object(defs)) = map.get_mut("$defs") {
        for (name, schema) in defs.iter_mut() {
            annotate_schema(schema, Some(name.as_str()));
        }
    }

    if let Some(Value::Object(props)) = map.get_mut("properties") {
        for value in props.values_mut() {
            annotate_schema(value, base);
        }
    }

    if let Some(items) = map.get_mut("items") {
        annotate_schema(items, base);
    }

    if let Some(additional) = map.get_mut("additionalProperties") {
        annotate_schema(additional, base);
    }

    for (key, child) in map.iter_mut() {
        match key.as_str() {
            "oneOf"
            | "anyOf"
            | "definitions"
            | "$defs"
            | "properties"
            | "items"
            | "additionalProperties" => {}
            _ => annotate_schema(child, base),
        }
    }
}

fn annotate_variant_list(variants: &mut [Value], base: Option<&str>) {
    let mut seen = HashSet::new();

    for variant in variants.iter() {
        if let Some(name) = variant_title(variant) {
            seen.insert(name.to_owned());
        }
    }

    for variant in variants.iter_mut() {
        let mut variant_name = variant_title(variant).map(str::to_owned);

        if variant_name.is_none()
            && let Some(base_name) = base
            && let Some(name) = variant_definition_name(base_name, variant)
        {
            let candidate = name.clone();
            if seen.contains(&candidate) {
                let collision_key = variant_title_collision_key(base_name, &name, variant);
                panic!(
                    "Variant title naming collision detected: {collision_key} (generated name: {name})"
                );
            }
            if let Some(obj) = variant.as_object_mut() {
                obj.insert("title".into(), Value::String(candidate.clone()));
            }
            seen.insert(candidate.clone());
            variant_name = Some(candidate);
        }

        if let Some(name) = variant_name.as_deref()
            && let Some(obj) = variant.as_object_mut()
            && let Some(Value::Object(props)) = obj.get_mut("properties")
        {
            set_discriminator_titles(props, name);
        }

        annotate_schema(variant, base);
    }
}

fn variant_title_collision_key(base: &str, generated_name: &str, variant: &Value) -> String {
    let mut parts = vec![
        format!("base={base}"),
        format!("generated={generated_name}"),
    ];

    if let Some(props) = variant.get("properties").and_then(Value::as_object) {
        for key in DISCRIMINATOR_KEYS {
            if let Some(value) = literal_from_property(props, key) {
                parts.push(format!("{key}={value}"));
            }
        }
        for (key, value) in props {
            if DISCRIMINATOR_KEYS.contains(&key.as_str()) {
                continue;
            }
            if let Some(literal) = string_literal(value) {
                parts.push(format!("literal:{key}={literal}"));
            }
        }

        if props.len() == 1
            && let Some(key) = props.keys().next()
        {
            parts.push(format!("only_property={key}"));
        }
    }

    if let Some(required) = variant.get("required").and_then(Value::as_array)
        && required.len() == 1
        && let Some(key) = required[0].as_str()
    {
        parts.push(format!("required_only={key}"));
    }

    if parts.len() == 2 {
        parts.push(format!("variant={variant}"));
    }

    parts.join("|")
}

const DISCRIMINATOR_KEYS: &[&str] = &["type", "method", "mode", "status", "role", "reason"];

fn set_discriminator_titles(props: &mut Map<String, Value>, owner: &str) {
    for key in DISCRIMINATOR_KEYS {
        if let Some(prop_schema) = props.get_mut(*key)
            && string_literal(prop_schema).is_some()
            && let Value::Object(prop_obj) = prop_schema
        {
            if prop_obj.contains_key("title") {
                continue;
            }
            let suffix = to_pascal_case(key);
            prop_obj.insert("title".into(), Value::String(format!("{owner}{suffix}")));
        }
    }
}

fn variant_title(value: &Value) -> Option<&str> {
    value
        .as_object()
        .and_then(|obj| obj.get("title"))
        .and_then(Value::as_str)
}

fn to_pascal_case(input: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in input.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
            continue;
        }

        if capitalize_next {
            result.extend(c.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create output directory {}", dir.display()))
}

fn prepend_header_if_missing(path: &Path) -> Result<()> {
    let mut content = String::new();
    {
        let mut f = fs::File::open(path)
            .with_context(|| format!("Failed to open {} for reading", path.display()))?;
        f.read_to_string(&mut content)
            .with_context(|| format!("Failed to read {}", path.display()))?;
    }

    if content.starts_with(GENERATED_TS_HEADER) {
        return Ok(());
    }

    let mut f = fs::File::create(path)
        .with_context(|| format!("Failed to open {} for writing", path.display()))?;
    f.write_all(GENERATED_TS_HEADER.as_bytes())
        .with_context(|| format!("Failed to write header to {}", path.display()))?;
    f.write_all(content.as_bytes())
        .with_context(|| format!("Failed to write content to {}", path.display()))?;
    Ok(())
}

fn ts_files_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in
        fs::read_dir(dir).with_context(|| format!("Failed to read dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension() == Some(OsStr::new("ts")) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn ts_files_in_recursive(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in
            fs::read_dir(&d).with_context(|| format!("Failed to read dir {}", d.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() && path.extension() == Some(OsStr::new("ts")) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn trim_trailing_whitespace_in_ts_files(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let trimmed = trim_trailing_line_whitespace(&content);
        if trimmed != content {
            fs::write(path, trimmed)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }
    }
    Ok(())
}

pub(crate) fn trim_trailing_line_whitespace(content: &str) -> String {
    let mut trimmed = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        if let Some(line_without_newline) = line.strip_suffix('\n') {
            trimmed.push_str(line_without_newline.trim_end_matches([' ', '\t']));
            trimmed.push('\n');
        } else {
            trimmed.push_str(line.trim_end_matches([' ', '\t']));
        }
    }
    trimmed
}

/// Generate an index.ts file that re-exports all generated types.
/// This allows consumers to import all types from a single file.
fn generate_index_ts(out_dir: &Path) -> Result<PathBuf> {
    let content = generated_index_ts_with_header(index_ts_entries(
        &ts_files_in(out_dir)?
            .iter()
            .map(PathBuf::as_path)
            .collect::<Vec<_>>(),
    ));

    let index_path = out_dir.join("index.ts");
    let mut f = fs::File::create(&index_path)
        .with_context(|| format!("Failed to create {}", index_path.display()))?;
    f.write_all(content.as_bytes())
        .with_context(|| format!("Failed to write {}", index_path.display()))?;
    Ok(index_path)
}

pub(crate) fn generate_index_ts_tree(tree: &mut BTreeMap<PathBuf, String>) {
    let root_entries = tree
        .keys()
        .filter(|path| path.components().count() == 1)
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    tree.insert(
        PathBuf::from("index.ts"),
        index_ts_entries(&root_entries),
    );
}

fn generated_index_ts_with_header(content: String) -> String {
    let mut with_header = String::with_capacity(GENERATED_TS_HEADER.len() + content.len());
    with_header.push_str(GENERATED_TS_HEADER);
    with_header.push_str(&content);
    with_header
}

fn index_ts_entries(paths: &[&Path]) -> String {
    let mut stems: Vec<String> = paths
        .iter()
        .filter(|path| path.extension() == Some(OsStr::new("ts")))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            if stem == "index" { None } else { Some(stem) }
        })
        .filter(|stem| stem != "EventMsg")
        .collect();
    stems.sort();
    stems.dedup();

    let mut entries = String::new();
    for name in stems {
        entries.push_str(&format!("export type {{ {name} }} from \"./{name}\";\n"));
    }
    entries
}

#[cfg(test)]
mod tests;
