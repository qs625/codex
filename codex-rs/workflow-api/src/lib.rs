mod runtime;
mod tool_contract;

pub use runtime::DisabledWorkflowRunController;
pub use runtime::WorkflowAgentBinding;
pub use runtime::WorkflowApi;
pub use runtime::WorkflowRun;
pub use runtime::WorkflowRunController;
pub use runtime::WorkflowRunFuture;
pub use runtime::WorkflowRunResult;
pub use runtime::WorkflowRunStatus;
pub use runtime::WorkflowRunUpdateError;
pub use runtime::WorkflowRunUpdateFuture;
pub use runtime::WorkflowRunUpdateReceiver;
pub use tool_contract::WorkflowAbortArgs;
pub use tool_contract::WorkflowDescribeArgs;
pub use tool_contract::WorkflowFollowupTaskToolCall;
pub use tool_contract::WorkflowPollEventToolCall;
pub use tool_contract::WorkflowResumeArgs;
pub use tool_contract::WorkflowSpawnAgentToolCall;
pub use tool_contract::WorkflowStartArgs;
pub use tool_contract::WorkflowStatusArgs;
pub use tool_contract::WorkflowWaitAgentToolCall;
pub use tool_contract::workflow_followup_task_tool_call;
pub use tool_contract::workflow_poll_event_tool_call;
pub use tool_contract::workflow_spawn_agent_tool_call;
pub use tool_contract::workflow_tool_call_id;
pub use tool_contract::workflow_tool_output_json;
pub use tool_contract::workflow_wait_agent_tool_call;

use codex_config_state::ConfigLayerEntry;
use codex_config_types::ConfigLayerSource;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::future::Future;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use thread_service_api::ThreadTurnCapability;

const WORKFLOW_INSTRUCTIONS_FILE: &str = "WORKFLOW.md";
const MAX_WORKFLOW_INSTRUCTIONS_BYTES: usize = 16 * 1024;
const MAX_WORKFLOWS_PER_SOURCE: usize = 100;
const MAX_CONTEXT_FIELD_CHARS: usize = 600;
const MAX_AVAILABLE_WORKFLOWS_CONTEXT_CHARS: usize = 24_000;
const TRUNCATED_NOTICE: &str = "... [truncated]";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDiscoveryContext {
    pub home_root: PathBuf,
    #[serde(default)]
    pub project_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeRequest {
    pub run_id: String,
    pub workflow_id: String,
    pub rpc_id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeError {
    pub code: String,
    pub message: String,
}

impl WorkflowRuntimeError {
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self {
            code: "unsupported".to_string(),
            message: message.into(),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_request".to_string(),
            message: message.into(),
        }
    }
}

pub trait WorkflowRuntimeBridge: Send + Sync {
    fn call(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Value, WorkflowRuntimeError>> + Send + '_>>;
}

pub type WorkflowProgressFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub trait WorkflowProgressSink: Send + Sync + 'static {
    #[allow(clippy::too_many_arguments)]
    fn record_workflow_progress<'a>(
        &'a self,
        run_id: &'a str,
        workflow_id: &'a str,
        status: Value,
        runner_status: Option<String>,
        kind: protocol::models::WorkflowRunProgressKind,
        message: Option<String>,
        updated_at: i64,
    ) -> WorkflowProgressFuture<'a>;
}

#[derive(Clone)]
pub struct WorkflowExecutionContext {
    discovery: WorkflowDiscoveryContext,
    turn: Option<Arc<dyn ThreadTurnCapability>>,
}

impl WorkflowExecutionContext {
    pub fn new(
        discovery: WorkflowDiscoveryContext,
        turn: Option<Arc<dyn ThreadTurnCapability>>,
    ) -> Self {
        Self { discovery, turn }
    }

    pub fn discovery(&self) -> &WorkflowDiscoveryContext {
        &self.discovery
    }

    pub fn turn(&self) -> Option<Arc<dyn ThreadTurnCapability>> {
        self.turn.clone()
    }
}

