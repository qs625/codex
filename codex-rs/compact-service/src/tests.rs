use super::*;
use compact_service_api::CompactCurrentWork;
use compact_service_api::CompactFileNote;
use compact_service_api::CompactModelOutput;
use compact_service_api::ReplacementHistoryInput;
use compact_service_api::SoftCompactInputs;
use pretty_assertions::assert_eq;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use tempfile::TempDir;

fn sample_output() -> CompactModelOutput {
    CompactModelOutput {
        current_work: CompactCurrentWork {
            goal: "切换 compact 主流程到 memory 中心".to_string(),
            status: "in_progress".to_string(),
            recent_progress: vec!["已确认 compact 主入口".to_string()],
            files_read: vec![CompactFileNote {
                path: "codex-rs/thread-service/src/compact.rs".to_string(),
                reason: "确认主流程".to_string(),
                conclusion: "当前仍依赖 summary".to_string(),
                revisit: Some("需要".to_string()),
            }],
            key_findings: vec!["shared root 可缺省为空".to_string()],
            skip_files: vec!["apps/root-worker-prototype".to_string()],
            blockers: vec!["暂无".to_string()],
            next_steps: vec!["抽 compact-service".to_string()],
        },
        shared_fact_candidates: vec!["project-understanding 只允许 PM canonical 写入".to_string()],
        handoff_summary: "已更新 local current-work 并生成 memory checkpoint".to_string(),
    }
}

fn user_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

#[tokio::test]
async fn writes_local_current_work_without_shared_memory() {
    let tempdir = TempDir::new().expect("create temp dir");
    let cwd = AbsolutePathBuf::try_from(tempdir.path().to_path_buf()).expect("abs cwd");
    let codex_home = AbsolutePathBuf::try_from(tempdir.path().join("home")).expect("abs home");
    let service = FsCompactService::new();
    let layout = service
        .derive_memory_layout(&cwd, &codex_home, Some("compact prompt"))
        .await
        .expect("derive layout");
    assert_eq!(layout.shared_memory_root, None);

    let bundle = service
        .read_memory_bundle(&layout)
        .await
        .expect("read bundle");
    let applied = service
        .apply_model_output(&layout, &bundle, &sample_output())
        .await
        .expect("write current-work");

    assert!(applied.current_work_markdown.contains("## Current Goal"));
    assert!(applied.current_work_markdown.contains("切换 compact 主流程到 memory 中心"));
}

#[tokio::test]
async fn derives_shared_memory_root_from_matching_workspace_prompt() {
    let tempdir = TempDir::new().expect("create temp dir");
    let cwd = AbsolutePathBuf::try_from(tempdir.path().to_path_buf()).expect("abs cwd");
    let codex_home = AbsolutePathBuf::try_from(tempdir.path().join("home")).expect("abs home");
    tokio::fs::create_dir_all(cwd.join(".codex").join("compact").as_path())
        .await
        .expect("create compact dir");
    tokio::fs::write(
        cwd.join(".codex").join("compact").join("COMPACT.md").as_path(),
        "workspace compact prompt",
    )
    .await
    .expect("write compact prompt");

    let service = FsCompactService::new();
    let layout = service
        .derive_memory_layout(&cwd, &codex_home, Some("workspace compact prompt"))
        .await
        .expect("derive layout");

    assert_eq!(
        layout.shared_memory_root,
        Some(cwd.join(".codex").join("memory"))
    );
}

