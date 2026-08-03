use crate::system::system_cache_root_dir;
use codex_config_types::ConfigLayerSource;
use codex_file_system::ExecutorFileSystem;
use codex_file_system::LOCAL_FS;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use codex_utils_home_dir::home_dir;
use plugin_service_api::PluginSkillRoot;
use plugin_service_api::plugin_namespace_for_skill_path;
use protocol::protocol::Product;
use protocol::protocol::SkillScope;
use serde::Deserialize;
use skill_service_api::SkillConfigLayerStack;
use skill_service_api::SkillConfigLayerStackOrdering;
use skill_service_api::model::SkillDependencies;
use skill_service_api::model::SkillError;
use skill_service_api::model::SkillInterface;
use skill_service_api::model::SkillLoadOutcome;
use skill_service_api::model::SkillMetadata;
use skill_service_api::model::SkillPolicy;
use skill_service_api::model::SkillToolDependency;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::io;
use std::path::Component;
use std::path::PathBuf;
use std::sync::Arc;
use toml::Value as TomlValue;
use tracing::error;

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    metadata: SkillFrontmatterMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontmatterMetadata {
    #[serde(default, rename = "short-description")]
    short_description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct SkillMetadataFile {
    #[serde(default)]
    interface: Option<Interface>,
    #[serde(default)]
    dependencies: Option<Dependencies>,
    #[serde(default)]
    policy: Option<Policy>,
}

#[derive(Default)]
struct LoadedSkillMetadata {
    interface: Option<SkillInterface>,
    dependencies: Option<SkillDependencies>,
    policy: Option<SkillPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct Interface {
    display_name: Option<String>,
    short_description: Option<String>,
    icon_small: Option<PathBuf>,
    icon_large: Option<PathBuf>,
    brand_color: Option<String>,
    default_prompt: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Dependencies {
    #[serde(default)]
    tools: Vec<DependencyTool>,
}

#[derive(Debug, Deserialize)]
struct Policy {
    #[serde(default)]
    allow_implicit_invocation: Option<bool>,
    #[serde(default)]
    products: Vec<Product>,
}

#[derive(Debug, Default, Deserialize)]
struct DependencyTool {
    #[serde(rename = "type")]
    kind: Option<String>,
    value: Option<String>,
    description: Option<String>,
    transport: Option<String>,
    command: Option<String>,
    url: Option<String>,
}

const SKILLS_FILENAME: &str = "SKILL.md";
const AGENTS_DIR_NAME: &str = ".agents";
const SKILLS_METADATA_DIR: &str = "agents";
const SKILLS_METADATA_FILENAME: &str = "openai.yaml";
const SKILLS_DIR_NAME: &str = "skills";
const MAX_NAME_LEN: usize = 64;
const MAX_DESCRIPTION_LEN: usize = 1024;
const MAX_SHORT_DESCRIPTION_LEN: usize = MAX_DESCRIPTION_LEN;
const MAX_DEFAULT_PROMPT_LEN: usize = MAX_DESCRIPTION_LEN;
const MAX_DEPENDENCY_TYPE_LEN: usize = MAX_NAME_LEN;
const MAX_DEPENDENCY_TRANSPORT_LEN: usize = MAX_NAME_LEN;
const MAX_DEPENDENCY_VALUE_LEN: usize = MAX_DESCRIPTION_LEN;
const MAX_DEPENDENCY_DESCRIPTION_LEN: usize = MAX_DESCRIPTION_LEN;
const MAX_DEPENDENCY_COMMAND_LEN: usize = MAX_DESCRIPTION_LEN;
const MAX_DEPENDENCY_URL_LEN: usize = MAX_DESCRIPTION_LEN;
// Traversal depth from the skills root.
const MAX_SCAN_DEPTH: usize = 6;
const MAX_SKILLS_DIRS_PER_ROOT: usize = 2000;

#[derive(Debug)]
enum SkillParseError {
    Read(std::io::Error),
    MissingFrontmatter,
    InvalidYaml(String),
    MissingField(&'static str),
    InvalidField { field: &'static str, reason: String },
}

impl fmt::Display for SkillParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkillParseError::Read(e) => write!(f, "failed to read file: {e}"),
            SkillParseError::MissingFrontmatter => {
                write!(f, "missing YAML frontmatter delimited by ---")
            }
            SkillParseError::InvalidYaml(e) => write!(f, "invalid YAML: {e}"),
            SkillParseError::MissingField(field) => write!(f, "missing field `{field}`"),
            SkillParseError::InvalidField { field, reason } => {
                write!(f, "invalid {field}: {reason}")
            }
        }
    }
}

impl Error for SkillParseError {}

pub struct SkillRoot {
    pub path: AbsolutePathBuf,
    pub scope: SkillScope,
    pub file_system: Arc<dyn ExecutorFileSystem>,
    pub plugin_id: Option<String>,
}

pub async fn load_skills_from_roots<I>(roots: I) -> SkillLoadOutcome
where
    I: IntoIterator<Item = SkillRoot>,
{
    let mut outcome = SkillLoadOutcome::default();
    let mut skill_roots: Vec<AbsolutePathBuf> = Vec::new();
    let mut skill_root_by_path: HashMap<AbsolutePathBuf, AbsolutePathBuf> = HashMap::new();
    let mut file_systems_by_skill_path: HashMap<AbsolutePathBuf, Arc<dyn ExecutorFileSystem>> =
        HashMap::new();
    for root in roots {
        let root_path = canonicalize_for_skill_identity(&root.path);
        let fs = root.file_system;
        let skills_before_root = outcome.skills.len();
        discover_skills_under_root(
            fs.as_ref(),
            &root_path,
            root.scope,
            root.plugin_id.as_deref(),
            &mut outcome,
        )
        .await;
        for skill in &outcome.skills[skills_before_root..] {
            if !skill_roots.contains(&root_path) {
                skill_roots.push(root_path.clone());
            }
            skill_root_by_path
                .entry(skill.path_to_skills_md.clone())
                .or_insert_with(|| root_path.clone());
            file_systems_by_skill_path
                .entry(skill.path_to_skills_md.clone())
                .or_insert_with(|| Arc::clone(&fs));
        }
    }

    let mut seen: HashSet<AbsolutePathBuf> = HashSet::new();
    outcome
        .skills
        .retain(|skill| seen.insert(skill.path_to_skills_md.clone()));
    let retained_skill_paths: HashSet<AbsolutePathBuf> = outcome
        .skills
        .iter()
        .map(|skill| skill.path_to_skills_md.clone())
        .collect();
    skill_root_by_path.retain(|path, _| retained_skill_paths.contains(path));
    let used_roots: HashSet<AbsolutePathBuf> = skill_root_by_path.values().cloned().collect();
    skill_roots.retain(|root| used_roots.contains(root));
    file_systems_by_skill_path.retain(|path, _| retained_skill_paths.contains(path));
    outcome.set_load_context(skill_roots, skill_root_by_path, file_systems_by_skill_path);

    fn scope_rank(scope: SkillScope) -> u8 {
        // Higher-priority scopes first (matches root scan order for dedupe).
        match scope {
            SkillScope::Repo => 0,
            SkillScope::User => 1,
            SkillScope::System => 2,
            SkillScope::Admin => 3,
        }
    }

    outcome.skills.sort_by(|a, b| {
        scope_rank(a.scope)
            .cmp(&scope_rank(b.scope))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path_to_skills_md.cmp(&b.path_to_skills_md))
    });

    outcome
}

pub(crate) async fn skill_roots(
    fs: Option<Arc<dyn ExecutorFileSystem>>,
    config_layer_stack: &SkillConfigLayerStack,
    cwd: &AbsolutePathBuf,
    plugin_skill_roots: Vec<PluginSkillRoot>,
) -> Vec<SkillRoot> {
    let home_dir =
        home_dir().and_then(|path| AbsolutePathBuf::from_absolute_path_checked(path).ok());
    skill_roots_with_home_dir(
        fs,
        config_layer_stack,
        cwd,
        home_dir.as_ref(),
        plugin_skill_roots,
    )
    .await
}

async fn skill_roots_with_home_dir(
    fs: Option<Arc<dyn ExecutorFileSystem>>,
    config_layer_stack: &SkillConfigLayerStack,
    cwd: &AbsolutePathBuf,
    home_dir: Option<&AbsolutePathBuf>,
    plugin_skill_roots: Vec<PluginSkillRoot>,
) -> Vec<SkillRoot> {
    let repo_fs = fs.unwrap_or_else(|| Arc::clone(&LOCAL_FS));
    let mut roots =
        skill_roots_from_layer_stack_inner(config_layer_stack, home_dir, Arc::clone(&repo_fs));
    roots.extend(plugin_skill_roots.into_iter().map(|root| SkillRoot {
        path: root.path,
        scope: SkillScope::User,
        file_system: Arc::clone(&LOCAL_FS),
        plugin_id: Some(root.plugin_id),
    }));
    roots.extend(repo_dot_codex_skill_roots(Arc::clone(&repo_fs), config_layer_stack, cwd).await);
    roots.extend(repo_agents_skill_roots(repo_fs, config_layer_stack, cwd).await);
    dedupe_skill_roots_by_path(&mut roots);
    roots
}

fn skill_roots_from_layer_stack_inner(
    config_layer_stack: &SkillConfigLayerStack,
    home_dir: Option<&AbsolutePathBuf>,
    repo_fs: Arc<dyn ExecutorFileSystem>,
) -> Vec<SkillRoot> {
    let mut roots = Vec::new();

    for layer in config_layer_stack.get_layers(
        SkillConfigLayerStackOrdering::HighestPrecedenceFirst,
        /*include_disabled*/ true,
    ) {
        let Some(config_folder) = layer.config_folder() else {
            continue;
        };

        match &layer.name {
            ConfigLayerSource::Project { .. } => {
                roots.push(SkillRoot {
                    path: config_folder.join(SKILLS_DIR_NAME),
                    scope: SkillScope::Repo,
                    file_system: Arc::clone(&repo_fs),
                    plugin_id: None,
                });
            }
            ConfigLayerSource::User { .. } => {
                // Deprecated user skills location (`$MORPHEUS_HOME/skills`), kept for backward
                // compatibility.
                roots.push(SkillRoot {
                    path: config_folder.join(SKILLS_DIR_NAME),
                    scope: SkillScope::User,
                    file_system: Arc::clone(&LOCAL_FS),
                    plugin_id: None,
                });

                // `$HOME/.agents/skills` (user-installed skills).
                if let Some(home_dir) = home_dir {
                    roots.push(SkillRoot {
                        path: home_dir.join(AGENTS_DIR_NAME).join(SKILLS_DIR_NAME),
                        scope: SkillScope::User,
                        file_system: Arc::clone(&LOCAL_FS),
                        plugin_id: None,
                    });
                }

                // Embedded system skills are cached under `$MORPHEUS_HOME/skills/.system` and are a
                // special case (not a config layer).
                roots.push(SkillRoot {
                    path: system_cache_root_dir(&config_folder),
                    scope: SkillScope::System,
                    file_system: Arc::clone(&LOCAL_FS),
                    plugin_id: None,
                });
            }
            ConfigLayerSource::System { .. } => {
                // The system config layer lives under `/etc/codex/` on Unix, so treat
                // `/etc/codex/skills` as admin-scoped skills.
                roots.push(SkillRoot {
                    path: config_folder.join(SKILLS_DIR_NAME),
                    scope: SkillScope::Admin,
                    file_system: Arc::clone(&LOCAL_FS),
                    plugin_id: None,
                });
            }
            ConfigLayerSource::Mdm { .. }
            | ConfigLayerSource::SessionFlags
            | ConfigLayerSource::LegacyManagedConfigTomlFromFile { .. }
            | ConfigLayerSource::LegacyManagedConfigTomlFromMdm => {}
        }
    }

    roots
}

async fn repo_agents_skill_roots(
    fs: Arc<dyn ExecutorFileSystem>,
    config_layer_stack: &SkillConfigLayerStack,
    cwd: &AbsolutePathBuf,
) -> Vec<SkillRoot> {
    repo_skills_roots_for_dirname(fs, config_layer_stack, cwd, AGENTS_DIR_NAME).await
}

async fn repo_dot_codex_skill_roots(
    fs: Arc<dyn ExecutorFileSystem>,
    config_layer_stack: &SkillConfigLayerStack,
    cwd: &AbsolutePathBuf,
) -> Vec<SkillRoot> {
    repo_skills_roots_for_dirname(fs, config_layer_stack, cwd, ".codex").await
}

async fn repo_skills_roots_for_dirname(
    fs: Arc<dyn ExecutorFileSystem>,
    config_layer_stack: &SkillConfigLayerStack,
    cwd: &AbsolutePathBuf,
    dirname: &str,
) -> Vec<SkillRoot> {
    let project_root_markers = project_root_markers_from_stack(config_layer_stack);
    let project_root = find_project_root(fs.as_ref(), cwd, &project_root_markers).await;
    let dirs = dirs_between_project_root_and_cwd(cwd, &project_root);
    let mut roots = Vec::new();
    for dir in dirs {
        let repo_skills = dir.join(dirname).join(SKILLS_DIR_NAME);
        match fs.get_metadata(&repo_skills, /*sandbox*/ None).await {
            Ok(metadata) if metadata.is_directory => roots.push(SkillRoot {
                path: repo_skills,
                scope: SkillScope::Repo,
                file_system: Arc::clone(&fs),
                plugin_id: None,
            }),
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                tracing::warn!(
                    "failed to stat repo skills root {}: {err:#}",
                    repo_skills.display()
                );
            }
        }
    }
    roots
}