impl From<thread_service_api::ThreadDiscoveryContext> for WorkflowDiscoveryContext {
    fn from(value: thread_service_api::ThreadDiscoveryContext) -> Self {
        Self {
            home_root: value.home_root,
            project_roots: value.project_roots,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowSource {
    Home,
    Project,
}

impl WorkflowSource {
    fn label(self) -> &'static str {
        match self {
            WorkflowSource::Home => "home",
            WorkflowSource::Project => "project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInputSpec {
    #[serde(rename = "type")]
    pub input_type: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entry: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, alias = "when_to_use")]
    pub when_to_use: Vec<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, WorkflowInputSpec>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: WorkflowSource,
    pub path: String,
    pub entry: String,
    pub version: Option<String>,
    pub when_to_use: Vec<String>,
    pub inputs: BTreeMap<String, WorkflowInputSpec>,
    #[serde(default, skip)]
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDetails {
    #[serde(flatten)]
    pub summary: WorkflowSummary,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowMarkdown {
    manifest: WorkflowManifest,
    body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDiagnostic {
    pub source: WorkflowSource,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRegistry {
    pub workflows: Vec<WorkflowSummary>,
    pub diagnostics: Vec<WorkflowDiagnostic>,
}

pub fn load_workflow_registry_from_roots(
    home_root: PathBuf,
    project_roots: Vec<PathBuf>,
) -> WorkflowRegistry {
    let mut diagnostics = Vec::new();
    let mut all = Vec::new();
    all.extend(load_source(
        WorkflowSource::Home,
        &home_root,
        &mut diagnostics,
    ));
    for root in project_roots {
        all.extend(load_source(
            WorkflowSource::Project,
            &root,
            &mut diagnostics,
        ));
    }

    let mut by_id = BTreeMap::<String, WorkflowSummary>::new();
    let mut invalid_same_source_ids = HashSet::<(WorkflowSource, String)>::new();
    for workflow in all {
        if invalid_same_source_ids.contains(&(workflow.source, workflow.id.clone())) {
            diagnostics.push(WorkflowDiagnostic {
                source: workflow.source,
                path: workflow.path.clone(),
                message: format!(
                    "workflow `{}` duplicates another {} workflow",
                    workflow.id,
                    workflow.source.label()
                ),
            });
            continue;
        }
        match by_id.get(&workflow.id) {
            Some(existing) if existing.source == workflow.source => {
                let Some(existing) = by_id.remove(&workflow.id) else {
                    diagnostics.push(WorkflowDiagnostic {
                        source: workflow.source,
                        path: workflow.path.clone(),
                        message: format!(
                            "workflow `{}` duplicates another {} workflow",
                            workflow.id,
                            workflow.source.label()
                        ),
                    });
                    continue;
                };
                diagnostics.push(WorkflowDiagnostic {
                    source: existing.source,
                    path: existing.path.clone(),
                    message: format!(
                        "workflow `{}` duplicates workflow at `{}`",
                        existing.id, workflow.path
                    ),
                });
                diagnostics.push(WorkflowDiagnostic {
                    source: workflow.source,
                    path: workflow.path.clone(),
                    message: format!(
                        "workflow `{}` duplicates workflow at `{}`",
                        workflow.id, existing.path
                    ),
                });
                invalid_same_source_ids.insert((workflow.source, workflow.id));
            }
            Some(existing) if workflow.source == WorkflowSource::Project => {
                diagnostics.push(WorkflowDiagnostic {
                    source: existing.source,
                    path: existing.path.clone(),
                    message: format!(
                        "workflow `{}` is shadowed by higher-precedence project workflow with the same id",
                        existing.id
                    ),
                });
                by_id.insert(workflow.id.clone(), workflow);
            }
            Some(existing) => {
                diagnostics.push(WorkflowDiagnostic {
                    source: workflow.source,
                    path: workflow.path.clone(),
                    message: format!(
                        "workflow `{}` duplicates an existing {} workflow",
                        workflow.id,
                        existing.source.label()
                    ),
                });
            }
            None => {
                by_id.insert(workflow.id.clone(), workflow);
            }
        }
    }

    WorkflowRegistry {
        workflows: by_id.into_values().collect(),
        diagnostics,
    }
}

pub fn load_workflow_registry(context: &WorkflowDiscoveryContext) -> WorkflowRegistry {
    load_workflow_registry_from_roots(context.home_root.clone(), context.project_roots.clone())
}

pub fn workflow_discovery_context_from_config_layers(
    codex_home: &Path,
    cwd: &Path,
    layers: Vec<ConfigLayerEntry>,
) -> WorkflowDiscoveryContext {
    WorkflowDiscoveryContext {
        home_root: codex_home.join("workflows"),
        project_roots: project_workflow_roots(cwd, layers),
    }
}

fn project_workflow_roots(cwd: &Path, layers: Vec<ConfigLayerEntry>) -> Vec<PathBuf> {
    if layers.is_empty() {
        return vec![cwd.join(".codex").join("workflows")];
    }

    let roots = layers
        .into_iter()
        .filter(|layer| matches!(&layer.name, ConfigLayerSource::Project { .. }))
        .filter_map(|layer| ConfigLayerEntry::config_folder(&layer))
        .map(|folder| folder.join("workflows").to_path_buf())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        vec![cwd.join(".codex").join("workflows")]
    } else {
        roots
    }
}

fn load_source(
    source: WorkflowSource,
    root: &Path,
    diagnostics: &mut Vec<WorkflowDiagnostic>,
) -> Vec<WorkflowSummary> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    paths.sort();
    if paths.len() > MAX_WORKFLOWS_PER_SOURCE {
        diagnostics.push(WorkflowDiagnostic {
            source,
            path: root.display().to_string(),
            message: format!(
                "only the first {MAX_WORKFLOWS_PER_SOURCE} workflow directories are loaded"
            ),
        });
        paths.truncate(MAX_WORKFLOWS_PER_SOURCE);
    }

    let mut by_id = BTreeMap::<String, WorkflowSummary>::new();
    let mut duplicate_ids = HashMap::<String, String>::new();
    for path in paths {
        match load_workflow(source, &path) {
            Ok(workflow) => {
                if let Some(existing_path) = duplicate_ids.get(&workflow.id) {
                    diagnostics.push(WorkflowDiagnostic {
                        source,
                        path: workflow.path.clone(),
                        message: format!(
                            "workflow `{}` duplicates workflow at `{existing_path}`",
                            workflow.id
                        ),
                    });
                    continue;
                }
                if let Some(existing) = by_id.remove(&workflow.id) {
                    diagnostics.push(WorkflowDiagnostic {
                        source,
                        path: existing.path.clone(),
                        message: format!(
                            "workflow `{}` duplicates workflow at `{}`",
                            existing.id, workflow.path
                        ),
                    });
                    diagnostics.push(WorkflowDiagnostic {
                        source,
                        path: workflow.path.clone(),
                        message: format!(
                            "workflow `{}` duplicates workflow at `{}`",
                            workflow.id, existing.path
                        ),
                    });
                    duplicate_ids.insert(workflow.id, existing.path);
                } else {
                    by_id.insert(workflow.id.clone(), workflow);
                }
            }
            Err(message) => diagnostics.push(WorkflowDiagnostic {
                source,
                path: path.display().to_string(),
                message,
            }),
        }
    }
    by_id.into_values().collect()
}

fn load_workflow(source: WorkflowSource, dir: &Path) -> Result<WorkflowSummary, String> {
    let workflow_markdown = load_workflow_markdown(dir)?;
    let manifest = workflow_markdown.manifest;
    let dir_name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "workflow directory name is not valid UTF-8".to_string())?;
    if manifest.id != dir_name {
        return Err(format!(
            "workflow id `{}` must match directory name `{dir_name}`",
            manifest.id
        ));
    }
    if manifest.entry.trim().is_empty() {
        return Err("workflow entry must not be empty".to_string());
    }
    if entry_escapes_workflow_dir(&manifest.entry) {
        return Err("workflow entry must stay inside the workflow directory".to_string());
    }
    if !entry_is_typescript(&manifest.entry) {
        return Err("workflow entry must be a TypeScript .ts file".to_string());
    }
    let entry_path = dir.join(&manifest.entry);
    if !entry_path.is_file() {
        return Err(format!(
            "workflow entry `{}` does not exist",
            manifest.entry
        ));
    }

    Ok(WorkflowSummary {
        id: manifest.id,
        name: manifest.name,
        description: manifest.description,
        source,
        path: dir.display().to_string(),
        entry: manifest.entry,
        version: manifest.version,
        when_to_use: manifest.when_to_use,
        inputs: manifest.inputs,
        instructions: workflow_markdown.body,
    })
}

fn load_workflow_markdown(dir: &Path) -> Result<WorkflowMarkdown, String> {
    let instructions_path = dir.join(WORKFLOW_INSTRUCTIONS_FILE);
    let text = fs::read_to_string(&instructions_path)
        .map_err(|err| format!("failed to read {WORKFLOW_INSTRUCTIONS_FILE}: {err}"))?;
    let (frontmatter, body) = extract_workflow_instructions_frontmatter(&text, &instructions_path)?;
    let manifest = parse_workflow_manifest_frontmatter(frontmatter).map_err(|err| {
        format!("failed to parse {WORKFLOW_INSTRUCTIONS_FILE} frontmatter: {err}")
    })?;
    validate_workflow_manifest(&manifest)?;
    Ok(WorkflowMarkdown {
        manifest,
        body: truncate_instructions(body.trim_start().to_string()),
    })
}

#[derive(Debug, Default)]
struct WorkflowManifestDraft {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    entry: Option<String>,
    version: Option<Option<String>>,
    when_to_use: Option<Vec<String>>,
    inputs: Option<BTreeMap<String, WorkflowInputSpecDraft>>,
}

#[derive(Debug, Default)]
struct WorkflowInputSpecDraft {
    input_type: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowManifestBlock {
    None,
    WhenToUse,
    Inputs,
    Input,
    Unknown,
}

fn parse_workflow_manifest_frontmatter(frontmatter: &str) -> Result<WorkflowManifest, String> {
    let mut draft = WorkflowManifestDraft::default();
    let mut block = WorkflowManifestBlock::None;
    let mut current_input: Option<String> = None;

    for (line_index, raw_line) in frontmatter.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed_start = line.trim_start();
        if trimmed_start.is_empty() || trimmed_start.starts_with('#') {
            continue;
        }

        let indent = workflow_manifest_indent(line, line_number)?;
        let trimmed = &line[indent..];
        if indent == 0 {
            current_input = None;
            let (key, value) = split_workflow_manifest_key_value(trimmed, line_number)?;
            match key {
                "id" => {
                    set_workflow_manifest_string(&mut draft.id, key, value, line_number)?;
                    block = WorkflowManifestBlock::None;
                }
                "name" => {
                    set_workflow_manifest_string(&mut draft.name, key, value, line_number)?;
                    block = WorkflowManifestBlock::None;
                }
                "description" => {
                    set_workflow_manifest_string(&mut draft.description, key, value, line_number)?;
                    block = WorkflowManifestBlock::None;
                }
                "entry" => {
                    set_workflow_manifest_string(&mut draft.entry, key, value, line_number)?;
                    block = WorkflowManifestBlock::None;
                }
                "version" => {
                    set_optional_workflow_manifest_string(
                        &mut draft.version,
                        key,
                        value,
                        line_number,
                    )?;
                    block = WorkflowManifestBlock::None;
                }
                "when_to_use" | "whenToUse" => {
                    set_workflow_manifest_list(&mut draft.when_to_use, key, value, line_number)?;
                    block = if value.trim().is_empty() {
                        WorkflowManifestBlock::WhenToUse
                    } else {
                        WorkflowManifestBlock::None
                    };
                }
                "inputs" => {
                    set_workflow_manifest_inputs(&mut draft.inputs, key, value, line_number)?;
                    block = if value.trim().is_empty() {
                        WorkflowManifestBlock::Inputs
                    } else {
                        WorkflowManifestBlock::None
                    };
                }
                _ => {
                    block = if value.trim().is_empty() {
                        WorkflowManifestBlock::Unknown
                    } else {
                        WorkflowManifestBlock::None
                    };
                }
            }
            continue;
        }

        match block {
            WorkflowManifestBlock::WhenToUse => {
                if indent != 2 {
                    return Err(format!(
                        "line {line_number}: `when_to_use` items must be indented by two spaces"
                    ));
                }
                let Some(item) = trimmed.strip_prefix("- ") else {
                    return Err(format!(
                        "line {line_number}: `when_to_use` entries must use `- value` list items"
                    ));
                };
                let entries = draft.when_to_use.get_or_insert_with(Vec::new);
                entries.push(parse_workflow_manifest_scalar(item, line_number)?);
            }
            WorkflowManifestBlock::Inputs | WorkflowManifestBlock::Input => {
                parse_workflow_manifest_input_line(
                    &mut draft.inputs,
                    &mut block,
                    &mut current_input,
                    indent,
                    trimmed,
                    line_number,
                )?;
            }
            WorkflowManifestBlock::Unknown => {}
            WorkflowManifestBlock::None => {
                return Err(format!(
                    "line {line_number}: unexpected indented manifest line"
                ));
            }
        }
    }

    let inputs = draft
        .inputs
        .unwrap_or_default()
        .into_iter()
        .map(|(name, input)| {
            let input_type = input.input_type.ok_or_else(|| {
                format!("input `{name}` field `type` is required in workflow manifest")
            })?;
            Ok((
                name,
                WorkflowInputSpec {
                    input_type,
                    description: input.description,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    Ok(WorkflowManifest {
        id: required_workflow_manifest_string(draft.id, "id")?,
        name: required_workflow_manifest_string(draft.name, "name")?,
        description: required_workflow_manifest_string(draft.description, "description")?,
        entry: required_workflow_manifest_string(draft.entry, "entry")?,
        version: draft.version.unwrap_or(None),
        when_to_use: draft.when_to_use.unwrap_or_default(),
        inputs,
    })
}

fn parse_workflow_manifest_input_line(
    inputs: &mut Option<BTreeMap<String, WorkflowInputSpecDraft>>,
    block: &mut WorkflowManifestBlock,
    current_input: &mut Option<String>,
    indent: usize,
    trimmed: &str,
    line_number: usize,
) -> Result<(), String> {
    match indent {
        2 => {
            let (input_name, value) = split_workflow_manifest_key_value(trimmed, line_number)?;
            if !value.trim().is_empty() {
                return Err(format!(
                    "line {line_number}: workflow input `{input_name}` must use nested fields"
                ));
            }
            let input_specs = inputs.get_or_insert_with(BTreeMap::new);
            if input_specs
                .insert(input_name.to_string(), WorkflowInputSpecDraft::default())
                .is_some()
            {
                return Err(format!(
                    "line {line_number}: duplicate input `{input_name}`"
                ));
            }
            *current_input = Some(input_name.to_string());
            *block = WorkflowManifestBlock::Input;
            Ok(())
        }
        4 => {
            let Some(input_name) = current_input.as_deref() else {
                return Err(format!(
                    "line {line_number}: workflow input field appears before an input name"
                ));
            };
            let (field, value) = split_workflow_manifest_key_value(trimmed, line_number)?;
            let input_specs = inputs.get_or_insert_with(BTreeMap::new);
            let input = input_specs.get_mut(input_name).ok_or_else(|| {
                format!("line {line_number}: workflow input `{input_name}` was not initialized")
            })?;
            match field {
                "type" => {
                    set_workflow_manifest_string(&mut input.input_type, field, value, line_number)
                }
                "description" => {
                    set_workflow_manifest_string(&mut input.description, field, value, line_number)
                }
                _ => Ok(()),
            }
        }
        _ => Err(format!(
            "line {line_number}: workflow inputs must use two-space indentation"
        )),
    }
}

fn workflow_manifest_indent(line: &str, line_number: usize) -> Result<usize, String> {
    let mut indent = 0;
    for byte in line.bytes() {
        match byte {
            b' ' => indent += 1,
            b'\t' => {
                return Err(format!(
                    "line {line_number}: tabs are not supported in workflow manifest indentation"
                ));
            }
            _ => break,
        }
    }
    Ok(indent)
}

fn split_workflow_manifest_key_value(
    line: &str,
    line_number: usize,
) -> Result<(&str, &str), String> {
    let Some((key, value)) = line.split_once(':') else {
        return Err(format!("line {line_number}: expected `key: value`"));
    };
    let key = key.trim();
    if key.is_empty() {
        return Err(format!(
            "line {line_number}: manifest key must not be empty"
        ));
    }
    Ok((key, value.trim_start()))
}

fn set_workflow_manifest_string(
    target: &mut Option<String>,
    field: &str,
    value: &str,
    line_number: usize,
) -> Result<(), String> {
    if target.is_some() {
        return Err(format!("line {line_number}: duplicate field `{field}`"));
    }
    *target = Some(parse_workflow_manifest_scalar(value, line_number)?);
    Ok(())
}

fn set_optional_workflow_manifest_string(
    target: &mut Option<Option<String>>,
    field: &str,
    value: &str,
    line_number: usize,
) -> Result<(), String> {
    if target.is_some() {
        return Err(format!("line {line_number}: duplicate field `{field}`"));
    }
    *target = if value.trim().is_empty() {
        Some(None)
    } else {
        Some(Some(parse_workflow_manifest_scalar(value, line_number)?))
    };
    Ok(())
}

fn set_workflow_manifest_list(
    target: &mut Option<Vec<String>>,
    field: &str,
    value: &str,
    line_number: usize,
) -> Result<(), String> {
    if target.is_some() {
        return Err(format!("line {line_number}: duplicate field `{field}`"));
    }
    match value.trim() {
        "" => {
            *target = Some(Vec::new());
            Ok(())
        }
        "[]" => {
            *target = Some(Vec::new());
            Ok(())
        }
        _ => Err(format!(
            "line {line_number}: field `{field}` must be a block list"
        )),
    }
}

fn set_workflow_manifest_inputs(
    target: &mut Option<BTreeMap<String, WorkflowInputSpecDraft>>,
    field: &str,
    value: &str,
    line_number: usize,
) -> Result<(), String> {
    if target.is_some() {
        return Err(format!("line {line_number}: duplicate field `{field}`"));
    }
    match value.trim() {
        "" => {
            *target = Some(BTreeMap::new());
            Ok(())
        }
        "{}" => {
            *target = Some(BTreeMap::new());
            Ok(())
        }
        _ => Err(format!(
            "line {line_number}: field `{field}` must be a block map"
        )),
    }
}

fn parse_workflow_manifest_scalar(value: &str, line_number: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!(
            "line {line_number}: scalar value must not be empty"
        ));
    }
    if value.starts_with('"') {
        let end = workflow_manifest_double_quote_end(value, line_number)?;
        validate_workflow_manifest_trailing_comment(&value[end + 1..], line_number)?;
        return serde_json::from_str::<String>(&value[..=end])
            .map_err(|err| format!("line {line_number}: invalid double-quoted scalar: {err}"));
    }
    if value.starts_with('\'') {
        let (parsed, trailing) = parse_workflow_manifest_single_quoted_scalar(value, line_number)?;
        validate_workflow_manifest_trailing_comment(trailing, line_number)?;
        return Ok(parsed);
    }
    if value.starts_with('[') || value.starts_with('{') {
        return Err(format!(
            "line {line_number}: inline YAML collections are not supported in workflow manifests"
        ));
    }
    let value = strip_workflow_manifest_inline_comment(value).trim_end();
    if value.is_empty() {
        return Err(format!(
            "line {line_number}: scalar value must not be empty"
        ));
    }
    Ok(value.to_string())
}

fn workflow_manifest_double_quote_end(value: &str, line_number: usize) -> Result<usize, String> {
    let mut escaped = false;
    for (index, ch) in value.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Ok(index),
            _ => {}
        }
    }
    Err(format!("line {line_number}: invalid double-quoted scalar"))
}

fn parse_workflow_manifest_single_quoted_scalar(
    value: &str,
    line_number: usize,
) -> Result<(String, &str), String> {
    let mut parsed = String::new();
    let mut index = 1;
    while index < value.len() {
        let Some(ch) = value[index..].chars().next() else {
            break;
        };
        if ch == '\'' {
            let next_index = index + ch.len_utf8();
            if value[next_index..].starts_with('\'') {
                parsed.push('\'');
                index = next_index + 1;
                continue;
            }
            return Ok((parsed, &value[next_index..]));
        }
        parsed.push(ch);
        index += ch.len_utf8();
    }
    Err(format!("line {line_number}: invalid single-quoted scalar"))
}

fn validate_workflow_manifest_trailing_comment(
    trailing: &str,
    line_number: usize,
) -> Result<(), String> {
    let trailing = trailing.trim();
    if trailing.is_empty() || trailing.starts_with('#') {
        return Ok(());
    }
    Err(format!(
        "line {line_number}: unexpected content after quoted scalar"
    ))
}

fn strip_workflow_manifest_inline_comment(value: &str) -> &str {
    for (index, ch) in value.char_indices() {
        let comment_starts = ch == '#'
            && (index == 0
                || value[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace));
        if comment_starts {
            return &value[..index];
        }
    }
    value
}

fn required_workflow_manifest_string(value: Option<String>, field: &str) -> Result<String, String> {
    value.ok_or_else(|| format!("field `{field}` is required in workflow manifest"))
}

fn validate_workflow_manifest(manifest: &WorkflowManifest) -> Result<(), String> {
    if manifest.id.trim().is_empty() {
        return Err(format!(
            "{WORKFLOW_INSTRUCTIONS_FILE} frontmatter field `id` must not be empty"
        ));
    }
    if manifest.name.trim().is_empty() {
        return Err(format!(
            "{WORKFLOW_INSTRUCTIONS_FILE} frontmatter field `name` must not be empty"
        ));
    }
    if manifest.description.trim().is_empty() {
        return Err(format!(
            "{WORKFLOW_INSTRUCTIONS_FILE} frontmatter field `description` must not be empty"
        ));
    }
    if manifest.entry.trim().is_empty() {
        return Err(format!(
            "{WORKFLOW_INSTRUCTIONS_FILE} frontmatter field `entry` must not be empty"
        ));
    }
    Ok(())
}

fn extract_workflow_instructions_frontmatter<'a>(
    text: &'a str,
    path: &Path,
) -> Result<(&'a str, &'a str), String> {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Err(format!(
            "{} must start with YAML frontmatter delimited by ---",
            path.display()
        ));
    };
    let Some((frontmatter, body)) = rest.split_once("\n---") else {
        return Err(format!(
            "{} must close YAML frontmatter with ---",
            path.display()
        ));
    };
    let body = body
        .strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body);
    Ok((frontmatter, body))
}

fn entry_is_typescript(entry: &str) -> bool {
    Path::new(entry).extension().and_then(|ext| ext.to_str()) == Some("ts")
}

fn entry_escapes_workflow_dir(entry: &str) -> bool {
    Path::new(entry).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

impl WorkflowRegistry {
    pub fn find(&self, id: &str) -> Option<&WorkflowSummary> {
        self.workflows.iter().find(|workflow| workflow.id == id)
    }

    pub fn details(&self, id: &str) -> Result<WorkflowDetails, String> {
        let summary = self
            .find(id)
            .ok_or_else(|| format!("unknown workflow `{id}`"))?
            .clone();
        Ok(WorkflowDetails {
            instructions: summary.instructions.clone(),
            summary,
        })
    }
}

fn truncate_instructions(instructions: String) -> String {
    if instructions.len() <= MAX_WORKFLOW_INSTRUCTIONS_BYTES {
        return instructions;
    }
    let mut end = MAX_WORKFLOW_INSTRUCTIONS_BYTES.saturating_sub(TRUNCATED_NOTICE.len());
    while !instructions.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}{TRUNCATED_NOTICE}", &instructions[..end])
}

pub fn render_available_workflows_body(registry: &WorkflowRegistry) -> Option<String> {
    if registry.workflows.is_empty() {
        return None;
    }

    let mut body = String::from(
        "\nWorkflows are scripted, resumable multi-agent procedures. Use `workflow_list` or `workflow_describe` when the user asks for a structured workflow and the task matches one of the entries below. Use `workflow_start`, `workflow_status`, `workflow_resume`, and `workflow_abort` to manage a workflow run.\n\n",
    );
    let mut used_chars = body.chars().count();
    for (index, workflow) in registry.workflows.iter().enumerate() {
        let mut entry = format!(
            "- {} ({})\n  Name: {}\n  Description: {}\n",
            workflow.id,
            workflow.source.label(),
            truncate_for_context(&workflow.name),
            truncate_for_context(&workflow.description)
        );
        if !workflow.when_to_use.is_empty() {
            entry.push_str(&format!(
                "  Use when: {}\n",
                truncate_for_context(&workflow.when_to_use.join("; "))
            ));
        }
        if !workflow.inputs.is_empty() {
            entry.push_str(&format!(
                "  Inputs: {}\n",
                workflow
                    .inputs
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        entry.push_str(&format!(
            "  Inspect: workflow_describe({{\"workflow\": \"{}\"}})\n",
            workflow.id
        ));
        let remaining = registry.workflows.len().saturating_sub(index);
        let omitted_notice = format!(
            "  ... {remaining} workflow(s) omitted because the workflow context budget was reached.\n"
        );
        let entry_chars = entry.chars().count();
        let notice_chars = omitted_notice.chars().count();
        if used_chars + entry_chars + notice_chars + 1 > MAX_AVAILABLE_WORKFLOWS_CONTEXT_CHARS {
            if used_chars + notice_chars < MAX_AVAILABLE_WORKFLOWS_CONTEXT_CHARS {
                body.push_str(&omitted_notice);
            }
            break;
        }
        body.push_str(&entry);
        used_chars += entry_chars;
    }
    if used_chars < MAX_AVAILABLE_WORKFLOWS_CONTEXT_CHARS {
        body.push('\n');
    }
    Some(body)
}

fn truncate_for_context(value: &str) -> String {
    if value.chars().count() <= MAX_CONTEXT_FIELD_CHARS {
        return value.to_string();
    }
    let keep = MAX_CONTEXT_FIELD_CHARS.saturating_sub(TRUNCATED_NOTICE.len());
    format!(
        "{}{TRUNCATED_NOTICE}",
        value.chars().take(keep).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn write_workflow(root: &Path, id: &str, description: &str) {
        let dir = root.join(id);
        fs::create_dir_all(&dir).expect("create workflow dir");
        fs::write(dir.join("workflow.ts"), "export default {};").expect("write workflow entry");
        fs::write(
            dir.join(WORKFLOW_INSTRUCTIONS_FILE),
            format!(
                r#"---
id: {id}
name: {id}
description: {description}
entry: workflow.ts
version: "0.1.0"
when_to_use:
  - when useful
inputs:
  objective:
    type: string
    description: goal
---
# {id}

Workflow instructions for {id}.
"#
            ),
        )
        .expect("write workflow markdown");
    }

    #[test]
    fn workflow_manifest_parser_supports_current_frontmatter_shape() {
        let manifest = parse_workflow_manifest_frontmatter(
            r#"id: feature-dev
name: "Feature Development"
description: 按调研、实现、review/fix、验证流程开发功能。
entry: workflow.ts
version: "0.1.0"
when_to_use:
  - 用户要求开发新功能
  - "需要多 agent 协作、review 和验证"
inputs:
  objective:
    type: string
    description: 要完成的开发目标
  cwd:
    type: string
    description: 执行 workflow 的 checkout 路径
"#,
        )
        .expect("parse workflow manifest");

        assert_eq!(
            manifest,
            WorkflowManifest {
                id: "feature-dev".to_string(),
                name: "Feature Development".to_string(),
                description: "按调研、实现、review/fix、验证流程开发功能。".to_string(),
                entry: "workflow.ts".to_string(),
                version: Some("0.1.0".to_string()),
                when_to_use: vec![
                    "用户要求开发新功能".to_string(),
                    "需要多 agent 协作、review 和验证".to_string(),
                ],
                inputs: BTreeMap::from([
                    (
                        "cwd".to_string(),
                        WorkflowInputSpec {
                            input_type: "string".to_string(),
                            description: Some("执行 workflow 的 checkout 路径".to_string()),
                        },
                    ),
                    (
                        "objective".to_string(),
                        WorkflowInputSpec {
                            input_type: "string".to_string(),
                            description: Some("要完成的开发目标".to_string()),
                        },
                    ),
                ]),
            }
        );
    }

    #[test]
    fn workflow_manifest_parser_supports_camel_case_alias_and_quoted_scalars() {
        let manifest = parse_workflow_manifest_frontmatter(
            r#"id: 'feature-dev' # workflow id
name: 'Feature ''Dev'''
description: "Build \"things\"" # quoted description
entry: workflow.ts
whenToUse:
  - 'quoted item' # list comment
inputs: {}
"#,
        )
        .expect("parse workflow manifest");

        assert_eq!(manifest.id, "feature-dev");
        assert_eq!(manifest.name, "Feature 'Dev'");
        assert_eq!(manifest.description, "Build \"things\"");
        assert_eq!(manifest.when_to_use, vec!["quoted item"]);
        assert!(manifest.inputs.is_empty());
    }

    #[test]
    fn workflow_manifest_parser_rejects_missing_input_type() {
        let err = parse_workflow_manifest_frontmatter(
            r#"id: feature-dev
name: Feature Dev
description: project description
entry: workflow.ts
inputs:
  objective:
    description: goal
"#,
        )
        .expect_err("manifest must reject input without type");

        assert_eq!(
            err,
            "input `objective` field `type` is required in workflow manifest"
        );
    }

    #[test]
    fn workflow_manifest_parser_rejects_inline_collections() {
        let err = parse_workflow_manifest_frontmatter(
            r#"id: feature-dev
name: Feature Dev
description: project description
entry: workflow.ts
when_to_use: [feature work]
"#,
        )
        .expect_err("manifest must reject inline list");

        assert!(err.contains("field `when_to_use` must be a block list"));
    }

    #[test]
    fn project_workflow_overrides_home_workflow() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("home");
        let cwd = temp.path().join("repo");
        write_workflow(
            &codex_home.join("workflows"),
            "feature-dev",
            "home description",
        );
        write_workflow(
            &cwd.join(".codex/workflows"),
            "feature-dev",
            "project description",
        );

        let registry = load_workflow_registry_from_roots(
            codex_home.join("workflows"),
            vec![cwd.join(".codex/workflows")],
        );

        assert_eq!(registry.workflows.len(), 1);
        assert_eq!(registry.workflows[0].source, WorkflowSource::Project);
        assert_eq!(registry.workflows[0].description, "project description");
        assert_eq!(registry.diagnostics.len(), 1);
    }

    #[test]
    fn duplicate_project_workflow_id_is_excluded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let lower = temp.path().join("repo/.codex/workflows");
        let higher = temp.path().join("repo/child/.codex/workflows");
        write_workflow(&lower, "feature-dev", "lower description");
        write_workflow(&higher, "feature-dev", "higher description");

        let registry = load_workflow_registry_from_roots(
            temp.path().join("home/workflows"),
            vec![lower, higher],
        );

        assert!(registry.workflows.is_empty());
        assert_eq!(registry.diagnostics.len(), 2);
        assert_eq!(render_available_workflows_body(&registry), None);
    }

    #[test]
    fn invalid_workflow_is_excluded_from_rendered_context() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("home");
        let cwd = temp.path().join("repo");
        let dir = cwd.join(".codex/workflows/bad");
        fs::create_dir_all(&dir).expect("create workflow dir");
        fs::write(
            dir.join(WORKFLOW_INSTRUCTIONS_FILE),
            "---
id: other
name: Bad
description: bad
entry: ../bad.ts
---
Bad.
",
        )
        .expect("write workflow instructions");

        let registry = load_workflow_registry_from_roots(
            codex_home.join("workflows"),
            vec![cwd.join(".codex/workflows")],
        );

        assert!(registry.workflows.is_empty());
        assert_eq!(registry.diagnostics.len(), 1);
        assert_eq!(render_available_workflows_body(&registry), None);
    }

    #[test]
    fn non_typescript_entry_is_invalid() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflows_root = temp.path().join("repo/.codex/workflows");
        let dir = workflows_root.join("bad");
        fs::create_dir_all(&dir).expect("create workflow dir");
        fs::write(dir.join("workflow.js"), "export default {};").expect("write workflow entry");
        fs::write(
            dir.join(WORKFLOW_INSTRUCTIONS_FILE),
            "---
id: bad
name: Bad
description: bad
entry: workflow.js
---
Bad.
",
        )
        .expect("write workflow instructions");

        let registry = load_workflow_registry_from_roots(
            temp.path().join("home/workflows"),
            vec![workflows_root],
        );

        assert!(registry.workflows.is_empty());
        assert_eq!(registry.diagnostics.len(), 1);
        assert_eq!(
            registry.diagnostics[0].message,
            "workflow entry must be a TypeScript .ts file"
        );
    }

    #[test]
    fn details_includes_workflow_instructions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("home");
        let cwd = temp.path().join("repo");
        let workflows_root = cwd.join(".codex/workflows");
        write_workflow(&workflows_root, "feature-dev", "project description");
        fs::write(
            workflows_root.join(format!("feature-dev/{WORKFLOW_INSTRUCTIONS_FILE}")),
            "---
id: feature-dev
name: Feature Dev
description: project description
entry: workflow.ts
---
# Feature Dev

Use this workflow for feature development.
",
        )
        .expect("write workflow instructions");

        let registry = load_workflow_registry_from_roots(
            codex_home.join("workflows"),
            vec![cwd.join(".codex/workflows")],
        );
        let details = registry.details("feature-dev").expect("workflow details");

        assert_eq!(details.summary.name, "Feature Dev");
        assert_eq!(
            details.instructions,
            "# Feature Dev\n\nUse this workflow for feature development.\n"
        );
    }

