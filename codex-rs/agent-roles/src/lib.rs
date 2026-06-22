use codex_config_state::ConfigLayerStack;
use codex_config_state::ConfigLayerStackOrdering;
use codex_config_toml::config_toml::ConfigToml;
use codex_config_types::AgentRoleToml;
use codex_config_types::AgentsToml;
use codex_file_system::ExecutorFileSystem;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use toml::Value as TomlValue;

const MAX_AGENT_ROLE_DESCRIPTION_LEN: usize = 1024;
const MAX_MARKDOWN_AGENT_BODY_LEN: usize = 8 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentRoleConfig {
    /// Human-facing role documentation used in spawn tool guidance.
    /// Required for loaded user-defined roles after deprecated/new metadata precedence resolves.
    pub description: Option<String>,
    /// Path to a role-specific config layer.
    pub config_file: Option<PathBuf>,
    /// Candidate nicknames for agents spawned with this role.
    pub nickname_candidates: Option<Vec<String>>,
    /// Tool allowlist declared by a Markdown agent definition.
    pub tool_allowlist: AgentCapabilityAllowlist,
    /// Skill allowlist declared by a Markdown agent definition.
    pub skill_allowlist: AgentCapabilityAllowlist,
    /// The model declared by a Markdown agent definition, used for startup context.
    pub model: Option<String>,
    /// The reasoning effort declared by a Markdown agent definition, used for startup context.
    pub model_reasoning_effort: Option<String>,
    /// Source path for startup context and diagnostics.
    pub source_path: Option<PathBuf>,
    /// Source metadata for startup context and diagnostics.
    pub source: Option<AgentRoleSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRoleSource {
    Plugin { plugin_id: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AgentCapabilityAllowlist {
    #[default]
    Inherit,
    All,
    Patterns(Vec<String>),
}

pub async fn load_agent_roles(
    fs: &dyn ExecutorFileSystem,
    cfg: &ConfigToml,
    config_layer_stack: &ConfigLayerStack,
    codex_home: &AbsolutePathBuf,
    cwd: &AbsolutePathBuf,
    startup_warnings: &mut Vec<String>,
) -> std::io::Result<BTreeMap<String, AgentRoleConfig>> {
    let layers = config_layer_stack.get_layers(
        ConfigLayerStackOrdering::LowestPrecedenceFirst,
        /*include_disabled*/ false,
    );
    if layers.is_empty() {
        let mut roles = load_agent_roles_without_layers(fs, cfg).await?;
        merge_agent_roles_from_dirs(
            fs,
            &mut roles,
            &[codex_home.join("agents"), cwd.join(".codex").join("agents")],
            startup_warnings,
        )
        .await?;
        return Ok(roles);
    }

    let mut roles: BTreeMap<String, AgentRoleConfig> = BTreeMap::new();
    let mut scanned_agent_dirs = BTreeSet::new();
    for layer in layers {
        let mut layer_roles: BTreeMap<String, AgentRoleConfig> = BTreeMap::new();
        let mut declared_role_files = BTreeSet::new();
        let config_folder = layer.config_folder();
        let agents_toml = match agents_toml_from_layer(&layer.config, config_folder.as_deref()) {
            Ok(agents_toml) => agents_toml,
            Err(err) => {
                push_agent_role_warning(startup_warnings, err);
                None
            }
        };
        if let Some(agents_toml) = agents_toml {
            for (declared_role_name, role_toml) in &agents_toml.roles {
                let (role_name, role) =
                    match read_declared_role(fs, declared_role_name, role_toml).await {
                        Ok(role) => role,
                        Err(err) => {
                            push_agent_role_warning(startup_warnings, err);
                            continue;
                        }
                    };
                if let Some(config_file) = role.config_file.clone() {
                    declared_role_files.insert(config_file);
                }
                if layer_roles.contains_key(&role_name) {
                    push_agent_role_warning(
                        startup_warnings,
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!(
                                "duplicate agent role name `{role_name}` declared in the same config layer"
                            ),
                        ),
                    );
                    continue;
                }
                layer_roles.insert(role_name, role);
            }
        }

        if let Some(config_folder) = layer.config_folder() {
            let agents_dir = config_folder.join("agents");
            scanned_agent_dirs.insert(agents_dir.to_path_buf());
            for (role_name, role) in
                discover_agent_roles_in_dir(fs, &agents_dir, &declared_role_files, startup_warnings)
                    .await?
            {
                if layer_roles.contains_key(&role_name) {
                    push_agent_role_warning(
                        startup_warnings,
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!(
                                "duplicate agent role name `{role_name}` declared in the same config layer"
                            ),
                        ),
                    );
                    continue;
                }
                layer_roles.insert(role_name, role);
            }
        }

        for (role_name, role) in layer_roles {
            let mut merged_role = role;
            if let Some(existing_role) = roles.get(&role_name) {
                merge_missing_role_fields(&mut merged_role, existing_role);
            }
            if let Err(err) = validate_required_agent_role_description(
                &role_name,
                merged_role.description.as_deref(),
            ) {
                push_agent_role_warning(startup_warnings, err);
                continue;
            }
            roles.insert(role_name, merged_role);
        }
    }

    let extra_agent_dirs = [codex_home.join("agents"), cwd.join(".codex").join("agents")];
    let extra_agent_dirs = extra_agent_dirs
        .iter()
        .filter(|agents_dir| !scanned_agent_dirs.contains(agents_dir.as_path()))
        .cloned()
        .collect::<Vec<_>>();
    merge_agent_roles_from_dirs(fs, &mut roles, &extra_agent_dirs, startup_warnings).await?;

    Ok(roles)
}

pub async fn merge_agent_roles_from_dirs(
    fs: &dyn ExecutorFileSystem,
    roles: &mut BTreeMap<String, AgentRoleConfig>,
    agent_dirs: &[AbsolutePathBuf],
    startup_warnings: &mut Vec<String>,
) -> std::io::Result<()> {
    merge_agent_roles_from_dirs_with_precedence(fs, roles, agent_dirs, startup_warnings).await
}

pub async fn merge_missing_agent_roles_from_plugin_dirs(
    fs: &dyn ExecutorFileSystem,
    roles: &mut BTreeMap<String, AgentRoleConfig>,
    plugin_agent_dirs: &[(String, AbsolutePathBuf)],
    startup_warnings: &mut Vec<String>,
) -> std::io::Result<()> {
    let mut plugin_roles = BTreeMap::new();
    for (plugin_id, agents_dir) in plugin_agent_dirs {
        for (role_name, role) in discover_agent_roles_in_dir_with_source(
            fs,
            agents_dir,
            &BTreeSet::new(),
            startup_warnings,
            Some(AgentRoleSource::Plugin {
                plugin_id: plugin_id.clone(),
            }),
        )
        .await?
        {
            if let Err(err) =
                validate_required_agent_role_description(&role_name, role.description.as_deref())
            {
                push_agent_role_warning(startup_warnings, err);
                continue;
            }
            if plugin_roles.contains_key(&role_name) {
                push_agent_role_warning(
                    startup_warnings,
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("duplicate plugin agent role name `{role_name}`"),
                    ),
                );
                continue;
            }
            plugin_roles.insert(role_name, role);
        }
    }

    for (role_name, role) in plugin_roles {
        roles.entry(role_name).or_insert(role);
    }

    Ok(())
}

