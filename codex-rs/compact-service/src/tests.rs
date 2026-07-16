use super::*;
use compact_service_api::CompactMemoryRole;
use compact_service_api::CompactReplacementFile;
use compact_service_api::ReplacementHistoryInput;
use compact_service_api::SoftCompactInputs;
use compact_service_api::SoftCompactThresholds;
use pretty_assertions::assert_eq;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use tempfile::TempDir;

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

fn developer_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn assistant_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

#[tokio::test]
async fn reads_configured_replacement_files_without_missing_file_errors() {
    let tempdir = TempDir::new().expect("create temp dir");
    let cwd = AbsolutePathBuf::try_from(tempdir.path().to_path_buf()).expect("abs cwd");
    let service = FsCompactService::new();
    tokio::fs::create_dir_all(cwd.join(".codex").join("memory").as_path())
        .await
        .expect("create memory dir");
    tokio::fs::write(
        cwd.join(".codex")
            .join("memory")
            .join("current-work.md")
            .as_path(),
        "# Current Work\n\n## Current Goal\n- 切换 compact 主流程到 memory 中心",
    )
    .await
    .expect("write current work");

    let bundle = service
        .read_memory_bundle(&[
            CompactReplacementFile {
                path: cwd.join(".codex").join("memory").join("current-work.md"),
                role: CompactMemoryRole::CurrentWork,
                label: None,
                token_limit: 1_500,
            },
            CompactReplacementFile {
                path: cwd.join(".codex").join("memory").join("missing.md"),
                role: CompactMemoryRole::Custom,
                label: Some("missing".to_string()),
                token_limit: 1_500,
            },
        ])
        .await
        .expect("read bundle");

    assert_eq!(bundle.snapshots.len(), 1);
    assert_eq!(bundle.snapshots[0].label, "current work");
    assert!(
        bundle.snapshots[0]
            .content
            .contains("切换 compact 主流程到 memory 中心")
    );
}

#[tokio::test]
async fn read_memory_bundle_truncates_oversized_memory_files() {
    let tempdir = TempDir::new().expect("create temp dir");
    let cwd = AbsolutePathBuf::try_from(tempdir.path().to_path_buf()).expect("abs cwd");
    tokio::fs::create_dir_all(cwd.join(".codex").join("memory").as_path())
        .await
        .expect("create memory dir");
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
    let bundle = service
        .read_memory_bundle(&[CompactReplacementFile {
            path: cwd
                .join(".codex")
                .join("memory")
                .join("user-preferences.md"),
            role: CompactMemoryRole::UserPreferences,
            label: None,
            token_limit: 1_500,
        }])
        .await
        .expect("read bundle");

    let truncated = &bundle.snapshots[0].content;
    assert!(truncated.len() < oversized.len());
}