fn project_root_markers_from_stack(config_layer_stack: &SkillConfigLayerStack) -> Vec<String> {
    let merged = config_layer_stack.effective_config_without_project_layers();
    match project_root_markers_from_config(&merged) {
        Ok(Some(markers)) => markers,
        Ok(None) => default_project_root_markers(),
        Err(err) => {
            tracing::warn!("invalid project_root_markers: {err}");
            default_project_root_markers()
        }
    }
}

fn project_root_markers_from_config(config: &TomlValue) -> io::Result<Option<Vec<String>>> {
    let Some(table) = config.as_table() else {
        return Ok(None);
    };
    let Some(markers_value) = table.get("project_root_markers") else {
        return Ok(None);
    };
    let TomlValue::Array(entries) = markers_value else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "project_root_markers must be an array of strings",
        ));
    };
    let mut markers = Vec::new();
    for entry in entries {
        let Some(marker) = entry.as_str() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "project_root_markers must be an array of strings",
            ));
        };
        markers.push(marker.to_string());
    }
    Ok(Some(markers))
}

fn default_project_root_markers() -> Vec<String> {
    vec![".git".to_string()]
}

async fn find_project_root(
    fs: &dyn ExecutorFileSystem,
    cwd: &AbsolutePathBuf,
    project_root_markers: &[String],
) -> AbsolutePathBuf {
    if project_root_markers.is_empty() {
        return cwd.clone();
    }

    for ancestor in cwd.ancestors() {
        for marker in project_root_markers {
            let marker_path = ancestor.join(marker);
            match fs.get_metadata(&marker_path, /*sandbox*/ None).await {
                Ok(_) => return ancestor,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => {
                    tracing::warn!(
                        "failed to stat project root marker {}: {err:#}",
                        marker_path.display()
                    );
                }
            }
        }
    }

    cwd.clone()
}