async fn merge_agent_roles_from_dirs_with_precedence(
    fs: &dyn ExecutorFileSystem,
    roles: &mut BTreeMap<String, AgentRoleConfig>,
    agent_dirs: &[AbsolutePathBuf],
    startup_warnings: &mut Vec<String>,
) -> std::io::Result<()> {
    for agents_dir in agent_dirs {
        for (role_name, role) in
            discover_agent_roles_in_dir(fs, agents_dir, &BTreeSet::new(), startup_warnings).await?
        {
            if let Err(err) =
                validate_required_agent_role_description(&role_name, role.description.as_deref())
            {
                push_agent_role_warning(startup_warnings, err);
                continue;
            }
            roles.insert(role_name, role);
        }
    }
    Ok(())
}

fn push_agent_role_warning(startup_warnings: &mut Vec<String>, err: std::io::Error) {
    let message = format!("Ignoring malformed agent role definition: {err}");
    tracing::warn!("{message}");
    startup_warnings.push(message);
}

async fn load_agent_roles_without_layers(
    fs: &dyn ExecutorFileSystem,
    cfg: &ConfigToml,
) -> std::io::Result<BTreeMap<String, AgentRoleConfig>> {
    let mut roles = BTreeMap::new();
    if let Some(agents_toml) = cfg.agents.as_ref() {
        for (declared_role_name, role_toml) in &agents_toml.roles {
            let (role_name, role) = read_declared_role(fs, declared_role_name, role_toml).await?;
            validate_required_agent_role_description(&role_name, role.description.as_deref())?;

            if roles.insert(role_name.clone(), role).is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("duplicate agent role name `{role_name}` declared in config"),
                ));
            }
        }
    }

    Ok(roles)
}

async fn read_declared_role(
    fs: &dyn ExecutorFileSystem,
    declared_role_name: &str,
    role_toml: &AgentRoleToml,
) -> std::io::Result<(String, AgentRoleConfig)> {
    let mut role = agent_role_config_from_toml(fs, declared_role_name, role_toml).await?;
    let mut role_name = declared_role_name.to_string();
    if let Some(config_file) = role.config_file.as_deref() {
        let config_file = AbsolutePathBuf::from_absolute_path(config_file)?;
        let is_markdown = is_markdown_agent_role_file(config_file.as_path());
        let role_name_hint = if is_markdown {
            None
        } else {
            Some(declared_role_name)
        };
        let parsed_file = read_resolved_agent_role_file(fs, &config_file, role_name_hint).await?;
        role_name = parsed_file.role_name;
        role.description = if is_markdown {
            parsed_file.description
        } else {
            parsed_file.description.or(role.description)
        };
        role.nickname_candidates = parsed_file.nickname_candidates.or(role.nickname_candidates);
        role.tool_allowlist = parsed_file.tool_allowlist;
        role.skill_allowlist = parsed_file.skill_allowlist;
        role.model = parsed_file.model;
        role.model_reasoning_effort = parsed_file.model_reasoning_effort;
        role.source_path = Some(config_file.to_path_buf());
    }

    Ok((role_name, role))
}

