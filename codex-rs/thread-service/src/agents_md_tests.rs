use super::*;
use crate::config::ConfigBuilder;
use codex_features::Feature;
use codex_file_system::LOCAL_FS;
use codex_utils_absolute_path::AbsolutePathBuf;
use core_test_support::TempDirExt;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;

async fn get_user_instructions(config: &Config) -> Option<String> {
    AgentsMdManager::new(config)
        .user_instructions_with_fs(LOCAL_FS.as_ref())
        .await
}

async fn make_config(root: &TempDir, limit: usize, instructions: Option<&str>) -> Config {
    let codex_home = TempDir::new().unwrap();
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("defaults for test should always succeed");

    config.cwd = root.abs();
    config.project_doc_max_bytes = limit;

    config.user_instructions = instructions.map(ToOwned::to_owned);
    config
}

async fn make_config_with_instruction_files(
    root: &TempDir,
    limit: usize,
    instructions: Option<&str>,
    instruction_files: Vec<AbsolutePathBuf>,
) -> Config {
    let mut config = make_config(root, limit, instructions).await;
    config.instruction_files = instruction_files;
    config
}

#[tokio::test]
async fn no_instruction_files_returns_none() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let res =
        get_user_instructions(&make_config(&tmp, /*limit*/ 4096, /*instructions*/ None).await)
            .await;
    assert!(res.is_none());
}

#[tokio::test]
async fn no_environment_returns_none() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = make_config(&tmp, /*limit*/ 4096, Some("user instructions")).await;

    let res = AgentsMdManager::new(&config)
        .user_instructions(/*environment*/ None)
        .await;

    assert_eq!(res, None);
}

#[tokio::test]
async fn explicit_instruction_files_are_loaded_in_order() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let instructions_dir = tmp.path().join("instructions");
    fs::create_dir_all(&instructions_dir).unwrap();
    let project_instruction = instructions_dir.join("project.md");
    let user_instruction = instructions_dir.join("user.md");
    fs::write(&project_instruction, "project understanding").unwrap();
    fs::write(&user_instruction, "user preferences").unwrap();

    let config = make_config_with_instruction_files(
        &tmp,
        4096,
        None,
        vec![
            AbsolutePathBuf::try_from(project_instruction).expect("absolute path"),
            AbsolutePathBuf::try_from(user_instruction).expect("absolute path"),
        ],
    )
    .await;

    let res = get_user_instructions(&config).await.expect("doc expected");
    assert_eq!(res, "project understanding\n\nuser preferences");
}

#[tokio::test]
async fn instruction_file_smaller_than_limit_is_returned() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let instruction_path = tmp.path().join("instruction.md");
    fs::write(&instruction_path, "hello world").unwrap();

    let config = make_config_with_instruction_files(
        &tmp,
        4096,
        None,
        vec![AbsolutePathBuf::try_from(instruction_path).expect("absolute path")],
    )
    .await;
    let res = get_user_instructions(&config).await.expect("doc expected");

    assert_eq!(res, "hello world");
}

#[tokio::test]
async fn instruction_file_larger_than_limit_is_truncated() {
    const LIMIT: usize = 1024;
    let tmp = tempfile::tempdir().expect("tempdir");

    let huge = "A".repeat(LIMIT * 2); // 2 KiB
    let instruction_path = tmp.path().join("instruction.md");
    fs::write(&instruction_path, &huge).unwrap();

    let config = make_config_with_instruction_files(
        &tmp,
        LIMIT,
        None,
        vec![AbsolutePathBuf::try_from(instruction_path).expect("absolute path")],
    )
    .await;
    let res = get_user_instructions(&config).await.expect("doc expected");

    assert_eq!(res.len(), LIMIT);
    assert_eq!(res, huge[..LIMIT]);
}

#[tokio::test]
async fn zero_byte_limit_disables_instruction_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let instruction_path = tmp.path().join("instruction.md");
    fs::write(&instruction_path, "something").unwrap();

    let config = make_config_with_instruction_files(
        &tmp,
        0,
        None,
        vec![AbsolutePathBuf::try_from(instruction_path).expect("absolute path")],
    )
    .await;

    let res = get_user_instructions(&config).await;
    assert!(res.is_none());
}

