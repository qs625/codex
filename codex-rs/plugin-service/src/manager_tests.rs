use super::*;
use crate::LoadedPlugin;
use crate::PluginLoadOutcome;
use crate::installed_marketplaces::marketplace_install_root;
use crate::loader::configured_curated_plugin_ids_from_codex_home;
use crate::loader::load_plugins_from_layer_stack;
use crate::loader::refresh_curated_plugin_cache;
use crate::loader::refresh_non_curated_plugin_cache;
use crate::loader::refresh_non_curated_plugin_cache_force_reinstall;
use crate::marketplace::MarketplacePluginInstallPolicy;
use crate::remote_state::RemoteInstalledPlugin;
use crate::startup_sync::curated_plugins_repo_path;
use crate::test_support::TEST_CURATED_PLUGIN_CACHE_VERSION;
use crate::test_support::TEST_CURATED_PLUGIN_SHA;
use crate::test_support::load_plugins_config as load_plugins_config_input;
use crate::test_support::write_curated_plugin_sha_with as write_curated_plugin_sha;
use crate::test_support::write_file;
use crate::test_support::write_openai_curated_marketplace;
use config_service::AppToolApproval;
use config_service::CONFIG_TOML_FILE;
use config_service::ConfigLayerEntry;
use config_service::ConfigLayerStack;
use config_service::ConfigRequirements;
use config_service::ConfigRequirementsToml;
use config_service::McpServerConfig;
use config_service::McpServerOAuthConfig;
use config_service::McpServerToolConfig;
use config_service::types::McpServerTransportConfig;
use codex_config_types::ConfigLayerSource;
use codex_utils_absolute_path::test_support::PathBufExt;
use pretty_assertions::assert_eq;
use protocol::protocol::HookEventName;
use protocol::protocol::Product;
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use toml::Value;

const MAX_CAPABILITY_SUMMARY_DESCRIPTION_LEN: usize = 1024;

fn write_plugin_with_version(
    root: &Path,
    dir_name: &str,
    manifest_name: &str,
    manifest_version: Option<&str>,
) {
    let plugin_root = root.join(dir_name);
    fs::create_dir_all(plugin_root.join(".codex-plugin")).unwrap();
    fs::create_dir_all(plugin_root.join("skills")).unwrap();
    let version = manifest_version
        .map(|manifest_version| format!(r#","version":"{manifest_version}""#))
        .unwrap_or_default();
    fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        format!(r#"{{"name":"{manifest_name}"{version}}}"#),
    )
    .unwrap();
    fs::write(plugin_root.join("skills/SKILL.md"), "skill").unwrap();
    fs::write(plugin_root.join(".mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
}

fn write_plugin(root: &Path, dir_name: &str, manifest_name: &str) {
    write_plugin_with_version(
        root,
        dir_name,
        manifest_name,
        /*manifest_version*/ None,
    );
}

fn init_git_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "codex-test@example.com"]);
    run_git(repo, &["config", "user.name", "Codex Test"]);
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("git should run: {err}"));
    assert!(
        output.status.success(),
        "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
        repo.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn plugin_config_toml(enabled: bool, plugins_feature_enabled: bool) -> String {
    plugin_config_toml_with_plugin_hooks(
        enabled,
        plugins_feature_enabled,
        /*plugin_hooks_feature_enabled*/ false,
    )
}

fn plugin_config_toml_with_plugin_hooks(
    enabled: bool,
    plugins_feature_enabled: bool,
    plugin_hooks_feature_enabled: bool,
) -> String {
    let mut root = toml::map::Map::new();

    let mut features = toml::map::Map::new();
    features.insert(
        "plugins".to_string(),
        Value::Boolean(plugins_feature_enabled),
    );
    features.insert(
        "plugin_hooks".to_string(),
        Value::Boolean(plugin_hooks_feature_enabled),
    );
    root.insert("features".to_string(), Value::Table(features));

    let mut plugin = toml::map::Map::new();
    plugin.insert("enabled".to_string(), Value::Boolean(enabled));

    let mut plugins = toml::map::Map::new();
    plugins.insert("sample@test".to_string(), Value::Table(plugin));
    root.insert("plugins".to_string(), Value::Table(plugins));

    toml::to_string(&Value::Table(root)).expect("plugin test config should serialize")
}

async fn load_plugins_from_config(config_toml: &str, codex_home: &Path) -> PluginLoadOutcome {
    write_file(&codex_home.join(CONFIG_TOML_FILE), config_toml);
    let config = load_config(codex_home, codex_home).await;
    PluginsManager::new(codex_home.to_path_buf())
        .plugins_for_config(&config)
        .await
}

async fn load_config(codex_home: &Path, cwd: &Path) -> PluginsConfigInput {
    load_plugins_config_input(codex_home, cwd).await
}


mod cache_refresh;
mod install_and_read;
mod load_and_summary;
mod marketplaces;
mod project_scope;