fn merge_missing_role_fields(role: &mut AgentRoleConfig, fallback: &AgentRoleConfig) {
    role.description = role.description.clone().or(fallback.description.clone());
    role.config_file = role.config_file.clone().or(fallback.config_file.clone());
    role.nickname_candidates = role
        .nickname_candidates
        .clone()
        .or(fallback.nickname_candidates.clone());
    if role.tool_allowlist == AgentCapabilityAllowlist::Inherit {
        role.tool_allowlist = fallback.tool_allowlist.clone();
    }
    if role.skill_allowlist == AgentCapabilityAllowlist::Inherit {
        role.skill_allowlist = fallback.skill_allowlist.clone();
    }
    role.model = role.model.clone().or(fallback.model.clone());
    role.model_reasoning_effort = role
        .model_reasoning_effort
        .clone()
        .or(fallback.model_reasoning_effort.clone());
    role.source_path = role.source_path.clone().or(fallback.source_path.clone());
    role.source = role.source.clone().or(fallback.source.clone());
}

fn agents_toml_from_layer(
    layer_toml: &TomlValue,
    config_base_dir: Option<&Path>,
) -> std::io::Result<Option<AgentsToml>> {
    let Some(agents_toml) = layer_toml.get("agents") else {
        return Ok(None);
    };

    // AbsolutePathBufGuard resolves relative paths while it remains in scope.
    let _guard = config_base_dir.map(AbsolutePathBufGuard::new);
    agents_toml
        .clone()
        .try_into()
        .map(Some)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

async fn agent_role_config_from_toml(
    fs: &dyn ExecutorFileSystem,
    role_name: &str,
    role: &AgentRoleToml,
) -> std::io::Result<AgentRoleConfig> {
    let config_file = role
        .config_file
        .as_ref()
        .map(AbsolutePathBuf::from_absolute_path)
        .transpose()?;
    validate_agent_role_config_file(fs, role_name, config_file.as_ref()).await?;
    let description = normalize_agent_role_description(
        &format!("agents.{role_name}.description"),
        role.description.as_deref(),
    )?;
    let nickname_candidates = normalize_agent_role_nickname_candidates(
        &format!("agents.{role_name}.nickname_candidates"),
        role.nickname_candidates.as_deref(),
    )?;

    Ok(AgentRoleConfig {
        description,
        config_file: config_file.map(AbsolutePathBuf::into_path_buf),
        nickname_candidates,
        ..Default::default()
    })
}

#[derive(Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawAgentRoleFileToml {
    name: Option<String>,
    description: Option<String>,
    nickname_candidates: Option<Vec<String>>,
    #[serde(flatten)]
    config: ConfigToml,
}

#[derive(Deserialize, Debug, Clone, Default, PartialEq)]
struct RawAgentRoleFileMarkdown {
    name: Option<String>,
    description: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    model_reasoning_effort: Option<String>,
    tools: Option<RawAgentCapabilityAllowlist>,
    skills: Option<RawAgentCapabilityAllowlist>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
enum RawAgentCapabilityAllowlist {
    All(String),
    Patterns(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAgentRoleFile {
    pub role_name: String,
    pub description: Option<String>,
    pub nickname_candidates: Option<Vec<String>>,
    pub config: TomlValue,
    pub tool_allowlist: AgentCapabilityAllowlist,
    pub skill_allowlist: AgentCapabilityAllowlist,
    pub model: Option<String>,
    pub model_reasoning_effort: Option<String>,
}

pub fn parse_agent_role_file_contents(
    contents: &str,
    role_file_label: &Path,
    config_base_dir: &Path,
    role_name_hint: Option<&str>,
) -> std::io::Result<ResolvedAgentRoleFile> {
    if is_markdown_agent_role_file(role_file_label) {
        return parse_markdown_agent_role_file_contents(contents, role_file_label);
    }

    let role_file_toml: TomlValue = toml::from_str(contents).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "failed to parse agent role file at {}: {err}",
                role_file_label.display()
            ),
        )
    })?;
    let _guard = AbsolutePathBufGuard::new(config_base_dir);
    let parsed: RawAgentRoleFileToml = role_file_toml.clone().try_into().map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "failed to deserialize agent role file at {}: {err}",
                role_file_label.display()
            ),
        )
    })?;
    let description = normalize_markdown_agent_role_description(
        &format!("agent role file {}.description", role_file_label.display()),
        parsed.description.as_deref(),
    )?;
    validate_agent_role_file_developer_instructions(
        role_file_label,
        parsed.config.developer_instructions.as_deref(),
        role_name_hint.is_none(),
    )?;

    let role_name = parsed
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| role_name_hint.map(str::to_string))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "agent role file at {} must define a non-empty `name`",
                    role_file_label.display()
                ),
            )
        })?;

    let nickname_candidates = normalize_agent_role_nickname_candidates(
        &format!(
            "agent role file {}.nickname_candidates",
            role_file_label.display()
        ),
        parsed.nickname_candidates.as_deref(),
    )?;

    let mut config = role_file_toml;
    let Some(config_table) = config.as_table_mut() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "agent role file at {} must contain a TOML table",
                role_file_label.display()
            ),
        ));
    };
    config_table.remove("name");
    config_table.remove("description");
    config_table.remove("nickname_candidates");

    Ok(ResolvedAgentRoleFile {
        role_name,
        description,
        nickname_candidates,
        config,
        tool_allowlist: AgentCapabilityAllowlist::Inherit,
        skill_allowlist: AgentCapabilityAllowlist::Inherit,
        model: None,
        model_reasoning_effort: None,
    })
}

