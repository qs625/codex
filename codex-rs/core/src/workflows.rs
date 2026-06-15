use crate::config::Config;
use codex_app_server_protocol::ConfigLayerSource;
use codex_config::ConfigLayerStackOrdering;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

const WORKFLOW_INSTRUCTIONS_FILE: &str = "WORKFLOW.md";
const MAX_WORKFLOW_INSTRUCTIONS_BYTES: usize = 16 * 1024;
const MAX_WORKFLOWS_PER_SOURCE: usize = 100;
const MAX_CONTEXT_FIELD_CHARS: usize = 600;
const MAX_CONTEXT_INSTRUCTIONS_CHARS: usize = 2_000;
const MAX_AVAILABLE_WORKFLOWS_CONTEXT_CHARS: usize = 24_000;
const TRUNCATED_NOTICE: &str = "... [truncated]";

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

pub fn load_workflow_registry(config: &Config) -> WorkflowRegistry {
    load_workflow_registry_from_roots(
        config.codex_home.join("workflows").to_path_buf(),
        project_workflow_roots(config),
    )
}

fn load_workflow_registry_from_roots(
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
                let existing = by_id
                    .remove(&workflow.id)
                    .expect("existing workflow should be present");
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

fn project_workflow_roots(config: &Config) -> Vec<PathBuf> {
    let layers = config.config_layer_stack.get_layers(
        ConfigLayerStackOrdering::LowestPrecedenceFirst,
        /*include_disabled*/ false,
    );
    if layers.is_empty() {
        return vec![config.cwd.join(".codex").join("workflows").to_path_buf()];
    }

    let roots = layers
        .into_iter()
        .filter(|layer| matches!(&layer.name, ConfigLayerSource::Project { .. }))
        .filter_map(codex_config::ConfigLayerEntry::config_folder)
        .map(|folder| folder.join("workflows").to_path_buf())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        vec![config.cwd.join(".codex").join("workflows").to_path_buf()]
    } else {
        roots
    }
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
    let (frontmatter, body) =
        extract_workflow_instructions_frontmatter(&text, &instructions_path)?;
    let manifest: WorkflowManifest = serde_yaml::from_str(frontmatter)
        .map_err(|err| format!("failed to parse {WORKFLOW_INSTRUCTIONS_FILE} frontmatter: {err}"))?;
    validate_workflow_manifest(&manifest)?;
    Ok(WorkflowMarkdown {
        manifest,
        body: truncate_instructions(body.trim_start().to_string()),
    })
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
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
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

pub(crate) fn render_available_workflows_body(registry: &WorkflowRegistry) -> Option<String> {
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
        if !workflow.instructions.trim().is_empty() {
            entry.push_str(&format!(
                "  Instructions:\n{}\n",
                indent_instructions_for_context(&workflow.instructions)
            ));
        }
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
            if used_chars + notice_chars + 1 <= MAX_AVAILABLE_WORKFLOWS_CONTEXT_CHARS {
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

fn truncate_instructions_for_context(value: &str) -> String {
    if value.chars().count() <= MAX_CONTEXT_INSTRUCTIONS_CHARS {
        return value.to_string();
    }
    let keep = MAX_CONTEXT_INSTRUCTIONS_CHARS.saturating_sub(TRUNCATED_NOTICE.len());
    format!(
        "{}{TRUNCATED_NOTICE}",
        value.chars().take(keep).collect::<String>()
    )
}

fn indent_instructions_for_context(value: &str) -> String {
    truncate_instructions_for_context(value)
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
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
        fs::write(dir.join(WORKFLOW_INSTRUCTIONS_FILE), "# Missing frontmatter")
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
    fn rendered_context_includes_workflow_name_and_instructions() {
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
        assert!(body.contains("Instructions:\n    # feature-dev"));
    }

    #[test]
    fn rendered_context_has_total_budget() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workflows_root = temp.path().join("repo/.codex/workflows");
        for index in 0..40 {
            let id = format!("workflow-{index:03}");
            let dir = workflows_root.join(&id);
            fs::create_dir_all(&dir).expect("create workflow dir");
            fs::write(dir.join("workflow.ts"), "export default {};")
                .expect("write workflow entry");
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
