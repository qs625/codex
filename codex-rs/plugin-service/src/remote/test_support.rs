use std::fs;
use std::path::Path;

use codex_config_edit::CONFIG_TOML_FILE;
use codex_config_types::ConfigLayerSource;
use crate::OPENAI_CURATED_MARKETPLACE_NAME;
use crate::PluginConfigLayerEntry;
use crate::PluginConfigLayerStack;
use crate::PluginsConfigInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use toml::Value;

pub(crate) const TEST_CURATED_PLUGIN_SHA: &str = "0123456789abcdef0123456789abcdef01234567";
pub(crate) const TEST_CURATED_PLUGIN_CACHE_VERSION: &str = "01234567";

pub(crate) fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("file should have a parent")).unwrap();
    fs::write(path, contents).unwrap();
}

pub(crate) fn write_curated_plugin(root: &Path, plugin_name: &str) {
    let plugin_root = root.join("plugins").join(plugin_name);
    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        &format!(
            r#"{{
  "name": "{plugin_name}",
  "description": "Plugin that includes skills, MCP servers, and app connectors"
}}"#
        ),
    );
    write_file(
        &plugin_root.join("skills/SKILL.md"),
        "---\nname: sample\ndescription: sample\n---\n",
    );
    write_file(
        &plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "sample-docs": {
      "type": "http",
      "url": "https://sample.example/mcp"
    }
  }
}"#,
    );
}

pub(crate) fn write_openai_curated_marketplace(root: &Path, plugin_names: &[&str]) {
    let plugins = plugin_names
        .iter()
        .map(|plugin_name| {
            format!(
                r#"{{
      "name": "{plugin_name}",
      "source": {{
        "source": "local",
        "path": "./plugins/{plugin_name}"
      }}
    }}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    write_file(
        &root.join(".agents/plugins/marketplace.json"),
        &format!(
            r#"{{
  "name": "{OPENAI_CURATED_MARKETPLACE_NAME}",
  "plugins": [
{plugins}
  ]
}}"#
        ),
    );
    for plugin_name in plugin_names {
        write_curated_plugin(root, plugin_name);
    }
}

pub(crate) fn write_curated_plugin_sha(codex_home: &Path) {
    write_curated_plugin_sha_with(codex_home, TEST_CURATED_PLUGIN_SHA);
}

pub(crate) fn write_curated_plugin_sha_with(codex_home: &Path, sha: &str) {
    write_file(&codex_home.join(".tmp/plugins.sha"), &format!("{sha}\n"));
}

pub(crate) fn write_plugin_with_version(
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

pub(crate) fn write_plugin(root: &Path, dir_name: &str, manifest_name: &str) {
    write_plugin_with_version(
        root,
        dir_name,
        manifest_name,
        /*manifest_version*/ None,
    );
}

pub(crate) fn load_plugins_config(codex_home: &Path) -> PluginsConfigInput {
    let config_path = codex_home.join(CONFIG_TOML_FILE);
    let config = match fs::read_to_string(&config_path) {
        Ok(raw) => toml::from_str::<Value>(&raw).expect("test config should parse as TOML"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Value::Table(toml::map::Map::new())
        }
        Err(err) => panic!("failed to read test config: {err}"),
    };
    let plugins_enabled = feature_enabled(&config, "plugins", /*default_enabled*/ true);
    let remote_plugin_enabled =
        feature_enabled(&config, "remote_plugin", /*default_enabled*/ false);
    let plugin_hooks_enabled =
        feature_enabled(&config, "plugin_hooks", /*default_enabled*/ false);
    let config_layer_stack = PluginConfigLayerStack::new(vec![PluginConfigLayerEntry::new(
        ConfigLayerSource::User {
            file: AbsolutePathBuf::try_from(config_path).expect("config path should be absolute"),
            profile: None,
        },
        config,
    )]);
    PluginsConfigInput::new(
        config_layer_stack,
        plugins_enabled,
        remote_plugin_enabled,
        plugin_hooks_enabled,
        "https://chatgpt.com/backend-api/".to_string(),
    )
}

fn feature_enabled(config: &Value, key: &str, default_enabled: bool) -> bool {
    config
        .get("features")
        .and_then(Value::as_table)
        .and_then(|features| features.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(default_enabled)
}