fn dirs_between_project_root_and_cwd(
    cwd: &AbsolutePathBuf,
    project_root: &AbsolutePathBuf,
) -> Vec<AbsolutePathBuf> {
    let mut dirs = cwd
        .ancestors()
        .scan(false, |done, dir| {
            if *done {
                None
            } else {
                if &dir == project_root {
                    *done = true;
                }
                Some(dir)
            }
        })
        .collect::<Vec<_>>();
    dirs.reverse();
    dirs
}

fn dedupe_skill_roots_by_path(roots: &mut Vec<SkillRoot>) {
    let mut seen: HashSet<AbsolutePathBuf> = HashSet::new();
    roots.retain(|root| seen.insert(root.path.clone()));
}

fn canonicalize_for_skill_identity(path: &AbsolutePathBuf) -> AbsolutePathBuf {
    path.canonicalize().unwrap_or_else(|_| path.clone())
}

async fn discover_skills_under_root(
    fs: &dyn ExecutorFileSystem,
    root: &AbsolutePathBuf,
    scope: SkillScope,
    plugin_id: Option<&str>,
    outcome: &mut SkillLoadOutcome,
) {
    let root = canonicalize_for_skill_identity(root);

    match fs.get_metadata(&root, /*sandbox*/ None).await {
        Ok(metadata) if metadata.is_directory => {}
        Ok(_) => return,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return,
        Err(err) => {
            error!("failed to stat skills root {}: {err:#}", root.display());
            return;
        }
    }

    fn enqueue_dir(
        queue: &mut VecDeque<(AbsolutePathBuf, usize)>,
        visited_dirs: &mut HashSet<AbsolutePathBuf>,
        truncated_by_dir_limit: &mut bool,
        path: AbsolutePathBuf,
        depth: usize,
    ) {
        if depth > MAX_SCAN_DEPTH {
            return;
        }
        if visited_dirs.len() >= MAX_SKILLS_DIRS_PER_ROOT {
            *truncated_by_dir_limit = true;
            return;
        }
        if visited_dirs.insert(path.clone()) {
            queue.push_back((path, depth));
        }
    }

    // Follow symlinked directories for user, admin, and repo skills. System skills are written by Codex itself.
    let follow_symlinks = matches!(
        scope,
        SkillScope::Repo | SkillScope::User | SkillScope::Admin
    );

    let mut visited_dirs: HashSet<AbsolutePathBuf> = HashSet::new();
    visited_dirs.insert(root.clone());

    let mut queue: VecDeque<(AbsolutePathBuf, usize)> = VecDeque::from([(root.clone(), 0)]);
    let mut truncated_by_dir_limit = false;

    while let Some((dir, depth)) = queue.pop_front() {
        let entries = match fs.read_directory(&dir, /*sandbox*/ None).await {
            Ok(entries) => entries,
            Err(e) => {
                error!("failed to read skills dir {}: {e:#}", dir.display());
                continue;
            }
        };

        for entry in entries {
            let file_name = entry.file_name;
            if file_name.starts_with('.') {
                continue;
            }

            let path = dir.join(&file_name);
            let metadata = match fs.get_metadata(&path, /*sandbox*/ None).await {
                Ok(metadata) => metadata,
                Err(e) => {
                    error!("failed to stat skills path {}: {e:#}", path.display());
                    continue;
                }
            };

            if metadata.is_symlink {
                if !follow_symlinks {
                    continue;
                }
                match fs.read_directory(&path, /*sandbox*/ None).await {
                    Ok(_) => {
                        let resolved_dir = canonicalize_for_skill_identity(&path);
                        enqueue_dir(
                            &mut queue,
                            &mut visited_dirs,
                            &mut truncated_by_dir_limit,
                            resolved_dir,
                            depth + 1,
                        );
                    }
                    Err(err)
                        if matches!(
                            err.kind(),
                            io::ErrorKind::NotADirectory | io::ErrorKind::NotFound
                        ) => {}
                    Err(err) => {
                        error!(
                            "failed to read skills symlink dir {}: {err:#}",
                            path.display()
                        );
                    }
                }
                continue;
            }

            if metadata.is_directory {
                let resolved_dir = canonicalize_for_skill_identity(&path);
                enqueue_dir(
                    &mut queue,
                    &mut visited_dirs,
                    &mut truncated_by_dir_limit,
                    resolved_dir,
                    depth + 1,
                );
                continue;
            }

            if metadata.is_file && file_name == SKILLS_FILENAME {
                match parse_skill_file(fs, &path, scope, plugin_id).await {
                    Ok(skill) => {
                        outcome.skills.push(skill);
                    }
                    Err(err) => {
                        if scope != SkillScope::System {
                            outcome.errors.push(SkillError {
                                path: path.clone(),
                                message: err.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    if truncated_by_dir_limit {
        tracing::warn!(
            "skills scan truncated after {} directories (root: {})",
            MAX_SKILLS_DIRS_PER_ROOT,
            root.display()
        );
    }
}

async fn parse_skill_file(
    fs: &dyn ExecutorFileSystem,
    path: &AbsolutePathBuf,
    scope: SkillScope,
    plugin_id: Option<&str>,
) -> Result<SkillMetadata, SkillParseError> {
    let contents = fs
        .read_file_text(path, /*sandbox*/ None)
        .await
        .map_err(SkillParseError::Read)?;

    let frontmatter = extract_frontmatter(&contents).ok_or(SkillParseError::MissingFrontmatter)?;

    let parsed = parse_skill_frontmatter(&frontmatter).map_err(SkillParseError::InvalidYaml)?;

    let base_name = parsed
        .name
        .as_deref()
        .map(sanitize_single_line)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_skill_name(path));
    let name = namespaced_skill_name(fs, path, &base_name).await;
    let description = parsed
        .description
        .as_deref()
        .map(sanitize_single_line)
        .unwrap_or_default();
    let short_description = parsed
        .metadata
        .short_description
        .as_deref()
        .map(sanitize_single_line)
        .filter(|value| !value.is_empty());
    let LoadedSkillMetadata {
        interface,
        dependencies,
        policy,
    } = load_skill_metadata(fs, path).await;

    validate_len(&name, MAX_NAME_LEN, "name")?;
    validate_len(&description, MAX_DESCRIPTION_LEN, "description")?;
    if let Some(short_description) = short_description.as_deref() {
        validate_len(
            short_description,
            MAX_SHORT_DESCRIPTION_LEN,
            "metadata.short-description",
        )?;
    }

    let resolved_path = canonicalize_for_skill_identity(path);

    Ok(SkillMetadata {
        name,
        description,
        short_description,
        interface,
        dependencies,
        policy,
        path_to_skills_md: resolved_path,
        scope,
        plugin_id: plugin_id.map(str::to_string),
    })
}

fn default_skill_name(path: &AbsolutePathBuf) -> String {
    path.parent()
        .and_then(|parent| {
            parent
                .file_name()
                .and_then(|name| name.to_str())
                .map(sanitize_single_line)
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "skill".to_string())
}

async fn namespaced_skill_name(
    fs: &dyn ExecutorFileSystem,
    path: &AbsolutePathBuf,
    base_name: &str,
) -> String {
    plugin_namespace_for_skill_path(fs, path)
        .await
        .map(|namespace| format!("{namespace}:{base_name}"))
        .unwrap_or_else(|| base_name.to_string())
}

async fn load_skill_metadata(
    fs: &dyn ExecutorFileSystem,
    skill_path: &AbsolutePathBuf,
) -> LoadedSkillMetadata {
    // Fail open: optional metadata should not block loading SKILL.md.
    let Some(skill_dir) = skill_path.parent() else {
        return LoadedSkillMetadata::default();
    };
    let metadata_path = skill_dir
        .join(SKILLS_METADATA_DIR)
        .join(SKILLS_METADATA_FILENAME);
    match fs.get_metadata(&metadata_path, /*sandbox*/ None).await {
        Ok(metadata) if metadata.is_file => {}
        Ok(_) => return LoadedSkillMetadata::default(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return LoadedSkillMetadata::default();
        }
        Err(error) => {
            tracing::warn!(
                "ignoring {path}: failed to stat {label}: {error}",
                path = metadata_path.display(),
                label = SKILLS_METADATA_FILENAME
            );
            return LoadedSkillMetadata::default();
        }
    }

    let contents = match fs.read_file_text(&metadata_path, /*sandbox*/ None).await {
        Ok(contents) => contents,
        Err(error) => {
            tracing::warn!(
                "ignoring {path}: failed to read {label}: {error}",
                path = metadata_path.display(),
                label = SKILLS_METADATA_FILENAME
            );
            return LoadedSkillMetadata::default();
        }
    };

    let parsed: SkillMetadataFile = {
        let _guard = AbsolutePathBufGuard::new(skill_dir.as_path());
        match parse_skill_metadata_file(&contents) {
            Ok(parsed) => parsed,
            Err(error) => {
                tracing::warn!(
                    "ignoring {path}: invalid {label}: {error}",
                    path = metadata_path.display(),
                    label = SKILLS_METADATA_FILENAME
                );
                return LoadedSkillMetadata::default();
            }
        }
    };

    let SkillMetadataFile {
        interface,
        dependencies,
        policy,
    } = parsed;
    LoadedSkillMetadata {
        interface: resolve_interface(interface, &skill_dir),
        dependencies: resolve_dependencies(dependencies),
        policy: resolve_policy(policy),
    }
}

fn resolve_interface(
    interface: Option<Interface>,
    skill_dir: &AbsolutePathBuf,
) -> Option<SkillInterface> {
    let interface = interface?;
    let interface = SkillInterface {
        display_name: resolve_str(
            interface.display_name,
            MAX_NAME_LEN,
            "interface.display_name",
        ),
        short_description: resolve_str(
            interface.short_description,
            MAX_SHORT_DESCRIPTION_LEN,
            "interface.short_description",
        ),
        icon_small: resolve_asset_path(skill_dir, "interface.icon_small", interface.icon_small),
        icon_large: resolve_asset_path(skill_dir, "interface.icon_large", interface.icon_large),
        brand_color: resolve_color_str(interface.brand_color, "interface.brand_color"),
        default_prompt: resolve_str(
            interface.default_prompt,
            MAX_DEFAULT_PROMPT_LEN,
            "interface.default_prompt",
        ),
    };
    let has_fields = interface.display_name.is_some()
        || interface.short_description.is_some()
        || interface.icon_small.is_some()
        || interface.icon_large.is_some()
        || interface.brand_color.is_some()
        || interface.default_prompt.is_some();
    if has_fields { Some(interface) } else { None }
}

fn resolve_dependencies(dependencies: Option<Dependencies>) -> Option<SkillDependencies> {
    let dependencies = dependencies?;
    let tools: Vec<SkillToolDependency> = dependencies
        .tools
        .into_iter()
        .filter_map(resolve_dependency_tool)
        .collect();
    if tools.is_empty() {
        None
    } else {
        Some(SkillDependencies { tools })
    }
}

fn resolve_policy(policy: Option<Policy>) -> Option<SkillPolicy> {
    policy.map(|policy| SkillPolicy {
        allow_implicit_invocation: policy.allow_implicit_invocation,
        products: policy.products,
    })
}

fn resolve_dependency_tool(tool: DependencyTool) -> Option<SkillToolDependency> {
    let r#type = resolve_required_str(
        tool.kind,
        MAX_DEPENDENCY_TYPE_LEN,
        "dependencies.tools.type",
    )?;
    let value = resolve_required_str(
        tool.value,
        MAX_DEPENDENCY_VALUE_LEN,
        "dependencies.tools.value",
    )?;
    let description = resolve_str(
        tool.description,
        MAX_DEPENDENCY_DESCRIPTION_LEN,
        "dependencies.tools.description",
    );
    let transport = resolve_str(
        tool.transport,
        MAX_DEPENDENCY_TRANSPORT_LEN,
        "dependencies.tools.transport",
    );
    let command = resolve_str(
        tool.command,
        MAX_DEPENDENCY_COMMAND_LEN,
        "dependencies.tools.command",
    );
    let url = resolve_str(tool.url, MAX_DEPENDENCY_URL_LEN, "dependencies.tools.url");

    Some(SkillToolDependency {
        r#type,
        value,
        description,
        transport,
        command,
        url,
    })
}

fn resolve_asset_path(
    skill_dir: &AbsolutePathBuf,
    field: &'static str,
    path: Option<PathBuf>,
) -> Option<AbsolutePathBuf> {
    // Icons must be relative paths under the skill's assets/ directory; otherwise return None.
    let path = path?;
    if path.as_os_str().is_empty() {
        return None;
    }

    let assets_dir = skill_dir.join("assets");
    if path.is_absolute() {
        tracing::warn!(
            "ignoring {field}: icon must be a relative assets path (not {})",
            assets_dir.display()
        );
        return None;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(component) => normalized.push(component),
            Component::ParentDir => {
                tracing::warn!("ignoring {field}: icon path must not contain '..'");
                return None;
            }
            _ => {
                tracing::warn!("ignoring {field}: icon path must be under assets/");
                return None;
            }
        }
    }

    let mut components = normalized.components();
    match components.next() {
        Some(Component::Normal(component)) if component == "assets" => {}
        _ => {
            tracing::warn!("ignoring {field}: icon path must be under assets/");
            return None;
        }
    }

    Some(skill_dir.join(normalized))
}

fn sanitize_single_line(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_len(
    value: &str,
    max_len: usize,
    field_name: &'static str,
) -> Result<(), SkillParseError> {
    if value.is_empty() {
        return Err(SkillParseError::MissingField(field_name));
    }
    if value.chars().count() > max_len {
        return Err(SkillParseError::InvalidField {
            field: field_name,
            reason: format!("exceeds maximum length of {max_len} characters"),
        });
    }
    Ok(())
}

fn resolve_str(value: Option<String>, max_len: usize, field: &'static str) -> Option<String> {
    let value = value?;
    let value = sanitize_single_line(&value);
    if value.is_empty() {
        tracing::warn!("ignoring {field}: value is empty");
        return None;
    }
    if value.chars().count() > max_len {
        tracing::warn!("ignoring {field}: exceeds maximum length of {max_len} characters");
        return None;
    }
    Some(value)
}

fn resolve_required_str(
    value: Option<String>,
    max_len: usize,
    field: &'static str,
) -> Option<String> {
    let Some(value) = value else {
        tracing::warn!("ignoring {field}: value is missing");
        return None;
    };
    resolve_str(Some(value), max_len, field)
}

fn resolve_color_str(value: Option<String>, field: &'static str) -> Option<String> {
    let value = value?;
    let value = value.trim();
    if value.is_empty() {
        tracing::warn!("ignoring {field}: value is empty");
        return None;
    }
    let mut chars = value.chars();
    if value.len() == 7 && chars.next() == Some('#') && chars.all(|c| c.is_ascii_hexdigit()) {
        Some(value.to_string())
    } else {
        tracing::warn!("ignoring {field}: expected #RRGGBB, got {value}");
        None
    }
}

fn parse_skill_frontmatter(frontmatter: &str) -> Result<SkillFrontmatter, String> {
    let lines: Vec<&str> = frontmatter.lines().collect();
    let mut parsed = SkillFrontmatter {
        name: None,
        description: None,
        metadata: SkillFrontmatterMetadata::default(),
    };
    let mut index = 0;
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent != 0 {
            return Err(format!("line {}: unexpected indented line", line.number));
        }
        let (key, value) = split_skill_yaml_key_value(line.content, line.number)?;
        match key {
            "name" => {
                parsed.name = Some(parse_skill_yaml_value(value, &lines, &mut index, 0)?);
                index += 1;
            }
            "description" => {
                parsed.description = Some(parse_skill_yaml_value(value, &lines, &mut index, 0)?);
                index += 1;
            }
            "metadata" => {
                if value.trim() == "{}" {
                    parsed.metadata = SkillFrontmatterMetadata::default();
                    index += 1;
                } else if value.trim().is_empty() {
                    let (metadata, next_index) =
                        parse_skill_frontmatter_metadata(&lines, index + 1)?;
                    parsed.metadata = metadata;
                    index = next_index;
                } else {
                    return Err(format!(
                        "line {}: `metadata` must be a block map",
                        line.number
                    ));
                }
            }
            _ => {
                index = skip_skill_yaml_nested_block(&lines, index + 1, 0)?;
            }
        }
    }
    Ok(parsed)
}

fn parse_skill_frontmatter_metadata(
    lines: &[&str],
    mut index: usize,
) -> Result<(SkillFrontmatterMetadata, usize), String> {
    let mut metadata = SkillFrontmatterMetadata::default();
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent == 0 {
            break;
        }
        if line.indent != 2 {
            return Err(format!(
                "line {}: `metadata` fields must be indented by two spaces",
                line.number
            ));
        }
        let (key, value) = split_skill_yaml_key_value(line.content, line.number)?;
        match key {
            "short-description" => {
                metadata.short_description =
                    Some(parse_skill_yaml_value(value, lines, &mut index, 2)?);
                index += 1;
            }
            _ => {
                index = skip_skill_yaml_nested_block(lines, index + 1, 2)?;
            }
        }
    }
    Ok((metadata, index))
}

fn parse_skill_metadata_file(contents: &str) -> Result<SkillMetadataFile, String> {
    if contents.trim_start().starts_with('{') {
        return serde_json::from_str(contents).map_err(|err| err.to_string());
    }

    let lines: Vec<&str> = contents.lines().collect();
    let mut parsed = SkillMetadataFile::default();
    let mut index = 0;
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent != 0 {
            return Err(format!("line {}: unexpected indented line", line.number));
        }
        let (key, value) = split_skill_yaml_key_value(line.content, line.number)?;
        match key {
            "interface" => {
                if value.trim() == "{}" {
                    parsed.interface = Some(Interface::default());
                    index += 1;
                } else if value.trim().is_empty() {
                    let (interface, next_index) = parse_skill_interface(&lines, index + 1)?;
                    parsed.interface = Some(interface);
                    index = next_index;
                } else {
                    return Err(format!(
                        "line {}: `interface` must be a block map",
                        line.number
                    ));
                }
            }
            "dependencies" => {
                if value.trim() == "{}" {
                    parsed.dependencies = Some(Dependencies::default());
                    index += 1;
                } else if value.trim().is_empty() {
                    let (dependencies, next_index) = parse_skill_dependencies(&lines, index + 1)?;
                    parsed.dependencies = Some(dependencies);
                    index = next_index;
                } else {
                    return Err(format!(
                        "line {}: `dependencies` must be a block map",
                        line.number
                    ));
                }
            }
            "policy" => {
                if value.trim() == "{}" {
                    parsed.policy = Some(Policy {
                        allow_implicit_invocation: None,
                        products: Vec::new(),
                    });
                    index += 1;
                } else if value.trim().is_empty() {
                    let (policy, next_index) = parse_skill_policy(&lines, index + 1)?;
                    parsed.policy = Some(policy);
                    index = next_index;
                } else {
                    return Err(format!(
                        "line {}: `policy` must be a block map",
                        line.number
                    ));
                }
            }
            _ => {
                index = skip_skill_yaml_nested_block(&lines, index + 1, 0)?;
            }
        }
    }
    Ok(parsed)
}

fn parse_skill_interface(lines: &[&str], mut index: usize) -> Result<(Interface, usize), String> {
    let mut interface = Interface::default();
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent == 0 {
            break;
        }
        if line.indent != 2 {
            return Err(format!(
                "line {}: `interface` fields must be indented by two spaces",
                line.number
            ));
        }
        let (key, value) = split_skill_yaml_key_value(line.content, line.number)?;
        match key {
            "display_name" => {
                interface.display_name = Some(parse_skill_yaml_value(value, lines, &mut index, 2)?);
                index += 1;
            }
            "short_description" => {
                interface.short_description =
                    Some(parse_skill_yaml_value(value, lines, &mut index, 2)?);
                index += 1;
            }
            "icon_small" => {
                interface.icon_small = Some(PathBuf::from(parse_skill_yaml_value(
                    value, lines, &mut index, 2,
                )?));
                index += 1;
            }
            "icon_large" => {
                interface.icon_large = Some(PathBuf::from(parse_skill_yaml_value(
                    value, lines, &mut index, 2,
                )?));
                index += 1;
            }
            "brand_color" => {
                interface.brand_color = Some(parse_skill_yaml_value(value, lines, &mut index, 2)?);
                index += 1;
            }
            "default_prompt" => {
                interface.default_prompt =
                    Some(parse_skill_yaml_value(value, lines, &mut index, 2)?);
                index += 1;
            }
            _ => {
                index = skip_skill_yaml_nested_block(lines, index + 1, 2)?;
            }
        }
    }
    Ok((interface, index))
}

fn parse_skill_dependencies(
    lines: &[&str],
    mut index: usize,
) -> Result<(Dependencies, usize), String> {
    let mut dependencies = Dependencies::default();
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent == 0 {
            break;
        }
        if line.indent != 2 {
            return Err(format!(
                "line {}: `dependencies` fields must be indented by two spaces",
                line.number
            ));
        }
        let (key, value) = split_skill_yaml_key_value(line.content, line.number)?;
        match key {
            "tools" => {
                if value.trim() == "[]" {
                    dependencies.tools = Vec::new();
                    index += 1;
                } else if value.trim().is_empty() {
                    let (tools, next_index) = parse_skill_dependency_tools(lines, index + 1, 2)?;
                    dependencies.tools = tools;
                    index = next_index;
                } else {
                    return Err(format!(
                        "line {}: `tools` must be a block list",
                        line.number
                    ));
                }
            }
            _ => {
                index = skip_skill_yaml_nested_block(lines, index + 1, 2)?;
            }
        }
    }
    Ok((dependencies, index))
}

fn parse_skill_dependency_tools(
    lines: &[&str],
    mut index: usize,
    parent_indent: usize,
) -> Result<(Vec<DependencyTool>, usize), String> {
    let mut tools = Vec::new();
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent <= parent_indent {
            break;
        }
        let item_indent = parent_indent + 2;
        if line.indent != item_indent {
            return Err(format!(
                "line {}: `tools` list items must be indented by two spaces",
                line.number
            ));
        }
        let Some(item) = line.content.strip_prefix('-') else {
            return Err(format!(
                "line {}: `tools` entries must use `- value` list items",
                line.number
            ));
        };
        let mut tool = DependencyTool::default();
        let item = item.trim_start();
        if !item.is_empty() {
            parse_skill_dependency_tool_field(&mut tool, item, lines, &mut index, item_indent)?;
        }
        index += 1;
        while index < lines.len() {
            let Some(field_line) = skill_yaml_line(lines[index], index + 1)? else {
                index += 1;
                continue;
            };
            if field_line.indent <= item_indent {
                break;
            }
            if field_line.indent != item_indent + 2 {
                return Err(format!(
                    "line {}: dependency tool fields must be indented by four spaces",
                    field_line.number
                ));
            }
            parse_skill_dependency_tool_field(
                &mut tool,
                field_line.content,
                lines,
                &mut index,
                item_indent + 2,
            )?;
            index += 1;
        }
        tools.push(tool);
    }
    Ok((tools, index))
}

fn parse_skill_dependency_tool_field(
    tool: &mut DependencyTool,
    content: &str,
    lines: &[&str],
    index: &mut usize,
    indent: usize,
) -> Result<(), String> {
    let line_number = *index + 1;
    let (key, value) = split_skill_yaml_key_value(content, line_number)?;
    let value = parse_skill_yaml_value(value, lines, index, indent)?;
    match key {
        "type" => tool.kind = Some(value),
        "value" => tool.value = Some(value),
        "description" => tool.description = Some(value),
        "transport" => tool.transport = Some(value),
        "command" => tool.command = Some(value),
        "url" => tool.url = Some(value),
        _ => {}
    }
    Ok(())
}

fn parse_skill_policy(lines: &[&str], mut index: usize) -> Result<(Policy, usize), String> {
    let mut policy = Policy {
        allow_implicit_invocation: None,
        products: Vec::new(),
    };
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent == 0 {
            break;
        }
        if line.indent != 2 {
            return Err(format!(
                "line {}: `policy` fields must be indented by two spaces",
                line.number
            ));
        }
        let (key, value) = split_skill_yaml_key_value(line.content, line.number)?;
        match key {
            "allow_implicit_invocation" => {
                policy.allow_implicit_invocation = Some(parse_skill_yaml_bool(value, line.number)?);
                index += 1;
            }
            "products" => {
                let value = value.trim();
                if value.starts_with('[') {
                    policy.products = parse_skill_yaml_flow_list(value, line.number)?
                        .into_iter()
                        .map(|product| parse_skill_yaml_product(&product, line.number))
                        .collect::<Result<Vec<_>, _>>()?;
                    index += 1;
                } else if value.is_empty() {
                    let (products, next_index) = parse_skill_yaml_product_list(lines, index + 1)?;
                    policy.products = products;
                    index = next_index;
                } else {
                    return Err(format!(
                        "line {}: `products` must be a block or inline list",
                        line.number
                    ));
                }
            }
            _ => {
                index = skip_skill_yaml_nested_block(lines, index + 1, 2)?;
            }
        }
    }
    Ok((policy, index))
}

