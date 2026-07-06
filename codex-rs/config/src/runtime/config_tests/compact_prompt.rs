use super::*;

#[tokio::test]
async fn compact_prompt_override_beats_default_compact_prompt_locations() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".codex").join("compact"))?;
    std::fs::create_dir_all(codex_home.path().join("compact"))?;

    std::fs::write(
        workspace.join(".codex").join("compact").join("COMPACT.md"),
        "  workspace compact prompt  ",
    )?;
    std::fs::write(
        codex_home.path().join("compact").join("COMPACT.md"),
        "  home compact prompt  ",
    )?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(workspace),
            compact_prompt: Some("Use the compact override".to_string()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.compact_prompt.as_deref(),
        Some("Use the compact override")
    );

    Ok(())
}

#[tokio::test]
async fn compact_prompt_override_skips_default_compact_prompt_reads() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".codex").join("compact").join("COMPACT.md"))?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(workspace),
            compact_prompt: Some("Use the compact override".to_string()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.compact_prompt.as_deref(),
        Some("Use the compact override")
    );

    Ok(())
}

#[tokio::test]
async fn loads_compact_prompt_from_file() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;

    let prompt_path = workspace.join("compact_prompt.txt");
    std::fs::write(&prompt_path, "  summarize differently  ")?;

    let cfg = ConfigToml {
        experimental_compact_prompt_file: Some(prompt_path.abs()),
        ..Default::default()
    };

    let overrides = ConfigOverrides {
        cwd: Some(workspace),
        ..Default::default()
    };

    let config =
        Config::load_from_base_config_with_overrides(cfg, overrides, codex_home.abs()).await?;

    assert_eq!(
        config.compact_prompt.as_deref(),
        Some("summarize differently")
    );

    Ok(())
}

#[tokio::test]
async fn loads_default_compact_prompt_from_workspace_before_codex_home() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".codex").join("compact"))?;
    std::fs::create_dir_all(codex_home.path().join("compact"))?;

    std::fs::write(
        workspace.join(".codex").join("compact").join("COMPACT.md"),
        "  workspace compact prompt  ",
    )?;
    std::fs::write(
        codex_home.path().join("compact").join("COMPACT.md"),
        "  home compact prompt  ",
    )?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(workspace),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.compact_prompt.as_deref(),
        Some("workspace compact prompt")
    );

    Ok(())
}

#[tokio::test]
async fn falls_back_to_codex_home_default_compact_prompt() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;
    std::fs::create_dir_all(codex_home.path().join("compact"))?;

    std::fs::write(
        codex_home.path().join("compact").join("COMPACT.md"),
        "  home compact prompt  ",
    )?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(workspace),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.compact_prompt.as_deref(), Some("home compact prompt"));

    Ok(())
}

#[tokio::test]
async fn empty_workspace_default_compact_prompt_falls_back_to_codex_home() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".codex").join("compact"))?;
    std::fs::create_dir_all(codex_home.path().join("compact"))?;

    std::fs::write(workspace.join(".codex").join("compact").join("COMPACT.md"), "   ")?;
    std::fs::write(
        codex_home.path().join("compact").join("COMPACT.md"),
        "  home compact prompt  ",
    )?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(workspace),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.compact_prompt.as_deref(), Some("home compact prompt"));

    Ok(())
}

#[tokio::test]
async fn empty_codex_home_default_compact_prompt_is_treated_as_missing() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;
    std::fs::create_dir_all(codex_home.path().join("compact"))?;

    std::fs::write(codex_home.path().join("compact").join("COMPACT.md"), "   ")?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(workspace),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.compact_prompt, None);

    Ok(())
}

#[tokio::test]
async fn explicit_compact_prompt_file_beats_default_compact_prompt_locations(
) -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".codex").join("compact"))?;
    std::fs::create_dir_all(codex_home.path().join("compact"))?;

    let explicit_prompt_path = workspace.join("compact_prompt.txt");
    std::fs::write(&explicit_prompt_path, "  explicit compact prompt  ")?;
    std::fs::write(
        workspace.join(".codex").join("compact").join("COMPACT.md"),
        "  workspace compact prompt  ",
    )?;
    std::fs::write(
        codex_home.path().join("compact").join("COMPACT.md"),
        "  home compact prompt  ",
    )?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            experimental_compact_prompt_file: Some(explicit_prompt_path.abs()),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(workspace),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.compact_prompt.as_deref(),
        Some("explicit compact prompt")
    );

    Ok(())
}

#[tokio::test]
async fn explicit_compact_prompt_file_error_does_not_fall_back_to_default_locations(
) -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".codex").join("compact"))?;
    std::fs::create_dir_all(codex_home.path().join("compact"))?;

    let explicit_prompt_path = workspace.join("compact_prompt.txt");
    std::fs::write(&explicit_prompt_path, "   ")?;
    std::fs::write(
        workspace.join(".codex").join("compact").join("COMPACT.md"),
        "  workspace compact prompt  ",
    )?;
    std::fs::write(
        codex_home.path().join("compact").join("COMPACT.md"),
        "  home compact prompt  ",
    )?;

    let err = Config::load_from_base_config_with_overrides(
        ConfigToml {
            experimental_compact_prompt_file: Some(explicit_prompt_path.abs()),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(workspace),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await
    .expect_err("empty explicit compact prompt file should fail");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        err.to_string()
            .contains("experimental compact prompt file is empty"),
        "unexpected error: {err}"
    );

    Ok(())
}