fn parse_markdown_agent_role_file_contents(
    contents: &str,
    role_file_label: &Path,
) -> std::io::Result<ResolvedAgentRoleFile> {
    let (frontmatter, body) = extract_agent_markdown_frontmatter(contents, role_file_label)?;
    let parsed = parse_agent_markdown_frontmatter(frontmatter).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "failed to parse agent role file frontmatter at {}: {err}",
                role_file_label.display()
            ),
        )
    })?;
    if parsed.effort.is_some() && parsed.model_reasoning_effort.is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "agent role file at {} cannot define both `effort` and `model_reasoning_effort`",
                role_file_label.display()
            ),
        ));
    }

    let description = normalize_markdown_agent_role_description(
        &format!("agent role file {}.description", role_file_label.display()),
        parsed.description.as_deref(),
    )?;
    validate_agent_role_file_developer_instructions(role_file_label, Some(body), true)?;
    if body.chars().count() > MAX_MARKDOWN_AGENT_BODY_LEN {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "agent role file at {} body must be {MAX_MARKDOWN_AGENT_BODY_LEN} characters or fewer",
                role_file_label.display()
            ),
        ));
    }
    let role_name = markdown_agent_role_name(parsed.name.as_deref(), role_file_label)?;
    let model_reasoning_effort = parsed
        .model_reasoning_effort
        .clone()
        .or_else(|| parsed.effort.clone());
    let config = markdown_agent_config_toml(
        body.trim(),
        parsed.model.as_deref(),
        model_reasoning_effort.as_deref(),
    );

    Ok(ResolvedAgentRoleFile {
        role_name,
        description,
        nickname_candidates: None,
        config,
        tool_allowlist: normalize_markdown_agent_allowlist(
            parsed.tools,
            &format!("agent role file {}.tools", role_file_label.display()),
        )?,
        skill_allowlist: normalize_markdown_agent_allowlist(
            parsed.skills,
            &format!("agent role file {}.skills", role_file_label.display()),
        )?,
        model: parsed.model,
        model_reasoning_effort,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentMarkdownFrontmatterBlock {
    None,
    Tools,
    Skills,
    Unknown,
}

fn parse_agent_markdown_frontmatter(frontmatter: &str) -> Result<RawAgentRoleFileMarkdown, String> {
    let mut parsed = RawAgentRoleFileMarkdown::default();
    let mut block = AgentMarkdownFrontmatterBlock::None;

    for (line_index, raw_line) in frontmatter.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed_start = line.trim_start();
        if trimmed_start.is_empty() || trimmed_start.starts_with('#') {
            continue;
        }

        let indent = agent_frontmatter_indent(line, line_number)?;
        let trimmed = &line[indent..];
        if indent == 0 {
            let (key, value) = split_agent_frontmatter_key_value(trimmed, line_number)?;
            match key {
                "name" => {
                    set_agent_frontmatter_string(&mut parsed.name, key, value, line_number)?;
                    block = AgentMarkdownFrontmatterBlock::None;
                }
                "description" => {
                    set_agent_frontmatter_string(&mut parsed.description, key, value, line_number)?;
                    block = AgentMarkdownFrontmatterBlock::None;
                }
                "model" => {
                    set_agent_frontmatter_string(&mut parsed.model, key, value, line_number)?;
                    block = AgentMarkdownFrontmatterBlock::None;
                }
                "effort" => {
                    set_agent_frontmatter_string(&mut parsed.effort, key, value, line_number)?;
                    block = AgentMarkdownFrontmatterBlock::None;
                }
                "model_reasoning_effort" => {
                    set_agent_frontmatter_string(
                        &mut parsed.model_reasoning_effort,
                        key,
                        value,
                        line_number,
                    )?;
                    block = AgentMarkdownFrontmatterBlock::None;
                }
                "tools" => {
                    block = parse_agent_frontmatter_allowlist_field(
                        &mut parsed.tools,
                        key,
                        value,
                        line_number,
                        AgentMarkdownFrontmatterBlock::Tools,
                    )?;
                }
                "skills" => {
                    block = parse_agent_frontmatter_allowlist_field(
                        &mut parsed.skills,
                        key,
                        value,
                        line_number,
                        AgentMarkdownFrontmatterBlock::Skills,
                    )?;
                }
                _ => {
                    block = if value.trim().is_empty() {
                        AgentMarkdownFrontmatterBlock::Unknown
                    } else {
                        AgentMarkdownFrontmatterBlock::None
                    };
                }
            }
            continue;
        }

        match block {
            AgentMarkdownFrontmatterBlock::Tools => parse_agent_frontmatter_allowlist_item(
                &mut parsed.tools,
                "tools",
                indent,
                trimmed,
                line_number,
            )?,
            AgentMarkdownFrontmatterBlock::Skills => parse_agent_frontmatter_allowlist_item(
                &mut parsed.skills,
                "skills",
                indent,
                trimmed,
                line_number,
            )?,
            AgentMarkdownFrontmatterBlock::Unknown => {}
            AgentMarkdownFrontmatterBlock::None => {
                return Err(format!("line {line_number}: unexpected indented line"));
            }
        }
    }

    Ok(parsed)
}

fn parse_agent_frontmatter_allowlist_field(
    target: &mut Option<RawAgentCapabilityAllowlist>,
    field: &str,
    value: &str,
    line_number: usize,
    block: AgentMarkdownFrontmatterBlock,
) -> Result<AgentMarkdownFrontmatterBlock, String> {
    if target.is_some() {
        return Err(format!("line {line_number}: duplicate field `{field}`"));
    }
    let value = value.trim();
    if value.is_empty() {
        *target = Some(RawAgentCapabilityAllowlist::Patterns(Vec::new()));
        return Ok(block);
    }
    if value.starts_with('[') {
        *target = Some(RawAgentCapabilityAllowlist::Patterns(
            parse_agent_frontmatter_flow_list(value, line_number)?,
        ));
        return Ok(AgentMarkdownFrontmatterBlock::None);
    }
    *target = Some(RawAgentCapabilityAllowlist::All(
        parse_agent_frontmatter_scalar(value, line_number)?,
    ));
    Ok(AgentMarkdownFrontmatterBlock::None)
}

fn parse_agent_frontmatter_allowlist_item(
    target: &mut Option<RawAgentCapabilityAllowlist>,
    field: &str,
    indent: usize,
    trimmed: &str,
    line_number: usize,
) -> Result<(), String> {
    if indent != 2 {
        return Err(format!(
            "line {line_number}: `{field}` list items must be indented by two spaces"
        ));
    }
    let Some(item) = trimmed.strip_prefix("- ") else {
        return Err(format!(
            "line {line_number}: `{field}` entries must use `- value` list items"
        ));
    };
    let value = parse_agent_frontmatter_scalar(item, line_number)?;
    match target {
        Some(RawAgentCapabilityAllowlist::Patterns(patterns)) => {
            patterns.push(value);
            Ok(())
        }
        _ => Err(format!(
            "line {line_number}: `{field}` block list was not initialized"
        )),
    }
}

fn agent_frontmatter_indent(line: &str, line_number: usize) -> Result<usize, String> {
    let mut indent = 0;
    for byte in line.bytes() {
        match byte {
            b' ' => indent += 1,
            b'\t' => {
                return Err(format!(
                    "line {line_number}: tabs are not supported in agent role frontmatter"
                ));
            }
            _ => break,
        }
    }
    Ok(indent)
}

fn split_agent_frontmatter_key_value<'a>(
    line: &'a str,
    line_number: usize,
) -> Result<(&'a str, &'a str), String> {
    let Some((key, value)) = line.split_once(':') else {
        return Err(format!("line {line_number}: expected `key: value`"));
    };
    let key = key.trim();
    if key.is_empty() {
        return Err(format!(
            "line {line_number}: frontmatter key must not be empty"
        ));
    }
    Ok((key, value.trim_start()))
}