fn parse_skill_yaml_product_list(
    lines: &[&str],
    mut index: usize,
) -> Result<(Vec<Product>, usize), String> {
    let mut products = Vec::new();
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent <= 2 {
            break;
        }
        if line.indent != 4 {
            return Err(format!(
                "line {}: `products` list items must be indented by four spaces",
                line.number
            ));
        }
        let Some(item) = line.content.strip_prefix("- ") else {
            return Err(format!(
                "line {}: `products` entries must use `- value` list items",
                line.number
            ));
        };
        let product = parse_skill_yaml_scalar(item, line.number)?;
        products.push(parse_skill_yaml_product(&product, line.number)?);
        index += 1;
    }
    Ok((products, index))
}

#[derive(Clone, Copy)]
struct SkillYamlLine<'a> {
    indent: usize,
    content: &'a str,
    number: usize,
}

fn skill_yaml_line<'a>(
    raw: &'a str,
    line_number: usize,
) -> Result<Option<SkillYamlLine<'a>>, String> {
    let raw = raw.strip_suffix('\r').unwrap_or(raw);
    let trimmed_start = raw.trim_start();
    if trimmed_start.is_empty() || trimmed_start.starts_with('#') {
        return Ok(None);
    }
    let mut indent = 0;
    for byte in raw.bytes() {
        match byte {
            b' ' => indent += 1,
            b'\t' => {
                return Err(format!(
                    "line {line_number}: tabs are not supported in skill metadata"
                ));
            }
            _ => break,
        }
    }
    Ok(Some(SkillYamlLine {
        indent,
        content: &raw[indent..],
        number: line_number,
    }))
}