#[test]
fn replacement_history_only_keeps_recent_real_user_messages() {
    let service = FsCompactService::new();
    let history = service.build_replacement_history(ReplacementHistoryInput {
        initial_context: Vec::new(),
        memory_bundle: CompactMemoryBundle {
            snapshots: vec![
                compact_service_api::CompactMemorySnapshot {
                    role: CompactMemoryRole::UserPreferences,
                    label: "user preferences".to_string(),
                    content: "# User Preferences\n- 全程中文".to_string(),
                },
                compact_service_api::CompactMemorySnapshot {
                    role: CompactMemoryRole::ProjectUnderstanding,
                    label: "project understanding".to_string(),
                    content: "# Project Understanding\n- typed display".to_string(),
                },
                compact_service_api::CompactMemorySnapshot {
                    role: CompactMemoryRole::CurrentWork,
                    label: "current work".to_string(),
                    content: "# Current Work\n- compact".to_string(),
                },
            ],
        },
        recent_real_user_messages: vec!["最近一次真实用户消息".to_string()],
        final_output: Some("compact 最后一条输出".to_string()),
    });

    assert_eq!(history.len(), 2);
    let message_texts = history
        .into_iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } => Some((
                role,
                content
                    .into_iter()
                    .filter_map(|content_item| match content_item {
                        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                            Some(text)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        message_texts,
        vec![
            ("user".to_string(), "最近一次真实用户消息".to_string()),
            ("assistant".to_string(), "compact 最后一条输出".to_string()),
        ]
    );
}

#[test]
fn replacement_history_preserves_initial_context_and_caps_recent_user_messages() {
    let service = FsCompactService::new();
    let history = service.build_replacement_history(ReplacementHistoryInput {
        initial_context: vec![user_message("已有上下文")],
        memory_bundle: CompactMemoryBundle {
            snapshots: vec![compact_service_api::CompactMemorySnapshot {
                role: CompactMemoryRole::CurrentWork,
                label: "current work".to_string(),
                content: "# Current Work\n- 不应再复制进 history".to_string(),
            }],
        },
        recent_real_user_messages: vec![
            "较早的真实用户消息".to_string(),
            "倒数第二条真实用户消息".to_string(),
            "最近一条真实用户消息".to_string(),
        ],
        final_output: Some("compact 最后一条输出".to_string()),
    });

    let message_texts = history
        .into_iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } => Some((
                role,
                content
                    .into_iter()
                    .filter_map(|content_item| match content_item {
                        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                            Some(text)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        message_texts,
        vec![
            ("user".to_string(), "已有上下文".to_string()),
            ("user".to_string(), "倒数第二条真实用户消息".to_string()),
            ("user".to_string(), "最近一条真实用户消息".to_string()),
            ("assistant".to_string(), "compact 最后一条输出".to_string()),
        ]
    );
}

#[test]
fn compact_window_ignores_memory_checkpoint_and_context_noise() {
    let service = FsCompactService::new();
    let window = service.summarize_compact_window(
        &[
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
fn compact_window_treats_compact_prompt_as_control_context() {
    let service = FsCompactService::new();
    let compact_prompt = "Custom compact prompt from COMPACT.md";
    let window = service.summarize_compact_window(
        &[
            user_message("真实用户任务一"),
            developer_message(compact_prompt),
            user_message("真实用户任务二"),
            assistant_message("compact final output"),
        ],
        "<summary>",
    );

    assert_eq!(
        window.recent_real_user_messages,
        vec!["真实用户任务一".to_string(), "真实用户任务二".to_string()]
    );

    let history = service.build_replacement_history(ReplacementHistoryInput {
        initial_context: Vec::new(),
        memory_bundle: CompactMemoryBundle { snapshots: vec![] },
        recent_real_user_messages: window.recent_real_user_messages,
        final_output: Some("compact final output".to_string()),
    });
    let message_texts = history
        .into_iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } => Some((
                role,
                content
                    .into_iter()
                    .filter_map(|content_item| match content_item {
                        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                            Some(text)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        message_texts,
        vec![
            ("user".to_string(), "真实用户任务一".to_string()),
            ("user".to_string(), "真实用户任务二".to_string()),
            ("assistant".to_string(), "compact final output".to_string()),
        ]
    );
}

#[test]
fn current_work_completeness_treats_placeholder_sections_as_empty() {
    let service = FsCompactService::new();
    let completeness = service.current_work_completeness(&CompactMemoryBundle {
        snapshots: vec![compact_service_api::CompactMemorySnapshot {
            role: CompactMemoryRole::CurrentWork,
            label: "current work".to_string(),
            content: "# Current Work

## Current Goal
- 暂无

## Current Status
- 暂无

## Files Already Read
- 暂无

## Key Findings
- 暂无

## Next Steps
- 暂无"
                .to_string(),
        }],
    });

    assert_eq!(completeness, 0.0);
}

#[test]
fn current_work_completeness_accepts_non_template_content() {
    let service = FsCompactService::new();
    let completeness = service.current_work_completeness(&CompactMemoryBundle {
        snapshots: vec![compact_service_api::CompactMemorySnapshot {
            role: CompactMemoryRole::CurrentWork,
            label: "current work".to_string(),
            content: "# 当前工作

## 目标
- 把 compact prompt 回退到原始 COMPACT.md

## 现状
- runtime 已移除 output schema

## 下一步
- 补 schema fixture 和 review 回归测试"
                .to_string(),
        }],
    });

    assert!(completeness > 0.0);
}

#[test]
fn current_work_completeness_ignores_missing_snapshot() {
    let service = FsCompactService::new();
    let completeness = service.current_work_completeness(&CompactMemoryBundle::default());

    assert_eq!(completeness, 0.0);
}

fn soft_compact_inputs(usage_ratio: f64) -> SoftCompactInputs {
    SoftCompactInputs {
        usage_ratio,
        thresholds: SoftCompactThresholds::default(),
        turns_since_last_compact: 3,
        recent_file_read_search_count: 0,
        recent_tool_output_bytes: 1_024,
        current_work_completeness: 1.0,
        cooldown_turns_satisfied: true,
        cooldown_bytes_satisfied: true,
    }
}

#[test]
fn soft_compact_skips_usage_below_new_soft_threshold() {
    let service = FsCompactService::new();
    let decision = service.evaluate_soft_compact(SoftCompactInputs {
        recent_file_read_search_count: 2,
        current_work_completeness: 0.2,
        ..soft_compact_inputs(0.79)
    });

    assert_eq!(decision.should_compact, false);
    assert_eq!(decision.reason, "usage below soft compact threshold");
}

#[test]
fn soft_compact_skips_old_soft_window_with_enough_progress() {
    let service = FsCompactService::new();
    let decision = service.evaluate_soft_compact(soft_compact_inputs(0.76));

    assert_eq!(decision.should_compact, false);
    assert_eq!(decision.reason, "usage below soft compact threshold");
}

#[test]
fn soft_compact_prefers_incomplete_current_work_inside_new_soft_window() {
    let service = FsCompactService::new();
    let decision = service.evaluate_soft_compact(SoftCompactInputs {
        recent_file_read_search_count: 2,
        current_work_completeness: 0.2,
        ..soft_compact_inputs(0.81)
    });

    assert_eq!(decision.should_compact, true);
    assert_eq!(decision.reason, "local current-work memory is incomplete");
}

#[test]
fn soft_compact_treats_missing_current_work_as_neutral_inside_new_soft_window() {
    let service = FsCompactService::new();
    let decision = service.evaluate_soft_compact(SoftCompactInputs {
        turns_since_last_compact: 1,
        ..soft_compact_inputs(0.81)
    });

    assert_eq!(decision.should_compact, false);
    assert_eq!(
        decision.reason,
        "too little new user progress since last compact"
    );
}

#[test]
fn soft_compact_hard_threshold_ignores_cooldown() {
    let service = FsCompactService::new();
    let decision = service.evaluate_soft_compact(SoftCompactInputs {
        turns_since_last_compact: 0,
        recent_tool_output_bytes: 0,
        cooldown_turns_satisfied: false,
        cooldown_bytes_satisfied: false,
        ..soft_compact_inputs(0.90)
    });

    assert_eq!(decision.should_compact, true);
    assert_eq!(decision.reason, "usage reached hard compact threshold");
}

#[test]
fn soft_compact_uses_custom_thresholds() {
    let service = FsCompactService::new();
    let thresholds = SoftCompactThresholds::resolve(Some(0.60), Some(0.75)).unwrap();

    let soft_decision = service.evaluate_soft_compact(SoftCompactInputs {
        thresholds,
        ..soft_compact_inputs(0.61)
    });
    assert_eq!(soft_decision.should_compact, true);
    assert_eq!(
        soft_decision.reason,
        "soft compact window exceeded with enough new progress"
    );

    let hard_decision = service.evaluate_soft_compact(SoftCompactInputs {
        thresholds,
        turns_since_last_compact: 0,
        cooldown_turns_satisfied: false,
        cooldown_bytes_satisfied: false,
        ..soft_compact_inputs(0.75)
    });
    assert_eq!(hard_decision.should_compact, true);
    assert_eq!(
        hard_decision.reason,
        "usage reached hard compact threshold"
    );
}
