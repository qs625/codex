use super::*;

#[tokio::test]
async fn load_plugins_ignores_project_config_files() {
    let codex_home = TempDir::new().unwrap();
    let project_root = codex_home.path().join("project");
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    );
    write_file(
        &project_root.join(".codex/config.toml"),
        &plugin_config_toml(/*enabled*/ true, /*plugins_feature_enabled*/ true),
    );

    let stack = ConfigLayerStack::new(
        vec![ConfigLayerEntry::new(
            ConfigLayerSource::Project {
                dot_codex_folder: AbsolutePathBuf::try_from(project_root.join(".codex")).unwrap(),
            },
            toml::from_str(&plugin_config_toml(
                /*enabled*/ true, /*plugins_feature_enabled*/ true,
            ))
            .expect("project config should parse"),
        )],
        ConfigRequirements::default(),
        ConfigRequirementsToml::default(),
    )
    .expect("config layer stack should build");
    let stack = crate::test_support::plugin_config_layer_stack_from_config(&stack);

    let outcome = load_plugins_from_layer_stack(
        &stack,
        std::collections::HashMap::new(),
        &PluginStore::new(codex_home.path().to_path_buf()),
        Some(Product::Codex),
        /*plugin_hooks_enabled*/ false,
    )
    .await;

    assert_eq!(outcome, PluginLoadOutcome::default());
}