fn split_skill_yaml_key_value(line: &str, line_number: usize) -> Result<(&str, &str), String> {
    let Some((key, value)) = line.split_once(':') else {
        return Err(format!("line {line_number}: expected `key: value`"));
    };
    let key = key.trim();
    if key.is_empty() {
        return Err(format!("line {line_number}: key must not be empty"));
    }
    Ok((key, value.trim_start()))
}

fn parse_skill_yaml_value(
    value: &str,
    lines: &[&str],
    index: &mut usize,
    parent_indent: usize,
) -> Result<String, String> {
    let value = value.trim();
    if matches!(value, "|" | "|-" | ">" | ">-") {
        let folded = value.starts_with('>');
        let (value, next_index) =
            parse_skill_yaml_block_scalar(lines, *index + 1, parent_indent, folded)?;
        *index = next_index.saturating_sub(1);
        return Ok(value);
    }
    parse_skill_yaml_scalar(value, *index + 1)
}

fn parse_skill_yaml_block_scalar(
    lines: &[&str],
    mut index: usize,
    parent_indent: usize,
    folded: bool,
) -> Result<(String, usize), String> {
    let content_indent = parent_indent + 2;
    let mut values = Vec::new();
    while index < lines.len() {
        let raw = lines[index].strip_suffix('\r').unwrap_or(lines[index]);
        if raw.trim().is_empty() {
            values.push(String::new());
            index += 1;
            continue;
        }
        let Some(line) = skill_yaml_line(raw, index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent < content_indent {
            break;
        }
        values.push(raw[content_indent.min(raw.len())..].to_string());
        index += 1;
    }
    let separator = if folded { " " } else { "\n" };
    Ok((values.join(separator), index))
}

fn parse_skill_yaml_scalar(value: &str, line_number: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!(
            "line {line_number}: scalar value must not be empty"
        ));
    }
    if value.starts_with('"') {
        let end = skill_yaml_double_quote_end(value, line_number)?;
        validate_skill_yaml_trailing_comment(&value[end + 1..], line_number)?;
        return serde_json::from_str::<String>(&value[..=end])
            .map_err(|err| format!("line {line_number}: invalid double-quoted scalar: {err}"));
    }
    if value.starts_with('\'') {
        let (parsed, trailing) = parse_skill_yaml_single_quoted_scalar(value, line_number)?;
        validate_skill_yaml_trailing_comment(trailing, line_number)?;
        return Ok(parsed);
    }
    if value.starts_with('[') || value.starts_with('{') {
        return Err(format!(
            "line {line_number}: inline collections are not supported for this field"
        ));
    }
    let value = strip_skill_yaml_inline_comment(value).trim_end();
    if value.is_empty() {
        return Err(format!(
            "line {line_number}: scalar value must not be empty"
        ));
    }
    Ok(value.to_string())
}