fn set_agent_frontmatter_string(
    target: &mut Option<String>,
    field: &str,
    value: &str,
    line_number: usize,
) -> Result<(), String> {
    if target.is_some() {
        return Err(format!("line {line_number}: duplicate field `{field}`"));
    }
    *target = Some(parse_agent_frontmatter_scalar(value, line_number)?);
    Ok(())
}

fn parse_agent_frontmatter_flow_list(
    value: &str,
    line_number: usize,
) -> Result<Vec<String>, String> {
    let (inside, trailing) = agent_frontmatter_bracketed_list(value, line_number)?;
    validate_agent_frontmatter_trailing_comment(trailing, line_number)?;
    let mut items = Vec::new();
    let mut rest = inside.trim();
    while !rest.is_empty() {
        let (item, next) = parse_agent_frontmatter_list_item(rest, line_number)?;
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

fn agent_frontmatter_bracketed_list(
    value: &str,
    line_number: usize,
) -> Result<(&str, &str), String> {
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
            if active_quote == '\'' && ch == '\'' {
                if iter.peek().is_some_and(|(_, next)| *next == '\'') {
                    iter.next();
                    continue;
                }
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

fn parse_agent_frontmatter_list_item(
    value: &str,
    line_number: usize,
) -> Result<(String, &str), String> {
    if value.starts_with('"') {
        let end = agent_frontmatter_double_quote_end(value, line_number)?;
        validate_agent_frontmatter_scalar_separator(&value[end + 1..], line_number)?;
        return serde_json::from_str::<String>(&value[..=end])
            .map(|parsed| (parsed, &value[end + 1..]))
            .map_err(|err| format!("line {line_number}: invalid double-quoted scalar: {err}"));
    }
    if value.starts_with('\'') {
        let (parsed, trailing) = parse_agent_frontmatter_single_quoted_scalar(value, line_number)?;
        validate_agent_frontmatter_scalar_separator(trailing, line_number)?;
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

fn validate_agent_frontmatter_scalar_separator(
    trailing: &str,
    line_number: usize,
) -> Result<(), String> {
    let trailing = trailing.trim_start();
    if trailing.is_empty() || trailing.starts_with(',') {
        return Ok(());
    }
    Err(format!(
        "line {line_number}: inline list items must be separated by commas"
    ))
}

fn parse_agent_frontmatter_scalar(value: &str, line_number: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!(
            "line {line_number}: scalar value must not be empty"
        ));
    }
    if value.starts_with('"') {
        let end = agent_frontmatter_double_quote_end(value, line_number)?;
        validate_agent_frontmatter_trailing_comment(&value[end + 1..], line_number)?;
        return serde_json::from_str::<String>(&value[..=end])
            .map_err(|err| format!("line {line_number}: invalid double-quoted scalar: {err}"));
    }
    if value.starts_with('\'') {
        let (parsed, trailing) = parse_agent_frontmatter_single_quoted_scalar(value, line_number)?;
        validate_agent_frontmatter_trailing_comment(trailing, line_number)?;
        return Ok(parsed);
    }
    if value.starts_with('[') || value.starts_with('{') {
        return Err(format!(
            "line {line_number}: inline collections are only supported for tools and skills"
        ));
    }
    let value = strip_agent_frontmatter_inline_comment(value).trim_end();
    if value.is_empty() {
        return Err(format!(
            "line {line_number}: scalar value must not be empty"
        ));
    }
    Ok(value.to_string())
}

fn agent_frontmatter_double_quote_end(value: &str, line_number: usize) -> Result<usize, String> {
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

fn parse_agent_frontmatter_single_quoted_scalar<'a>(
    value: &'a str,
    line_number: usize,
) -> Result<(String, &'a str), String> {
    let mut parsed = String::new();
    let mut index = 1;
    while index < value.len() {
        let ch = value[index..]
            .chars()
            .next()
            .expect("index stays on char boundary");
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

fn validate_agent_frontmatter_trailing_comment(
    trailing: &str,
    line_number: usize,
) -> Result<(), String> {
    let trailing = trailing.trim();
    if trailing.is_empty() || trailing.starts_with('#') {
        return Ok(());
    }
    Err(format!(
        "line {line_number}: unexpected content after scalar value"
    ))
}

fn strip_agent_frontmatter_inline_comment(value: &str) -> &str {
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

fn markdown_agent_role_name(name: Option<&str>, role_file_label: &Path) -> std::io::Result<String> {
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        return Ok(name.to_string());
    }

    let Some(file_stem) = role_file_label.file_stem().and_then(|stem| stem.to_str()) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "agent role file at {} must define a non-empty `name` or have a UTF-8 file name",
                role_file_label.display()
            ),
        ));
    };
    let file_stem = file_stem.trim();
    if file_stem.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "agent role file at {} must define a non-empty `name` or have a non-empty file name",
                role_file_label.display()
            ),
        ));
    }
    Ok(file_stem.to_string())
}