#[tokio::test]
async fn merges_existing_instructions_with_instruction_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let instruction_path = tmp.path().join("project.md");
    fs::write(&instruction_path, "proj doc").unwrap();

    const INSTRUCTIONS: &str = "base instructions";

    let config = make_config_with_instruction_files(
        &tmp,
        4096,
        Some(INSTRUCTIONS),
        vec![AbsolutePathBuf::try_from(instruction_path).expect("absolute path")],
    )
    .await;
    let res = get_user_instructions(&config)
        .await
        .expect("should produce a combined instruction string");

    let expected = format!("{INSTRUCTIONS}{AGENTS_MD_SEPARATOR}{}", "proj doc");

    assert_eq!(res, expected);
}

#[tokio::test]
async fn keeps_existing_instructions_when_instruction_files_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");

    const INSTRUCTIONS: &str = "some instructions";

    let res =
        get_user_instructions(&make_config(&tmp, /*limit*/ 4096, Some(INSTRUCTIONS)).await).await;

    assert_eq!(res, Some(INSTRUCTIONS.to_string()));
}

#[tokio::test]
async fn instruction_sources_match_configured_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_instruction = tmp.path().join("project.md");
    let user_instruction = tmp.path().join("user.md");
    fs::write(&project_instruction, "project doc").unwrap();
    fs::write(&user_instruction, "user doc").unwrap();

    let cfg = make_config_with_instruction_files(
        &tmp,
        4096,
        None,
        vec![
            AbsolutePathBuf::try_from(project_instruction.clone()).expect("absolute path"),
            AbsolutePathBuf::try_from(user_instruction.clone()).expect("absolute path"),
        ],
    )
    .await;

    let sources = AgentsMdManager::new(&cfg)
        .instruction_sources(LOCAL_FS.as_ref())
        .await;
    assert_eq!(
        sources,
        vec![
            AbsolutePathBuf::try_from(project_instruction).expect("absolute path"),
            AbsolutePathBuf::try_from(user_instruction).expect("absolute path"),
        ]
    );
}

#[tokio::test]
async fn missing_instruction_files_are_skipped() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let present_instruction = tmp.path().join("present.md");
    let missing_instruction = tmp.path().join("missing.md");
    fs::write(&present_instruction, "example instructions").unwrap();

    let cfg = make_config_with_instruction_files(
        &tmp,
        4096,
        None,
        vec![
            AbsolutePathBuf::try_from(missing_instruction).expect("absolute path"),
            AbsolutePathBuf::try_from(present_instruction).expect("absolute path"),
        ],
    )
    .await;

    let res = get_user_instructions(&cfg)
        .await
        .expect("instruction doc expected");

    assert_eq!(res, "example instructions");
}

#[tokio::test]
async fn empty_instruction_files_do_not_produce_output() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let empty_instruction = tmp.path().join("empty.md");
    fs::write(&empty_instruction, " \n\t").unwrap();

    let cfg = make_config_with_instruction_files(
        &tmp,
        4096,
        None,
        vec![AbsolutePathBuf::try_from(empty_instruction).expect("absolute path")],
    )
    .await;

    let res = get_user_instructions(&cfg).await;
    assert_eq!(res, None);
}

#[tokio::test]
async fn child_agents_md_feature_appends_hierarchical_message() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let instruction_path = tmp.path().join("project.md");
    fs::write(&instruction_path, "base doc").unwrap();

    let mut cfg = make_config_with_instruction_files(
        &tmp,
        4096,
        None,
        vec![AbsolutePathBuf::try_from(instruction_path).expect("absolute path")],
    )
    .await;
    cfg.features
        .enable(Feature::ChildAgentsMd)
        .expect("test config should allow child agents md");

    let res = get_user_instructions(&cfg)
        .await
        .expect("instructions expected");
    assert_eq!(res, format!("base doc\n\n{HIERARCHICAL_AGENTS_MESSAGE}"));
}

#[tokio::test]
async fn apps_feature_does_not_emit_instruction_files_by_itself() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let mut cfg = make_config(&tmp, /*limit*/ 4096, /*instructions*/ None).await;
    cfg.features
        .enable(Feature::Apps)
        .expect("test config should allow apps");

    let res = get_user_instructions(&cfg).await;
    assert_eq!(res, None);
}