fn parse_skill_yaml_bool(value: &str, line_number: usize) -> Result<bool, String> {
    match parse_skill_yaml_scalar(value, line_number)?
        .to_ascii_lowercase()
        .as_str()
    {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("line {line_number}: expected boolean value")),
    }
}

fn parse_skill_yaml_product(value: &str, line_number: usize) -> Result<Product, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "codex" => Ok(Product::Codex),
        "chatgpt" => Ok(Product::Chatgpt),
        "atlas" => Ok(Product::Atlas),
        _ => Err(format!("line {line_number}: unknown product `{value}`")),
    }
}

fn parse_skill_yaml_flow_list(value: &str, line_number: usize) -> Result<Vec<String>, String> {
    let (inside, trailing) = skill_yaml_bracketed_list(value, line_number)?;
    validate_skill_yaml_trailing_comment(trailing, line_number)?;
    let mut items = Vec::new();
    let mut rest = inside.trim();
    while !rest.is_empty() {
        let (item, next) = parse_skill_yaml_flow_list_item(rest, line_number)?;
        items.push(item);
        rest = next.trim_start();
        if rest.is_empty() {
            break;
        }
        let Some(after_comma) = rest.strip_prefix(',') else {
            return Err(format!(
                "line {line_number}: inline list items must be separated by commas"
            ));
        };
        rest = after_comma.trim_start();
        if rest.is_empty() {
            return Err(format!(
                "line {line_number}: inline list must not end with a trailing comma"
            ));
        }
    }
    Ok(items)
}