fn extract_agent_markdown_frontmatter<'a>(
    contents: &'a str,
    role_file_label: &Path,
) -> std::io::Result<(&'a str, &'a str)> {
    let Some(rest) = contents
        .strip_prefix("---\n")
        .or_else(|| contents.strip_prefix("---\r\n"))
    else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "agent role file at {} must start with YAML frontmatter delimited by ---",
                role_file_label.display()
            ),
        ));
    };
    let Some((frontmatter, body)) = rest.split_once("\n---") else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "agent role file at {} must close YAML frontmatter with ---",
                role_file_label.display()
            ),
        ));
    };
    let body = body
        .strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body);
    Ok((frontmatter, body))
}

fn markdown_agent_config_toml(
    developer_instructions: &str,
    model: Option<&str>,
    model_reasoning_effort: Option<&str>,
) -> TomlValue {
    let mut table = toml::map::Map::new();
    table.insert(
        "developer_instructions".to_string(),
        TomlValue::String(developer_instructions.to_string()),
    );
    if let Some(model) = model {
        table.insert("model".to_string(), TomlValue::String(model.to_string()));
    }
    if let Some(model_reasoning_effort) = model_reasoning_effort {
        table.insert(
            "model_reasoning_effort".to_string(),
            TomlValue::String(model_reasoning_effort.to_string()),
        );
    }
    TomlValue::Table(table)
}

fn normalize_agent_allowlist(
    raw: Option<RawAgentCapabilityAllowlist>,
    field_label: &str,
) -> std::io::Result<AgentCapabilityAllowlist> {
    match raw {
        None => Ok(AgentCapabilityAllowlist::Inherit),
        Some(RawAgentCapabilityAllowlist::All(value)) if value.trim() == "*" => {
            Ok(AgentCapabilityAllowlist::All)
        }
        Some(RawAgentCapabilityAllowlist::All(value)) => {
            normalize_agent_allowlist_patterns(vec![value], field_label)
        }
        Some(RawAgentCapabilityAllowlist::Patterns(patterns)) => {
            normalize_agent_allowlist_patterns(patterns, field_label)
        }
    }
}

fn normalize_markdown_agent_allowlist(
    raw: Option<RawAgentCapabilityAllowlist>,
    field_label: &str,
) -> std::io::Result<AgentCapabilityAllowlist> {
    match raw {
        None => Ok(AgentCapabilityAllowlist::All),
        Some(raw) => normalize_agent_allowlist(Some(raw), field_label),
    }
}