#[tokio::test]
async fn read_memory_bundle_truncates_oversized_memory_files() {
    let tempdir = TempDir::new().expect("create temp dir");
    let cwd = AbsolutePathBuf::try_from(tempdir.path().to_path_buf()).expect("abs cwd");
    let codex_home = AbsolutePathBuf::try_from(tempdir.path().join("home")).expect("abs home");
    tokio::fs::create_dir_all(cwd.join(".codex").join("compact").as_path())
        .await
        .expect("create compact dir");
    tokio::fs::create_dir_all(cwd.join(".codex").join("memory").as_path())
        .await
        .expect("create memory dir");
    tokio::fs::write(
        cwd.join(".codex").join("compact").join("COMPACT.md").as_path(),
        "workspace compact prompt",
    )
    .await
    .expect("write compact prompt");
    let oversized = "事实 ".repeat(3_000);
    tokio::fs::write(
        cwd.join(".codex")
            .join("memory")
            .join("user-preferences.md")
            .as_path(),
        &oversized,
    )
    .await
    .expect("write user preferences");

    let service = FsCompactService::new();
    let layout = service
        .derive_memory_layout(&cwd, &codex_home, Some("workspace compact prompt"))
        .await
        .expect("derive layout");
    let bundle = service
        .read_memory_bundle(&layout)
        .await
        .expect("read bundle");

    let truncated = bundle
        .user_preferences
        .expect("user preferences should be present");
    assert!(truncated.len() < oversized.len());
}

#[test]
fn replacement_history_is_memory_backed_not_summary_only() {
    let service = FsCompactService::new();
    let history = service.build_replacement_history(ReplacementHistoryInput {
        initial_context: Vec::new(),
        memory_bundle: CompactMemoryBundle {
            user_preferences: Some("# User Preferences\n- 全程中文".to_string()),
            project_understanding: Some("# Project Understanding\n- typed display".to_string()),
            current_work: Some("# Current Work\n- compact".to_string()),
        },
        recent_real_user_messages: vec!["最近一次真实用户消息".to_string()],
        compact_marker_text: "<summary>\nMemory-backed checkpoint".to_string(),
    });

    assert_eq!(history.len(), 5);
    let texts = history
        .into_iter()
        .filter_map(|item| match item {
            ResponseItem::Message { content, .. } => Some(
                content
                    .into_iter()
                    .filter_map(|content_item| match content_item {
                        ContentItem::InputText { text } => Some(text),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(texts.iter().any(|text| text.contains("Memory checkpoint: current work")));
    assert!(texts.iter().any(|text| text.contains("Memory checkpoint: project understanding")));
    assert!(texts.iter().any(|text| text.starts_with("<summary>\nMemory-backed checkpoint")));
}

#[test]
fn compact_window_ignores_memory_checkpoint_and_context_noise() {
    let service = FsCompactService::new();
    let window = service.summarize_compact_window(
        &[
            user_message("<summary>\nMemory-backed checkpoint"),
            user_message("Memory checkpoint: current work\n# Current Work\n- compact"),
            user_message(
                r#"<environment_context>
  <cwd>/repo</cwd>
</environment_context>"#,
            ),
            user_message("真实用户进展一"),
            user_message("真实用户进展二"),
        ],
        "<summary>",
    );

    assert_eq!(
        window.recent_real_user_messages,
        vec!["真实用户进展一".to_string(), "真实用户进展二".to_string()]
    );
    assert_eq!(window.turns_since_last_compact, 2);
}

#[test]
fn current_work_completeness_treats_placeholder_sections_as_empty() {
    let service = FsCompactService::new();
    let completeness = service.current_work_completeness(Some(
        "# Current Work

## Current Goal
- 暂无

## Current Status
- 暂无

## Files Already Read
- 暂无

## Key Findings
- 暂无

## Next Steps
- 暂无",
    ));

    assert_eq!(completeness, 0.0);
}

#[test]
fn soft_compact_prefers_incomplete_current_work_inside_soft_window() {
    let service = FsCompactService::new();
    let decision = service.evaluate_soft_compact(SoftCompactInputs {
        usage_ratio: 0.76,
        turns_since_last_compact: 3,
        recent_file_read_search_count: 2,
        recent_tool_output_bytes: 1024,
        current_work_completeness: 0.2,
        cooldown_turns_satisfied: true,
        cooldown_bytes_satisfied: true,
    });
    assert_eq!(decision.should_compact, true);
    assert_eq!(decision.reason, "local current-work memory is incomplete");
}
