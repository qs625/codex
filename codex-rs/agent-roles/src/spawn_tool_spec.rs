use crate::AgentRoleConfig;
use crate::DEFAULT_ROLE_NAME;
use crate::built_in_config_file_contents;
use crate::built_in_configs;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use toml::Value as TomlValue;

const MAX_SPAWN_TOOL_AGENT_ROLES: usize = 32;
const MAX_SPAWN_TOOL_DESCRIPTION_CHARS: usize = 1024;

/// Builds the spawn-agent tool description text from built-in and configured roles.
pub fn build(user_defined_agent_roles: &BTreeMap<String, AgentRoleConfig>) -> String {
    build_from_configs(built_in_configs(), user_defined_agent_roles)
}

// This function is not inlined for testing purpose.
fn build_from_configs(
    built_in_roles: &BTreeMap<String, AgentRoleConfig>,
    user_defined_roles: &BTreeMap<String, AgentRoleConfig>,
) -> String {
    let total_unique_roles = user_defined_roles
        .keys()
        .chain(built_in_roles.keys())
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .len();
    let mut seen = BTreeSet::new();
    let mut formatted_roles = Vec::new();
    for (name, declaration) in user_defined_roles {
        if formatted_roles.len() >= MAX_SPAWN_TOOL_AGENT_ROLES {
            break;
        }
        if seen.insert(name.as_str()) {
            formatted_roles.push(format_role(name, declaration));
        }
    }
    for (name, declaration) in built_in_roles {
        if formatted_roles.len() >= MAX_SPAWN_TOOL_AGENT_ROLES {
            break;
        }
        if seen.insert(name.as_str()) {
            formatted_roles.push(format_role(name, declaration));
        }
    }
    let omitted_roles = total_unique_roles.saturating_sub(formatted_roles.len());
    if omitted_roles > 0 {
        formatted_roles.push(format!("{omitted_roles} additional roles omitted."));
    }

    format!(
        "Optional type name for the new agent. If omitted, `{DEFAULT_ROLE_NAME}` is used.\nAvailable roles:\n{}",
        formatted_roles.join("\n"),
    )
}

fn format_role(name: &str, declaration: &AgentRoleConfig) -> String {
    if let Some(description) = &declaration.description {
        let description = description
            .chars()
            .take(MAX_SPAWN_TOOL_DESCRIPTION_CHARS)
            .collect::<String>();
        let (model, reasoning_effort) = role_locked_settings(declaration);
        let locked_settings_note =
            locked_settings_note(model.as_deref(), reasoning_effort.as_deref());
        format!("{name}: {{\n{description}{locked_settings_note}\n}}")
    } else {
        format!("{name}: no description")
    }
}

fn role_locked_settings(declaration: &AgentRoleConfig) -> (Option<String>, Option<String>) {
    let mut model = declaration.model.clone();
    let mut reasoning_effort = declaration.model_reasoning_effort.clone();
    if model.is_some() && reasoning_effort.is_some() {
        return (model, reasoning_effort);
    }

    if let Some(role_toml) = declaration
        .config_file
        .as_ref()
        .and_then(|config_file| {
            built_in_config_file_contents(config_file)
                .map(str::to_owned)
                .or_else(|| std::fs::read_to_string(config_file).ok())
        })
        .and_then(|contents| toml::from_str::<TomlValue>(&contents).ok())
    {
        let role_model = role_toml
            .get("model")
            .and_then(TomlValue::as_str)
            .map(ToOwned::to_owned);
        let role_reasoning_effort = role_toml
            .get("model_reasoning_effort")
            .and_then(TomlValue::as_str)
            .map(ToOwned::to_owned);
        model = model.or(role_model);
        reasoning_effort = reasoning_effort.or(role_reasoning_effort);
    }

    (model, reasoning_effort)
}