fn normalize_agent_allowlist_patterns(
    patterns: Vec<String>,
    field_label: &str,
) -> std::io::Result<AgentCapabilityAllowlist> {
    let mut normalized = Vec::new();
    for pattern in patterns {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{field_label} cannot contain blank patterns"),
            ));
        }
        if pattern == "*" {
            return Ok(AgentCapabilityAllowlist::All);
        }
        normalized.push(pattern.to_string());
    }
    Ok(AgentCapabilityAllowlist::Patterns(normalized))
}

fn is_markdown_agent_role_file(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
    })
}

async fn read_resolved_agent_role_file(
    fs: &dyn ExecutorFileSystem,
    path: &AbsolutePathBuf,
    role_name_hint: Option<&str>,
) -> std::io::Result<ResolvedAgentRoleFile> {
    let contents = fs.read_file_text(path, /*sandbox*/ None).await?;
    let config_base_dir = path.parent().unwrap_or_else(|| path.clone());
    parse_agent_role_file_contents(
        &contents,
        path.as_path(),
        config_base_dir.as_path(),
        role_name_hint,
    )
}

fn normalize_agent_role_description(
    field_label: &str,
    description: Option<&str>,
) -> std::io::Result<Option<String>> {
    let Some(description) = description else {
        return Ok(None);
    };
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{field_label} cannot be blank"),
        ));
    }
    Ok(Some(description))
}

fn normalize_markdown_agent_role_description(
    field_label: &str,
    description: Option<&str>,
) -> std::io::Result<Option<String>> {
    let description = normalize_agent_role_description(field_label, description)?;
    if description
        .as_deref()
        .is_some_and(|description| description.chars().count() > MAX_AGENT_ROLE_DESCRIPTION_LEN)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{field_label} must be {MAX_AGENT_ROLE_DESCRIPTION_LEN} characters or fewer"),
        ));
    }
    Ok(description)
}

fn validate_required_agent_role_description(
    role_name: &str,
    description: Option<&str>,
) -> std::io::Result<()> {
    if description.is_some() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("agent role `{role_name}` must define a description"),
        ))
    }
}

fn validate_agent_role_file_developer_instructions(
    role_file_label: &Path,
    developer_instructions: Option<&str>,
    require_present: bool,
) -> std::io::Result<()> {
    match developer_instructions.map(str::trim) {
        Some("") => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "agent role file at {}.developer_instructions cannot be blank",
                role_file_label.display()
            ),
        )),
        Some(_) => Ok(()),
        None if require_present => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "agent role file at {} must define `developer_instructions`",
                role_file_label.display()
            ),
        )),
        None => Ok(()),
    }
}

async fn validate_agent_role_config_file(
    fs: &dyn ExecutorFileSystem,
    role_name: &str,
    config_file: Option<&AbsolutePathBuf>,
) -> std::io::Result<()> {
    let Some(config_file) = config_file else {
        return Ok(());
    };

    let metadata = fs
        .get_metadata(config_file, /*sandbox*/ None)
        .await
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "agents.{role_name}.config_file must point to an existing file at {}: {e}",
                    config_file.as_path().display()
                ),
            )
        })?;
    if metadata.is_file {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "agents.{role_name}.config_file must point to a file: {}",
                config_file.as_path().display()
            ),
        ))
    }
}

fn normalize_agent_role_nickname_candidates(
    field_label: &str,
    nickname_candidates: Option<&[String]>,
) -> std::io::Result<Option<Vec<String>>> {
    let Some(nickname_candidates) = nickname_candidates else {
        return Ok(None);
    };

    if nickname_candidates.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{field_label} must contain at least one name"),
        ));
    }

    let mut normalized_candidates = Vec::with_capacity(nickname_candidates.len());
    let mut seen_candidates = BTreeSet::new();

    for nickname in nickname_candidates {
        let normalized_nickname = nickname.trim();
        if normalized_nickname.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{field_label} cannot contain blank names"),
            ));
        }

        if !seen_candidates.insert(normalized_nickname.to_owned()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{field_label} cannot contain duplicates"),
            ));
        }

        if !normalized_nickname
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{field_label} may only contain ASCII letters, digits, spaces, hyphens, and underscores"
                ),
            ));
        }

        normalized_candidates.push(normalized_nickname.to_owned());
    }

    Ok(Some(normalized_candidates))
}

async fn discover_agent_roles_in_dir(
    fs: &dyn ExecutorFileSystem,
    agents_dir: &AbsolutePathBuf,
    declared_role_files: &BTreeSet<PathBuf>,
    startup_warnings: &mut Vec<String>,
) -> std::io::Result<BTreeMap<String, AgentRoleConfig>> {
    discover_agent_roles_in_dir_with_source(
        fs,
        agents_dir,
        declared_role_files,
        startup_warnings,
        None,
    )
    .await
}

