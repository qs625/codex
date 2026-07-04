use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::OPENAI_CURATED_MARKETPLACE_NAME;
use crate::PluginsConfigInput;
use crate::PluginConfigLayerEntry;
use crate::PluginConfigLayerStack;
use config_service::CloudRequirementsLoader;
use config_service::ConfigLayerStack;
use config_service::ConfigLayerStackOrdering;
use config_service::LoaderOverrides;
use config_service::NoopThreadConfigLoader;
use config_service::load_config_layers_state;
use codex_file_system::LOCAL_FS;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::model_provider_auth_manager;
use codex_utils_absolute_path::AbsolutePathBuf;
use model_service::DefaultApiRuntimeFactory;
use model_service::DefaultModelProviderFactory;
use model_service::ModelService;
use model_service::ModelServiceRuntimeDeps;
use model_service_api::OPENAI_PROVIDER_ID;
use model_service_api::SharedModelServiceApi;
use model_service_api::built_in_model_providers;
use std::collections::HashSet;
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
    write_file(
        &plugin_root.join(".app.json"),
        r#"{
  "apps": {
    "calendar": {
      "id": "connector_calendar"
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

pub(crate) fn write_curated_plugin_sha_with(codex_home: &Path, sha: &str) {
    write_file(&codex_home.join(".tmp/plugins.sha"), &format!("{sha}\n"));
}

pub(crate) fn write_curated_plugin_sha(codex_home: &Path) {
    write_curated_plugin_sha_with(codex_home, TEST_CURATED_PLUGIN_SHA);
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

pub(crate) async fn load_plugins_config(codex_home: &Path, cwd: &Path) -> PluginsConfigInput {
    let codex_home = AbsolutePathBuf::try_from(codex_home).expect("codex home should be absolute");
    let cwd = AbsolutePathBuf::try_from(cwd).expect("cwd should be absolute");
    let config_layer_stack = load_config_layers_state(
        LOCAL_FS.as_ref(),
        codex_home.as_path(),
        Some(cwd),
        &[],
        LoaderOverrides::without_managed_config_for_tests(),
        CloudRequirementsLoader::default(),
        &NoopThreadConfigLoader,
    )
    .await
    .expect("config should load");
    let effective_config = config_layer_stack.effective_config();
    PluginsConfigInput::new(
        plugin_config_layer_stack_from_config(&config_layer_stack),
        feature_enabled(&effective_config, "plugins", /*default_enabled*/ true),
        feature_enabled(
            &effective_config,
            "remote_plugin",
            /*default_enabled*/ false,
        ),
        feature_enabled(
            &effective_config,
            "plugin_hooks",
            /*default_enabled*/ false,
        ),
        "https://chatgpt.com/backend-api/".to_string(),
    )
}

pub(crate) fn build_test_model_service(
    codex_home: &Path,
    chatgpt_base_url: &str,
    auth: Option<CodexAuth>,
) -> SharedModelServiceApi {
    let auth_manager = auth.map(AuthManager::from_auth_for_testing);
    let provider_auth_manager = model_provider_auth_manager(auth_manager);
    let providers_by_id = built_in_model_providers(Some(chatgpt_base_url.to_string()));
    let default_provider = providers_by_id.get(OPENAI_PROVIDER_ID).cloned();
    Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
        codex_home: codex_home.to_path_buf(),
        config_model_catalog: None,
        api_runtime_factory: Arc::new(DefaultApiRuntimeFactory),
        provider_auth_manager,
        model_provider_factory: Arc::new(DefaultModelProviderFactory),
        default_provider,
        providers_by_id,
        model_metadata_overrides: Vec::new(),
        attestation_provider: None,
    }))
}

pub(crate) fn plugin_config_layer_stack_from_config(
    config_layer_stack: &ConfigLayerStack,
) -> PluginConfigLayerStack {
    PluginConfigLayerStack::new(
        config_layer_stack
            .get_layers(
                ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ true,
            )
            .into_iter()
            .map(|layer| {
                PluginConfigLayerEntry::new_with_config_folder(
                    layer.name.clone(),
                    layer.config.clone(),
                    layer.config_folder(),
                    layer.is_disabled(),
                )
            })
            .collect(),
    )
}

pub(crate) async fn load_tool_suggest_plugin_ids(
    codex_home: &Path,
    cwd: &Path,
) -> (HashSet<String>, HashSet<String>) {
    let codex_home = AbsolutePathBuf::try_from(codex_home).expect("codex home should be absolute");
    let cwd = AbsolutePathBuf::try_from(cwd).expect("cwd should be absolute");
    let config_layer_stack = load_config_layers_state(
        LOCAL_FS.as_ref(),
        codex_home.as_path(),
        Some(cwd),
        &[],
        LoaderOverrides::without_managed_config_for_tests(),
        CloudRequirementsLoader::default(),
        &NoopThreadConfigLoader,
    )
    .await
    .expect("config should load");
    let effective_config = config_layer_stack.effective_config();

    let configured_plugin_ids = effective_config
        .get("tool_suggest")
        .and_then(Value::as_table)
        .and_then(|tool_suggest| tool_suggest.get("discoverables"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.as_table()?;
            if entry.get("type").and_then(Value::as_str) != Some("plugin") {
                return None;
            }
            entry.get("id").and_then(Value::as_str).map(str::to_owned)
        })
        .collect();
    let disabled_plugin_ids = effective_config
        .get("tool_suggest")
        .and_then(Value::as_table)
        .and_then(|tool_suggest| tool_suggest.get("disabled_tools"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.as_table()?;
            if entry.get("type").and_then(Value::as_str) != Some("plugin") {
                return None;
            }
            entry.get("id").and_then(Value::as_str).map(str::to_owned)
        })
        .collect();

    (configured_plugin_ids, disabled_plugin_ids)
}

fn feature_enabled(config: &Value, key: &str, default_enabled: bool) -> bool {
    config
        .get("features")
        .and_then(Value::as_table)
        .and_then(|features| features.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(default_enabled)
}