fn locked_settings_note(model: Option<&str>, reasoning_effort: Option<&str>) -> String {
    match (model, reasoning_effort) {
        (Some(model), Some(reasoning_effort)) => format!(
            "\n- This role's model is set to `{model}` and its reasoning effort is set to `{reasoning_effort}`. These settings cannot be changed."
        ),
        (Some(model), None) => {
            format!("\n- This role's model is set to `{model}` and cannot be changed.")
        }
        (None, Some(reasoning_effort)) => {
            format!(
                "\n- This role's reasoning effort is set to `{reasoning_effort}` and cannot be changed."
            )
        }
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentRoleConfig;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn build_deduplicates_user_defined_built_in_roles() {
        let user_defined_roles = BTreeMap::from([
            (
                "explorer".to_string(),
                AgentRoleConfig {
                    description: Some("user override".to_string()),
                    config_file: None,
                    nickname_candidates: None,
                    ..Default::default()
                },
            ),
            ("researcher".to_string(), AgentRoleConfig::default()),
        ]);

        let spec = build(&user_defined_roles);

        assert!(spec.contains("researcher: no description"));
        assert!(spec.contains("explorer: {\nuser override\n}"));
        assert!(spec.contains("default: {\nDefault agent.\n}"));
        assert!(!spec.contains("Explorers are fast and authoritative."));
    }

    #[test]
    fn lists_user_defined_roles_before_built_ins() {
        let user_defined_roles = BTreeMap::from([(
            "aaa".to_string(),
            AgentRoleConfig {
                description: Some("first".to_string()),
                config_file: None,
                nickname_candidates: None,
                ..Default::default()
            },
        )]);

        let spec = build(&user_defined_roles);
        let user_index = spec.find("aaa: {\nfirst\n}").expect("find user role");
        let built_in_index = spec
            .find("default: {\nDefault agent.\n}")
            .expect("find built-in role");

        assert!(user_index < built_in_index);
    }

    #[test]
    fn caps_user_defined_role_guidance() {
        let long_description = "x".repeat(1200);
        let user_defined_roles = (0..40)
            .map(|index| {
                (
                    format!("role-{index:02}"),
                    AgentRoleConfig {
                        description: Some(long_description.clone()),
                        ..Default::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        let spec = build(&user_defined_roles);

        assert!(spec.contains("11 additional roles omitted."));
        assert!(spec.contains(&"x".repeat(1024)));
        assert!(!spec.contains(&"x".repeat(1025)));
    }

    #[test]
    fn marks_role_locked_model_and_reasoning_effort() {
        let tempdir = TempDir::new().expect("create temp dir");
        let role_path = tempdir.path().join("researcher.toml");
        fs::write(
            &role_path,
            "developer_instructions = \"Research carefully\"\nmodel = \"gpt-5\"\nmodel_reasoning_effort = \"high\"\n",
        )
        .expect("write role config");
        let user_defined_roles = BTreeMap::from([(
            "researcher".to_string(),
            AgentRoleConfig {
                description: Some("Research carefully.".to_string()),
                config_file: Some(role_path),
                nickname_candidates: None,
                ..Default::default()
            },
        )]);

        let spec = build(&user_defined_roles);

        assert!(spec.contains(
            "Research carefully.\n- This role's model is set to `gpt-5` and its reasoning effort is set to `high`. These settings cannot be changed."
        ));
    }

    #[test]
    fn marks_role_locked_reasoning_effort_only() {
        let tempdir = TempDir::new().expect("create temp dir");
        let role_path = tempdir.path().join("reviewer.toml");
        fs::write(
            &role_path,
            "developer_instructions = \"Review carefully\"\nmodel_reasoning_effort = \"medium\"\n",
        )
        .expect("write role config");
        let user_defined_roles = BTreeMap::from([(
            "reviewer".to_string(),
            AgentRoleConfig {
                description: Some("Review carefully.".to_string()),
                config_file: Some(role_path),
                nickname_candidates: None,
                ..Default::default()
            },
        )]);

        let spec = build(&user_defined_roles);

        assert!(spec.contains(
            "Review carefully.\n- This role's reasoning effort is set to `medium` and cannot be changed."
        ));
    }
}