fn skill_yaml_bracketed_list(value: &str, line_number: usize) -> Result<(&str, &str), String> {
    let mut quote = None;
    let mut escaped = false;
    let mut iter = value.char_indices().skip(1).peekable();
    while let Some((index, ch)) = iter.next() {
        if let Some(active_quote) = quote {
            if active_quote == '"' && escaped {
                escaped = false;
                continue;
            }
            if active_quote == '"' && ch == '\\' {
                escaped = true;
                continue;
            }
            if active_quote == '\'' && ch == '\'' && iter.peek().is_some_and(|(_, c)| *c == '\'') {
                iter.next();
                continue;
            }
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            ']' => return Ok((&value[1..index], &value[index + 1..])),
            _ => {}
        }
    }
    Err(format!("line {line_number}: invalid inline list"))
}

fn parse_skill_yaml_flow_list_item(
    value: &str,
    line_number: usize,
) -> Result<(String, &str), String> {
    if value.starts_with('"') {
        let end = skill_yaml_double_quote_end(value, line_number)?;
        validate_skill_yaml_scalar_separator(&value[end + 1..], line_number)?;
        return serde_json::from_str::<String>(&value[..=end])
            .map(|parsed| (parsed, &value[end + 1..]))
            .map_err(|err| format!("line {line_number}: invalid double-quoted scalar: {err}"));
    }
    if value.starts_with('\'') {
        let (parsed, trailing) = parse_skill_yaml_single_quoted_scalar(value, line_number)?;
        validate_skill_yaml_scalar_separator(trailing, line_number)?;
        return Ok((parsed, trailing));
    }
    let end = value
        .char_indices()
        .find_map(|(index, ch)| (ch == ',').then_some(index))
        .unwrap_or(value.len());
    let raw = value[..end].trim();
    if raw.is_empty() {
        return Err(format!(
            "line {line_number}: inline list item must not be empty"
        ));
    }
    if raw.starts_with('[') || raw.starts_with('{') {
        return Err(format!(
            "line {line_number}: nested inline collections are not supported"
        ));
    }
    Ok((raw.to_string(), &value[end..]))
}

