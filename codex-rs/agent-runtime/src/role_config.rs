//! Applies agent-role configuration layers on top of an existing session config.
//!
//! Roles are selected at spawn time and are loaded with the same config machinery as
//! `config.toml`. This module resolves built-in and user-defined role files, inserts the role as a
//! high-precedence layer, and preserves the caller's current profile/provider unless the role
//! explicitly takes ownership of model selection. It does not decide when to spawn a sub-agent or
//! which role to use; the multi-agent tool handler owns that orchestration.

use anyhow::anyhow;
use codex_agent_roles::AgentCapabilityAllowlist;
use codex_agent_roles::AgentRoleConfig;
use codex_agent_roles::DEFAULT_ROLE_NAME;
use codex_agent_roles::built_in_config_file_contents;
use codex_agent_roles::parse_agent_role_file_contents;
use codex_agent_roles::resolve_role_config;
use codex_config_toml::config_toml::ConfigToml;
use codex_config_toml::deserialize_config_toml_with_base;
use codex_config_toml::resolve_relative_paths_in_config_toml;
use codex_config_types::ConfigLayerSource;
use codex_file_system::LOCAL_FS;
use config_service::Config;
use config_service::ConfigLayerEntry;
use config_service::ConfigLayerStack;
use config_service::ConfigLayerStackOrdering;
use config_service::ConfigOverrides;
use serde::Serialize;
use std::path::Path;
use toml::Value as TomlValue;

pub const AGENT_TYPE_UNAVAILABLE_ERROR: &str = "agent type is currently not available";

/// Applies a named role layer to `config` while preserving caller-owned model selection.
///
/// The role layer is inserted at session-flag precedence so it can override persisted config, but
/// the caller's current `profile` and `model_provider` remain sticky runtime choices unless the
/// role explicitly sets `profile`, explicitly sets `model_provider`, or rewrites the active
/// profile's `model_provider` in place. Rebuilding the config without those overrides would make a
/// spawned agent silently fall back to the default provider, which is the bug this preservation
/// logic avoids.
pub async fn apply_role_to_config(
    config: &mut Config,
    role_name: Option<&str>,
) -> Result<(), String> {
    let role_name = role_name.unwrap_or(DEFAULT_ROLE_NAME);

    let role = resolve_role_config(&config.agent_roles, role_name)
        .cloned()
        .ok_or_else(|| format!("unknown agent_type '{role_name}'"))?;

    apply_role_to_config_inner(config, role_name, &role)
        .await
        .map_err(|err| {
            tracing::warn!("failed to apply role to config: {err}");
            AGENT_TYPE_UNAVAILABLE_ERROR.to_string()
        })
}

async fn apply_role_to_config_inner(
    config: &mut Config,
    role_name: &str,
    role: &AgentRoleConfig,
) -> anyhow::Result<()> {
    let is_built_in = !config.agent_roles.contains_key(role_name);
    let Some(config_file) = role.config_file.as_ref() else {
        return Ok(());
    };
    let role_layer_toml = load_role_layer_toml(config, config_file, is_built_in, role_name).await?;
    if role_layer_toml
        .as_table()
        .is_some_and(toml::map::Map::is_empty)
    {
        return Ok(());
    }
    let inherited_tool_patterns = config.agent_tool_patterns.clone();
    let inherited_skill_patterns = config.agent_skill_patterns.clone();
    let (preserve_current_profile, preserve_current_provider) =
        preservation_policy(config, &role_layer_toml);

    let current_config = config.clone();
    let mut next_config = reload::build_next_config(
        config,
        role_layer_toml.clone(),
        preserve_current_profile,
        preserve_current_provider,
    )
    .await?;
    preserve_runtime_session_fields(&current_config, &mut next_config, &role_layer_toml);
    *config = next_config;
    config.agent_tool_patterns =
        resolve_allowlist_patterns(&role.tool_allowlist, inherited_tool_patterns);
    config.agent_skill_patterns =
        resolve_allowlist_patterns(&role.skill_allowlist, inherited_skill_patterns);
    Ok(())
}