async fn discover_agent_roles_in_dir_with_source(
    fs: &dyn ExecutorFileSystem,
    agents_dir: &AbsolutePathBuf,
    declared_role_files: &BTreeSet<PathBuf>,
    startup_warnings: &mut Vec<String>,
    source: Option<AgentRoleSource>,
) -> std::io::Result<BTreeMap<String, AgentRoleConfig>> {
    let mut roles = BTreeMap::new();

    for agent_file in collect_agent_role_files(fs, agents_dir).await? {
        if declared_role_files.contains(agent_file.as_path()) {
            continue;
        }
        let parsed_file =
            match read_resolved_agent_role_file(fs, &agent_file, /*role_name_hint*/ None).await {
                Ok(parsed_file) => parsed_file,
                Err(err) => {
                    push_agent_role_warning(startup_warnings, err);
                    continue;
                }
            };
        let role_name = parsed_file.role_name;
        if roles.contains_key(&role_name) {
            push_agent_role_warning(
                startup_warnings,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "duplicate agent role name `{role_name}` discovered in {}",
                        agents_dir.as_path().display()
                    ),
                ),
            );
            continue;
        }
        roles.insert(
            role_name,
            AgentRoleConfig {
                description: parsed_file.description,
                config_file: Some(agent_file.to_path_buf()),
                nickname_candidates: parsed_file.nickname_candidates,
                tool_allowlist: parsed_file.tool_allowlist,
                skill_allowlist: parsed_file.skill_allowlist,
                model: parsed_file.model,
                model_reasoning_effort: parsed_file.model_reasoning_effort,
                source_path: Some(agent_file.to_path_buf()),
                source: source.clone(),
            },
        );
    }

    Ok(roles)
}

async fn collect_agent_role_files(
    fs: &dyn ExecutorFileSystem,
    dir: &AbsolutePathBuf,
) -> std::io::Result<Vec<AbsolutePathBuf>> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.clone()];
    while let Some(current_dir) = dirs.pop() {
        let entries = match fs.read_directory(&current_dir, /*sandbox*/ None).await {
            Ok(entries) => entries,
            Err(err) if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
                continue;
            }
            Err(err) => return Err(err),
        };

        for entry in entries {
            let path = current_dir.join(entry.file_name);
            if entry.is_directory {
                dirs.push(path);
                continue;
            }
            if !entry.is_file {
                continue;
            }

            let Some(extension) = path.as_path().extension() else {
                continue;
            };
            if extension.eq_ignore_ascii_case("toml")
                || extension.eq_ignore_ascii_case("md")
                || extension.eq_ignore_ascii_case("markdown")
            {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_agent_description_has_hard_length_limit() {
        let description = "x".repeat(MAX_AGENT_ROLE_DESCRIPTION_LEN + 1);
        let contents =
            format!("---\nname: reviewer\ndescription: {description}\n---\n\nReview carefully.");

        let err = parse_agent_role_file_contents(
            &contents,
            Path::new("reviewer.md"),
            Path::new("."),
            /*role_name_hint*/ None,
        )
        .expect_err("oversized descriptions should be rejected");

        assert!(err.to_string().contains("1024 characters or fewer"));
    }

    #[test]
    fn markdown_agent_frontmatter_name_defaults_to_file_stem() {
        let contents = "---\ndescription: Review code.\n---\n\nReview carefully.";

        let parsed = parse_agent_role_file_contents(
            contents,
            Path::new("reviewer.md"),
            Path::new("."),
            Some("reviewer"),
        )
        .expect("markdown name should default to the file stem");

        assert_eq!(parsed.role_name, "reviewer");
    }

    #[test]
    fn markdown_agent_frontmatter_ignores_unknown_fields() {
        let contents = r#"---
description: Review code.
level: project
parallelizable: true
schema_version: 1
---

Review carefully.
"#;

        let parsed = parse_agent_role_file_contents(
            contents,
            Path::new("reviewer.md"),
            Path::new("."),
            /*role_name_hint*/ None,
        )
        .expect("unknown markdown frontmatter fields should be ignored");

        assert_eq!(parsed.role_name, "reviewer");
        assert_eq!(parsed.description.as_deref(), Some("Review code."));
    }

    #[test]
    fn markdown_agent_frontmatter_missing_tools_and_skills_default_to_all() {
        let contents = "---\ndescription: Review code.\n---\n\nReview carefully.";

        let parsed = parse_agent_role_file_contents(
            contents,
            Path::new("reviewer.md"),
            Path::new("."),
            /*role_name_hint*/ None,
        )
        .expect("missing allowlists should parse");

        assert_eq!(parsed.tool_allowlist, AgentCapabilityAllowlist::All);
        assert_eq!(parsed.skill_allowlist, AgentCapabilityAllowlist::All);
    }

    #[test]
    fn markdown_agent_body_has_hard_length_limit() {
        let body = "x".repeat(MAX_MARKDOWN_AGENT_BODY_LEN + 1);
        let contents = format!("---\nname: reviewer\ndescription: Review code.\n---\n\n{body}");

        let err = parse_agent_role_file_contents(
            &contents,
            Path::new("reviewer.md"),
            Path::new("."),
            /*role_name_hint*/ None,
        )
        .expect_err("oversized body should be rejected");

        assert!(err.to_string().contains("8192 characters or fewer"));
    }

    #[test]
    fn markdown_agent_frontmatter_accepts_crlf_opening_delimiter() {
        let parsed = parse_agent_role_file_contents(
            "---\r\nname: reviewer\r\ndescription: Reviews code.\r\n---\r\n\r\nReview carefully.",
            Path::new("reviewer.md"),
            Path::new("."),
            /*role_name_hint*/ None,
        )
        .expect("crlf frontmatter should parse");

        assert_eq!(parsed.role_name, "reviewer");
        assert_eq!(parsed.description.as_deref(), Some("Reviews code."));
    }
}