    #[test]
    fn details_truncates_large_workflow_instructions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflows_root = temp.path().join("repo/.codex/workflows");
        write_workflow(&workflows_root, "feature-dev", "project description");
        fs::write(
            workflows_root.join(format!("feature-dev/{WORKFLOW_INSTRUCTIONS_FILE}")),
            format!(
                "---
id: feature-dev
name: Feature Dev
description: project description
entry: workflow.ts
---
{}",
                "a".repeat(MAX_WORKFLOW_INSTRUCTIONS_BYTES + 100)
            ),
        )
        .expect("write large workflow instructions");

        let registry = load_workflow_registry_from_roots(
            temp.path().join("home/workflows"),
            vec![workflows_root],
        );
        let details = registry.details("feature-dev").expect("workflow details");

        assert!(details.instructions.len() <= MAX_WORKFLOW_INSTRUCTIONS_BYTES);
        assert!(details.instructions.ends_with(TRUNCATED_NOTICE));
    }

    #[test]
    fn workflow_instructions_frontmatter_is_required() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflows_root = temp.path().join("repo/.codex/workflows");
        let dir = workflows_root.join("bad");
        fs::create_dir_all(&dir).expect("create workflow dir");
        fs::write(dir.join("workflow.ts"), "export default {};").expect("write workflow entry");
        fs::write(
            dir.join(WORKFLOW_INSTRUCTIONS_FILE),
            "# Missing frontmatter",
        )
        .expect("write workflow instructions");

        let registry = load_workflow_registry_from_roots(
            temp.path().join("home/workflows"),
            vec![workflows_root],
        );

        assert!(registry.workflows.is_empty());
        assert_eq!(registry.diagnostics.len(), 1);
        assert!(
            registry.diagnostics[0]
                .message
                .contains("must start with YAML frontmatter")
        );
    }

    #[test]
    fn rendered_context_includes_workflow_frontmatter_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflows_root = temp.path().join("repo/.codex/workflows");
        write_workflow(&workflows_root, "feature-dev", "project description");

        let registry = load_workflow_registry_from_roots(
            temp.path().join("home/workflows"),
            vec![workflows_root],
        );
        let body = render_available_workflows_body(&registry).expect("rendered workflows");

        assert!(body.contains("- feature-dev (project)"));
        assert!(body.contains("Name: feature-dev"));
        assert!(body.contains("Description: project description"));
        assert!(body.contains("Use when: when useful"));
        assert!(!body.contains("Instructions:"));
        assert!(!body.contains("# feature-dev"));
    }

    #[test]
    fn rendered_context_has_total_budget() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflows_root = temp.path().join("repo/.codex/workflows");
        for index in 0..40 {
            let id = format!("workflow-{index:03}");
            let dir = workflows_root.join(&id);
            fs::create_dir_all(&dir).expect("create workflow dir");
            fs::write(dir.join("workflow.ts"), "export default {};").expect("write workflow entry");
            fs::write(
                dir.join(WORKFLOW_INSTRUCTIONS_FILE),
                format!(
                    r#"---
id: {id}
name: Workflow {index}
description: workflow {index} description
entry: workflow.ts
---
{}
"#,
                    "long instructions ".repeat(400)
                ),
            )
            .expect("write workflow markdown");
        }

        let registry = load_workflow_registry_from_roots(
            temp.path().join("home/workflows"),
            vec![workflows_root],
        );
        let body = render_available_workflows_body(&registry).expect("rendered workflows");

        assert!(body.chars().count() <= MAX_AVAILABLE_WORKFLOWS_CONTEXT_CHARS);
        assert!(body.contains("workflow-000"));
        assert!(body.contains("workflow context budget was reached"));
        assert!(!body.contains("workflow-039"));
    }
}