fn preserve_runtime_session_fields(
    current_config: &Config,
    next_config: &mut Config,
    role_layer_toml: &TomlValue,
) {
    if role_preserves_field(current_config, role_layer_toml, "model") {
        next_config.model = current_config.model.clone();
    }
    if role_preserves_field(current_config, role_layer_toml, "model_reasoning_effort") {
        next_config.model_reasoning_effort = current_config.model_reasoning_effort;
    }
    if role_preserves_field(current_config, role_layer_toml, "model_reasoning_summary") {
        next_config.model_reasoning_summary = current_config.model_reasoning_summary;
    }
    if role_preserves_field(current_config, role_layer_toml, "service_tier") {
        next_config.service_tier = current_config.service_tier.clone();
    }
    if role_preserves_field(current_config, role_layer_toml, "personality") {
        next_config.personality = current_config.personality;
    }
    if role_preserves_field(current_config, role_layer_toml, "approval_policy") {
        next_config.permissions.approval_policy =
            current_config.permissions.approval_policy.clone();
    }
    if role_preserves_field(current_config, role_layer_toml, "approvals_reviewer") {
        next_config.approvals_reviewer = current_config.approvals_reviewer;
    }
    if role_preserves_field(current_config, role_layer_toml, "developer_instructions") {
        next_config.developer_instructions = current_config.developer_instructions.clone();
    }
    if role_preserves_field(current_config, role_layer_toml, "compact_prompt") {
        next_config.compact_prompt = current_config.compact_prompt.clone();
    }
    if role_preserves_permission_profile(current_config, role_layer_toml) {
        let role_permissions = next_config.permissions.clone();
        next_config.permissions = current_config.permissions.clone();
        if !role_preserves_field(current_config, role_layer_toml, "approval_policy") {
            next_config.permissions.approval_policy = role_permissions.approval_policy;
        }
        if !role_preserves_field(current_config, role_layer_toml, "allow_login_shell") {
            next_config.permissions.allow_login_shell = role_permissions.allow_login_shell;
        }
        if !role_preserves_field(current_config, role_layer_toml, "shell_environment_policy") {
            next_config.permissions.shell_environment_policy =
                role_permissions.shell_environment_policy;
        }
        if !role_preserves_field(current_config, role_layer_toml, "windows") {
            next_config.permissions.windows_sandbox_mode = role_permissions.windows_sandbox_mode;
            next_config.permissions.windows_sandbox_private_desktop =
                role_permissions.windows_sandbox_private_desktop;
        }
        next_config.workspace_roots = current_config.workspace_roots.clone();
        next_config.workspace_roots_explicit = current_config.workspace_roots_explicit;
    }
}

fn role_preserves_field(config: &Config, role_layer_toml: &TomlValue, key: &str) -> bool {
    role_layer_toml.get("profile").is_none()
        && role_layer_toml.get(key).is_none()
        && !role_active_profile_contains(config, role_layer_toml, key)
}

fn role_preserves_permission_profile(config: &Config, role_layer_toml: &TomlValue) -> bool {
    role_layer_toml.get("profile").is_none()
        && [
            "sandbox_mode",
            "default_permissions",
            "permissions",
            "sandbox_workspace_write",
        ]
        .into_iter()
        .all(|key| {
            role_layer_toml.get(key).is_none()
                && !role_active_profile_contains(config, role_layer_toml, key)
        })
}

fn role_active_profile_contains(config: &Config, role_layer_toml: &TomlValue, key: &str) -> bool {
    let Some(active_profile) = config.active_profile.as_deref() else {
        return false;
    };
    role_layer_toml
        .get("profiles")
        .and_then(TomlValue::as_table)
        .and_then(|profiles| profiles.get(active_profile))
        .and_then(TomlValue::as_table)
        .is_some_and(|profile| profile.contains_key(key))
}

fn resolve_allowlist_patterns(
    allowlist: &AgentCapabilityAllowlist,
    inherited_patterns: Option<Vec<String>>,
) -> Option<Vec<String>> {
    match allowlist {
        AgentCapabilityAllowlist::Inherit => inherited_patterns,
        AgentCapabilityAllowlist::All => None,
        AgentCapabilityAllowlist::Patterns(patterns) => Some(patterns.clone()),
    }
}