fn skill_yaml_double_quote_end(value: &str, line_number: usize) -> Result<usize, String> {
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

fn parse_skill_yaml_single_quoted_scalar(
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

fn validate_skill_yaml_scalar_separator(trailing: &str, line_number: usize) -> Result<(), String> {
    let trailing = trailing.trim_start();
    if trailing.is_empty() || trailing.starts_with(',') {
        return Ok(());
    }
    Err(format!(
        "line {line_number}: inline list items must be separated by commas"
    ))
}

fn validate_skill_yaml_trailing_comment(trailing: &str, line_number: usize) -> Result<(), String> {
    let trailing = trailing.trim();
    if trailing.is_empty() || trailing.starts_with('#') {
        return Ok(());
    }
    Err(format!(
        "line {line_number}: unexpected content after scalar value"
    ))
}

fn strip_skill_yaml_inline_comment(value: &str) -> &str {
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

fn skip_skill_yaml_nested_block(
    lines: &[&str],
    mut index: usize,
    parent_indent: usize,
) -> Result<usize, String> {
    while index < lines.len() {
        let Some(line) = skill_yaml_line(lines[index], index + 1)? else {
            index += 1;
            continue;
        };
        if line.indent <= parent_indent {
            break;
        }
        index += 1;
    }
    Ok(index)
}

fn extract_frontmatter(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if !matches!(lines.next(), Some(line) if line.trim() == "---") {
        return None;
    }

    let mut frontmatter_lines: Vec<&str> = Vec::new();
    let mut found_closing = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            found_closing = true;
            break;
        }
        frontmatter_lines.push(line);
    }

    if frontmatter_lines.is_empty() || !found_closing {
        return None;
    }

    Some(frontmatter_lines.join("\n"))
}
#[cfg(test)]
pub(crate) async fn skill_roots_from_layer_stack(
    fs: Arc<dyn ExecutorFileSystem>,
    config_layer_stack: &config_service::ConfigLayerStack,
    cwd: &AbsolutePathBuf,
    home_dir: Option<&AbsolutePathBuf>,
) -> Vec<SkillRoot> {
    let config_layer_stack = skill_config_layer_stack_from_config_layer_stack(config_layer_stack);
    skill_roots_with_home_dir(Some(fs), &config_layer_stack, cwd, home_dir, Vec::new()).await
}

#[cfg(test)]
pub(crate) fn skill_config_layer_stack_from_config_layer_stack(
    stack: &config_service::ConfigLayerStack,
) -> SkillConfigLayerStack {
    let layers = stack
        .get_layers(
            config_service::ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ true,
        )
        .into_iter()
        .map(|layer| {
            SkillConfigLayerEntry::new_with_config_folder(
                layer.name.clone(),
                layer.config.clone(),
                layer.config_folder(),
                layer.is_disabled(),
            )
        })
        .collect();
    SkillConfigLayerStack::new(layers)
}

#[cfg(test)]
#[path = "loader_tests.rs"]
mod tests;
