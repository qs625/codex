use super::*;
use crate::config::AgentRoleConfig;
use crate::config::Config;
use crate::config::ConfigBuilder;
use crate::skills_load_input_from_config;
use plugin_service::PluginsManager;
use plugin_service_api::PluginRuntime;
use codex_core_skills::SkillsManager;
use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use toml::Value as TomlValue;

async fn test_config_with_cli_overrides(
    cli_overrides: Vec<(String, TomlValue)>,
) -> (TempDir, Config) {
    let home = TempDir::new().expect("create temp dir");
    let home_path = home.path().to_path_buf();
    let config = ConfigBuilder::default()
        .codex_home(home_path.clone())
        .cli_overrides(cli_overrides)
        .fallback_cwd(Some(home_path))
        .build()
        .await
        .expect("load test config");
    (home, config)
}

async fn write_role_config(home: &TempDir, name: &str, contents: &str) -> PathBuf {
    let role_path = home.path().join(name);
    tokio::fs::write(&role_path, contents)
        .await
        .expect("write role config");
    role_path
}

#[cfg_attr(windows, ignore)]
#[tokio::test]
async fn apply_role_skills_config_disables_skill_for_spawned_agent() {
    let (home, mut config) = test_config_with_cli_overrides(Vec::new()).await;
    let skill_dir = home.path().join("skills").join("demo");
    fs::create_dir_all(&skill_dir).expect("create skill dir");
    let skill_path = skill_dir.join("SKILL.md");
    fs::write(
        &skill_path,
        "---\nname: demo-skill\ndescription: demo description\n---\n\n# Body\n",
    )
    .expect("write skill");
    let role_path = write_role_config(
        &home,
        "skills-role.toml",
        &format!(
            r#"developer_instructions = "Stay focused"

[[skills.config]]
path = "{}"
enabled = false
"#,
            skill_path.display()
        ),
    )
    .await;
    config.agent_roles.insert(
        "custom".to_string(),
        AgentRoleConfig {
            description: None,
            config_file: Some(role_path),
            nickname_candidates: None,
            ..Default::default()
        },
    );

    apply_role_to_config(&mut config, Some("custom"))
        .await
        .expect("custom role should apply");

    let plugins_manager = Arc::new(PluginsManager::new(home.path().to_path_buf()));
    let skills_manager =
        SkillsManager::new(home.path().abs(), /*bundled_skills_enabled*/ true);
    let plugins_input = config.plugins_config_input();
    let effective_skill_roots = plugins_manager
        .effective_skill_roots_for_config(&plugins_input)
        .await;
    let skills_input = skills_load_input_from_config(&config, effective_skill_roots);
    let outcome = skills_manager
        .skills_for_config(
            &skills_input,
            Some(Arc::clone(&codex_file_system::LOCAL_FS)),
        )
        .await;
    let skill = outcome
        .skills
        .iter()
        .find(|skill| skill.name == "demo-skill")
        .expect("demo skill should be discovered");

    assert_eq!(outcome.is_skill_enabled(skill), false);
}