async fn load_role_layer_toml(
    config: &Config,
    config_file: &Path,
    is_built_in: bool,
    role_name: &str,
) -> anyhow::Result<TomlValue> {
    let (role_config_toml, role_config_base) = if is_built_in {
        let role_config_contents = built_in_config_file_contents(config_file)
            .map(str::to_owned)
            .ok_or(anyhow!("No corresponding config content"))?;
        let role_config_toml: TomlValue = toml::from_str(&role_config_contents)?;
        (role_config_toml, config.codex_home.as_path())
    } else {
        let role_config_contents = tokio::fs::read_to_string(config_file).await?;
        let role_config_base = config_file
            .parent()
            .ok_or(anyhow!("No corresponding config content"))?;
        let role_config_toml = parse_agent_role_file_contents(
            &role_config_contents,
            config_file,
            role_config_base,
            Some(role_name),
        )?
        .config;
        (role_config_toml, role_config_base)
    };

    deserialize_config_toml_with_base(role_config_toml.clone(), role_config_base)?;
    Ok(resolve_relative_paths_in_config_toml(
        role_config_toml,
        role_config_base,
    )?)
}

fn preservation_policy(config: &Config, role_layer_toml: &TomlValue) -> (bool, bool) {
    let role_selects_provider = role_layer_toml.get("model_provider").is_some();
    let role_selects_profile = role_layer_toml.get("profile").is_some();
    let role_updates_active_profile_provider = config
        .active_profile
        .as_ref()
        .and_then(|active_profile| {
            role_layer_toml
                .get("profiles")
                .and_then(TomlValue::as_table)
                .and_then(|profiles| profiles.get(active_profile))
                .and_then(TomlValue::as_table)
                .map(|profile| profile.contains_key("model_provider"))
        })
        .unwrap_or(false);
    let preserve_current_profile = !role_selects_provider && !role_selects_profile;
    let preserve_current_provider =
        preserve_current_profile && !role_updates_active_profile_provider;
    (preserve_current_profile, preserve_current_provider)
}

mod reload {
    use super::*;

    pub(super) async fn build_next_config(
        config: &Config,
        role_layer_toml: TomlValue,
        preserve_current_profile: bool,
        preserve_current_provider: bool,
    ) -> anyhow::Result<Config> {
        let active_profile_name = preserve_current_profile
            .then_some(config.active_profile.as_deref())
            .flatten();
        let config_layer_stack =
            build_config_layer_stack(config, &role_layer_toml, active_profile_name)?;
        let mut merged_config = deserialize_effective_config(config, &config_layer_stack)?;
        if preserve_current_profile {
            merged_config.profile = None;
        }

        let mut next_config = Config::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            merged_config,
            reload_overrides(config, preserve_current_provider),
            config.codex_home.clone(),
            config_layer_stack,
        )
        .await?;
        if preserve_current_profile {
            next_config.active_profile = config.active_profile.clone();
        }
        Ok(next_config)
    }

    fn build_config_layer_stack(
        config: &Config,
        role_layer_toml: &TomlValue,
        active_profile_name: Option<&str>,
    ) -> anyhow::Result<ConfigLayerStack> {
        let mut layers = existing_layers(config);
        if let Some(session_runtime_layer) = session_runtime_layer(config)? {
            insert_layer(&mut layers, session_runtime_layer);
        }
        if let Some(resolved_profile_layer) =
            resolved_profile_layer(config, &layers, role_layer_toml, active_profile_name)?
        {
            insert_layer(&mut layers, resolved_profile_layer);
        }
        insert_layer(&mut layers, role_layer(role_layer_toml.clone()));
        Ok(ConfigLayerStack::new(
            layers,
            config.config_layer_stack.requirements().clone(),
            config.config_layer_stack.requirements_toml().clone(),
        )?)
    }

    fn resolved_profile_layer(
        config: &Config,
        existing_layers: &[ConfigLayerEntry],
        role_layer_toml: &TomlValue,
        active_profile_name: Option<&str>,
    ) -> anyhow::Result<Option<ConfigLayerEntry>> {
        let Some(active_profile_name) = active_profile_name else {
            return Ok(None);
        };

        let mut layers = existing_layers.to_vec();
        insert_layer(&mut layers, role_layer(role_layer_toml.clone()));
        let merged_config = deserialize_effective_config(
            config,
            &ConfigLayerStack::new(
                layers,
                config.config_layer_stack.requirements().clone(),
                config.config_layer_stack.requirements_toml().clone(),
            )?,
        )?;
        let resolved_profile =
            merged_config.get_config_profile(Some(active_profile_name.to_string()))?;
        Ok(Some(ConfigLayerEntry::new(
            ConfigLayerSource::SessionFlags,
            TomlValue::try_from(resolved_profile)?,
        )))
    }

    fn session_runtime_layer(config: &Config) -> anyhow::Result<Option<ConfigLayerEntry>> {
        let mut table = toml::map::Map::new();
        if let Some(model) = config.model.as_ref() {
            table.insert("model".to_string(), TomlValue::String(model.clone()));
        }
        insert_serialized_option(
            &mut table,
            "model_reasoning_effort",
            config.model_reasoning_effort,
        )?;
        insert_serialized_option(
            &mut table,
            "model_reasoning_summary",
            config.model_reasoning_summary,
        )?;
        insert_serialized_option(&mut table, "service_tier", config.service_tier.as_ref())?;
        insert_serialized_option(&mut table, "personality", config.personality)?;
        insert_serialized(
            &mut table,
            "approval_policy",
            config.permissions.approval_policy.value(),
        )?;
        insert_serialized(&mut table, "approvals_reviewer", config.approvals_reviewer)?;
        insert_serialized_option(
            &mut table,
            "developer_instructions",
            config.developer_instructions.as_ref(),
        )?;
        insert_serialized_option(&mut table, "compact_prompt", config.compact_prompt.as_ref())?;

        Ok((!table.is_empty()).then(|| {
            ConfigLayerEntry::new(ConfigLayerSource::SessionFlags, TomlValue::Table(table))
        }))
    }

    fn insert_serialized<T: Serialize>(
        table: &mut toml::map::Map<String, TomlValue>,
        key: &str,
        value: T,
    ) -> anyhow::Result<()> {
        table.insert(key.to_string(), TomlValue::try_from(value)?);
        Ok(())
    }

    fn insert_serialized_option<T: Serialize>(
        table: &mut toml::map::Map<String, TomlValue>,
        key: &str,
        value: Option<T>,
    ) -> anyhow::Result<()> {
        if let Some(value) = value {
            insert_serialized(table, key, value)?;
        }
        Ok(())
    }

    fn deserialize_effective_config(
        config: &Config,
        config_layer_stack: &ConfigLayerStack,
    ) -> anyhow::Result<ConfigToml> {
        Ok(deserialize_config_toml_with_base(
            config_layer_stack.effective_config(),
            &config.codex_home,
        )?)
    }

    fn existing_layers(config: &Config) -> Vec<ConfigLayerEntry> {
        config
            .config_layer_stack
            .get_layers(
                ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ true,
            )
            .into_iter()
            .cloned()
            .collect()
    }

    fn insert_layer(layers: &mut Vec<ConfigLayerEntry>, layer: ConfigLayerEntry) {
        let insertion_index =
            layers.partition_point(|existing_layer| existing_layer.name <= layer.name);
        layers.insert(insertion_index, layer);
    }

    fn role_layer(role_layer_toml: TomlValue) -> ConfigLayerEntry {
        ConfigLayerEntry::new(ConfigLayerSource::SessionFlags, role_layer_toml)
    }

    fn reload_overrides(config: &Config, preserve_current_provider: bool) -> ConfigOverrides {
        ConfigOverrides {
            cwd: Some(config.cwd.to_path_buf()),
            model_provider: preserve_current_provider.then(|| config.model_provider_id.clone()),
            codex_linux_sandbox_exe: config.codex_linux_sandbox_exe.clone(),
            main_execve_wrapper_exe: config.main_execve_wrapper_exe.clone(),
            ..Default::default()
        }
    }
}
