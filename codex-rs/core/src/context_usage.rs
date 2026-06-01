use crate::SkillLoadOutcome;
use crate::TurnContext;
use crate::context::is_contextual_user_fragment;
use crate::context_manager::ContextManager;
use crate::context_manager::estimate_response_item_model_visible_bytes;
use crate::event_mapping::is_contextual_dev_message_content;
use crate::event_mapping::is_contextual_user_message_content;
use codex_core_skills::detect_implicit_skill_invocation_for_command;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::APPS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::APPS_INSTRUCTIONS_OPEN_TAG;
use codex_protocol::protocol::COLLABORATION_MODE_CLOSE_TAG;
use codex_protocol::protocol::COLLABORATION_MODE_OPEN_TAG;
use codex_protocol::protocol::PLUGINS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::PLUGINS_INSTRUCTIONS_OPEN_TAG;
use codex_protocol::protocol::REALTIME_CONVERSATION_CLOSE_TAG;
use codex_protocol::protocol::REALTIME_CONVERSATION_OPEN_TAG;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_OPEN_TAG;
use codex_protocol::protocol::ThreadContextUsage;
use codex_protocol::protocol::ThreadContextUsageCategoryBreakdown;
use codex_protocol::protocol::ThreadContextUsageLoadedSkills;
use codex_protocol::protocol::ThreadContextUsageSkill;
use codex_protocol::protocol::ThreadSkill;
use codex_protocol::protocol::ThreadSkillKind;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use std::collections::HashMap;

const SKILL_OPEN_TAG: &str = "<skill>";
const SKILL_CLOSE_TAG: &str = "</skill>";

#[derive(Clone)]
struct PendingSkillOutput {
    path: String,
}

#[derive(Default)]
struct SkillUsageAccumulator {
    skills: HashMap<String, ThreadContextUsageSkill>,
}

impl SkillUsageAccumulator {
    fn seed(&mut self, thread_skills: &[ThreadSkill]) {
        for skill in thread_skills {
            self.skills
                .entry(skill.path.clone())
                .or_insert_with(|| ThreadContextUsageSkill {
                    name: skill.name.clone(),
                    path: skill.path.clone(),
                    kind: skill.kind,
                    load_count: 0,
                });
        }
    }

    fn increment(&mut self, name: &str, path: &str, kind: ThreadSkillKind) {
        let entry =
            self.skills
                .entry(path.to_string())
                .or_insert_with(|| ThreadContextUsageSkill {
                    name: name.to_string(),
                    path: path.to_string(),
                    kind,
                    load_count: 0,
                });
        entry.kind = merge_skill_kind(entry.kind, kind);
        entry.load_count = entry.load_count.saturating_add(1);
    }

    fn into_loaded_skills(self, total_count: Option<u32>) -> ThreadContextUsageLoadedSkills {
        let mut skills = self
            .skills
            .into_values()
            .filter(|skill| skill.load_count > 0)
            .collect::<Vec<_>>();
        skills.sort_by(|left, right| {
            right
                .load_count
                .cmp(&left.load_count)
                .then_with(|| left.name.cmp(&right.name))
        });
        ThreadContextUsageLoadedSkills {
            loaded_count: u32::try_from(skills.len()).unwrap_or(u32::MAX),
            total_count,
            skills,
        }
    }
}

pub(crate) fn build_thread_context_usage(
    history: &ContextManager,
    turn_context: &TurnContext,
    thread_skills: &[ThreadSkill],
) -> ThreadContextUsage {
    let total_skills = turn_context
        .turn_skills
        .outcome
        .skills_with_enabled()
        .filter(|(_, enabled)| *enabled)
        .count();
    build_thread_context_usage_inner(
        history,
        thread_skills,
        Some(SkillDetectionContext {
            outcome: &turn_context.turn_skills.outcome,
            cwd: selected_turn_cwd(turn_context),
            total_count: Some(u32::try_from(total_skills).unwrap_or(u32::MAX)),
        }),
    )
}

pub(crate) fn build_thread_context_usage_from_history(
    history: &ContextManager,
    thread_skills: &[ThreadSkill],
) -> ThreadContextUsage {
    build_thread_context_usage_inner(history, thread_skills, None)
}

struct SkillDetectionContext<'a> {
    outcome: &'a SkillLoadOutcome,
    cwd: &'a AbsolutePathBuf,
    total_count: Option<u32>,
}

fn build_thread_context_usage_inner(
    history: &ContextManager,
    thread_skills: &[ThreadSkill],
    skill_detection: Option<SkillDetectionContext<'_>>,
) -> ThreadContextUsage {
    let mut categories = ThreadContextUsageCategoryBreakdown {
        compact: 0,
        skills_metadata: 0,
        concrete_skills: 0,
        tools_metadata: 0,
        tool_calls: 0,
        user_messages: 0,
        llm_messages: 0,
        reasoning: 0,
    };
    let mut pending_skill_outputs = HashMap::<String, PendingSkillOutput>::new();
    let mut skills = SkillUsageAccumulator::default();
    skills.seed(thread_skills);

    for item in history.raw_items() {
        match item {
            ResponseItem::Message { role, content, .. } => {
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                match role.as_str() {
                    "user" => {
                        let injected_skills = parse_explicit_skill_injections(content);
                        if !injected_skills.is_empty() {
                            for injected_skill in injected_skills {
                                categories.skills_metadata = categories
                                    .skills_metadata
                                    .saturating_add(injected_skill.metadata_bytes);
                                categories.concrete_skills = categories
                                    .concrete_skills
                                    .saturating_add(injected_skill.concrete_bytes);
                                skills.increment(
                                    injected_skill.name.as_str(),
                                    injected_skill.path.as_str(),
                                    ThreadSkillKind::Explicit,
                                );
                            }
                            categories.tools_metadata = categories
                                .tools_metadata
                                .saturating_add(non_skill_contextual_user_bytes(content));
                        } else if is_contextual_user_message_content(content) {
                            categories.tools_metadata =
                                categories.tools_metadata.saturating_add(item_bytes);
                        } else {
                            categories.user_messages =
                                categories.user_messages.saturating_add(item_bytes);
                        }
                    }
                    "assistant" => {
                        categories.llm_messages =
                            categories.llm_messages.saturating_add(item_bytes);
                    }
                    "developer" => {
                        let developer_usage = classify_developer_message(content);
                        categories.skills_metadata = categories
                            .skills_metadata
                            .saturating_add(developer_usage.skills_metadata);
                        categories.tools_metadata = categories
                            .tools_metadata
                            .saturating_add(developer_usage.tools_metadata);
                        categories.llm_messages = categories
                            .llm_messages
                            .saturating_add(developer_usage.llm_messages);
                    }
                    _ => {}
                }
            }
            ResponseItem::Reasoning { .. } => {
                categories.reasoning = categories
                    .reasoning
                    .saturating_add(estimate_response_item_model_visible_bytes(item));
            }
            ResponseItem::Compaction { .. } | ResponseItem::ContextCompaction { .. } => {
                categories.compact = categories
                    .compact
                    .saturating_add(estimate_response_item_model_visible_bytes(item));
            }
            ResponseItem::LocalShellCall {
                call_id,
                action: LocalShellAction::Exec(action),
                ..
            } => {
                categories.tool_calls = categories
                    .tool_calls
                    .saturating_add(estimate_response_item_model_visible_bytes(item));
                if let Some(skill_detection) = skill_detection.as_ref()
                    && let Some(call_id) = call_id.as_ref()
                    && let Some((name, path)) = detect_skill_for_command(
                        skill_detection.outcome,
                        action.command.as_slice(),
                        action.working_directory.as_deref(),
                        skill_detection.cwd,
                    )
                {
                    pending_skill_outputs
                        .insert(call_id.clone(), PendingSkillOutput { path: path.clone() });
                    skills.increment(name.as_str(), path.as_str(), ThreadSkillKind::Implicit);
                }
            }
            ResponseItem::FunctionCall {
                call_id, arguments, ..
            } => {
                categories.tool_calls = categories
                    .tool_calls
                    .saturating_add(estimate_response_item_model_visible_bytes(item));
                if let Some(skill_detection) = skill_detection.as_ref()
                    && let Some((command, workdir)) =
                        extract_command_from_function_arguments(arguments, skill_detection.cwd)
                    && let Some((name, path)) = detect_implicit_skill_invocation_for_command(
                        skill_detection.outcome,
                        command.as_str(),
                        &workdir,
                    )
                    .map(|skill| {
                        (
                            skill.name,
                            skill.path_to_skills_md.to_string_lossy().into_owned(),
                        )
                    })
                {
                    pending_skill_outputs
                        .insert(call_id.clone(), PendingSkillOutput { path: path.clone() });
                    skills.increment(name.as_str(), path.as_str(), ThreadSkillKind::Implicit);
                }
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                if let Some(skill_output) = pending_skill_outputs.get(call_id) {
                    categories.concrete_skills = categories
                        .concrete_skills
                        .saturating_add(function_call_output_bytes(output));
                    if let Some(skill) = skills.skills.get_mut(skill_output.path.as_str()) {
                        skill.kind = merge_skill_kind(skill.kind, ThreadSkillKind::Implicit);
                    }
                } else {
                    categories.tool_calls = categories
                        .tool_calls
                        .saturating_add(estimate_response_item_model_visible_bytes(item));
                }
            }
            ResponseItem::CustomToolCall { .. }
            | ResponseItem::CustomToolCallOutput { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. } => {
                categories.tool_calls = categories
                    .tool_calls
                    .saturating_add(estimate_response_item_model_visible_bytes(item));
            }
            ResponseItem::Other => {}
        }
    }

    let total_bytes = [
        categories.compact,
        categories.skills_metadata,
        categories.concrete_skills,
        categories.tools_metadata,
        categories.tool_calls,
        categories.user_messages,
        categories.llm_messages,
        categories.reasoning,
    ]
    .into_iter()
    .fold(0i64, i64::saturating_add);
    let budget_used_percent = history.token_info().and_then(|info| {
        info.model_context_window.and_then(|window| {
            if window <= 0 {
                None
            } else {
                Some(
                    info.total_token_usage
                        .total_tokens
                        .saturating_mul(100)
                        .saturating_div(window)
                        .clamp(0, 100),
                )
            }
        })
    });
    ThreadContextUsage {
        total_bytes,
        budget_used_percent,
        categories,
        loaded_skills: skills
            .into_loaded_skills(skill_detection.and_then(|context| context.total_count)),
    }
}

fn detect_skill_for_command(
    outcome: &SkillLoadOutcome,
    command: &[String],
    working_directory: Option<&str>,
    fallback_cwd: &AbsolutePathBuf,
) -> Option<(String, String)> {
    let command = command.join(" ");
    let workdir = working_directory
        .map(|cwd| AbsolutePathBuf::resolve_path_against_base(cwd, fallback_cwd.as_path()))
        .unwrap_or_else(|| fallback_cwd.clone());
    detect_implicit_skill_invocation_for_command(outcome, command.as_str(), &workdir).map(|skill| {
        (
            skill.name,
            skill.path_to_skills_md.to_string_lossy().into_owned(),
        )
    })
}

fn extract_command_from_function_arguments(
    arguments: &str,
    fallback_cwd: &AbsolutePathBuf,
) -> Option<(String, AbsolutePathBuf)> {
    let value = serde_json::from_str::<Value>(arguments).ok()?;
    let command = value
        .get("cmd")
        .and_then(Value::as_str)
        .or_else(|| value.get("command").and_then(Value::as_str))?
        .to_string();
    let workdir = value
        .get("cwd")
        .and_then(Value::as_str)
        .or_else(|| value.get("workdir").and_then(Value::as_str))
        .map(|cwd| AbsolutePathBuf::resolve_path_against_base(cwd, fallback_cwd.as_path()))
        .unwrap_or_else(|| fallback_cwd.clone());
    Some((command, workdir))
}

fn selected_turn_cwd(turn_context: &TurnContext) -> &AbsolutePathBuf {
    turn_context
        .environments
        .turn_environments
        .first()
        .map(|turn_environment| &turn_environment.cwd)
        .unwrap_or(&turn_context.config.cwd)
}

fn function_call_output_bytes(output: &codex_protocol::models::FunctionCallOutputPayload) -> i64 {
    if let Some(text) = output.body.to_text() {
        i64::try_from(text.len()).unwrap_or(i64::MAX)
    } else if let Some(items) = output.content_items() {
        items
            .iter()
            .map(function_call_output_content_item_bytes)
            .fold(0i64, i64::saturating_add)
    } else {
        0
    }
}

fn function_call_output_content_item_bytes(item: &FunctionCallOutputContentItem) -> i64 {
    match item {
        FunctionCallOutputContentItem::InputText { text } => {
            i64::try_from(text.len()).unwrap_or(i64::MAX)
        }
        FunctionCallOutputContentItem::InputImage { image_url, .. } => {
            i64::try_from(image_url.len()).unwrap_or(i64::MAX)
        }
    }
}

fn merge_skill_kind(current: ThreadSkillKind, next: ThreadSkillKind) -> ThreadSkillKind {
    match (current, next) {
        (ThreadSkillKind::All, _) | (_, ThreadSkillKind::All) => ThreadSkillKind::All,
        (ThreadSkillKind::Explicit, ThreadSkillKind::Implicit)
        | (ThreadSkillKind::Implicit, ThreadSkillKind::Explicit) => ThreadSkillKind::All,
        (kind, _) => kind,
    }
}

struct ParsedSkillInjection {
    name: String,
    path: String,
    metadata_bytes: i64,
    concrete_bytes: i64,
}

#[derive(Default)]
struct DeveloperUsageBreakdown {
    skills_metadata: i64,
    tools_metadata: i64,
    llm_messages: i64,
}

fn classify_developer_message(content: &[ContentItem]) -> DeveloperUsageBreakdown {
    let mut usage = DeveloperUsageBreakdown::default();

    for item in content {
        let ContentItem::InputText { text } = item else {
            continue;
        };
        let bytes = text_bytes_len(text);

        if tagged_fragment_body_len(
            text,
            SKILLS_INSTRUCTIONS_OPEN_TAG,
            SKILLS_INSTRUCTIONS_CLOSE_TAG,
        )
        .is_some()
        {
            usage.skills_metadata = usage.skills_metadata.saturating_add(bytes);
        } else if is_tagged_developer_tools_metadata(text) {
            usage.tools_metadata = usage.tools_metadata.saturating_add(bytes);
        } else {
            usage.llm_messages = usage.llm_messages.saturating_add(bytes);
        }
    }

    usage
}

fn is_tagged_developer_tools_metadata(text: &str) -> bool {
    is_tagged_fragment(
        text,
        APPS_INSTRUCTIONS_OPEN_TAG,
        APPS_INSTRUCTIONS_CLOSE_TAG,
    ) || is_tagged_fragment(
        text,
        PLUGINS_INSTRUCTIONS_OPEN_TAG,
        PLUGINS_INSTRUCTIONS_CLOSE_TAG,
    ) || is_tagged_fragment(
        text,
        COLLABORATION_MODE_OPEN_TAG,
        COLLABORATION_MODE_CLOSE_TAG,
    ) || is_tagged_fragment(
        text,
        REALTIME_CONVERSATION_OPEN_TAG,
        REALTIME_CONVERSATION_CLOSE_TAG,
    ) || is_tagged_fragment(
        text,
        "<permissions instructions>",
        "</permissions instructions>",
    ) || is_tagged_fragment(text, "<model_switch>", "</model_switch>")
        || is_tagged_fragment(text, "<personality_spec>", "</personality_spec>")
        || is_contextual_dev_message_content(&[ContentItem::InputText {
            text: text.to_string(),
        }])
}

fn is_tagged_fragment(text: &str, start: &str, end: &str) -> bool {
    tagged_fragment_body_len(text, start, end).is_some()
}

fn tagged_fragment_body_len(text: &str, start: &str, end: &str) -> Option<i64> {
    let trimmed = text.trim();
    let body = trimmed.strip_prefix(start)?.strip_suffix(end)?.trim();
    Some(text_bytes_len(body))
}

fn parse_explicit_skill_injections(content: &[ContentItem]) -> Vec<ParsedSkillInjection> {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } => parse_explicit_skill_injection_text(text),
            ContentItem::InputImage { .. } | ContentItem::OutputText { .. } => None,
        })
        .collect()
}

fn parse_explicit_skill_injection_text(text: &str) -> Option<ParsedSkillInjection> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix(SKILL_OPEN_TAG)?
        .strip_suffix(SKILL_CLOSE_TAG)?
        .trim();
    let (name, name_end) = extract_tag(body, "name")?;
    let body_after_name = body.get(name_end..)?.trim_start();
    let (path, path_end) = extract_tag(body_after_name, "path")?;
    let concrete = body_after_name.get(path_end..)?.trim();
    let metadata_prefix = body_after_name
        .get(..path_end)
        .map(str::trim)
        .unwrap_or_default();
    let metadata_bytes = text_bytes_len(SKILL_OPEN_TAG)
        .saturating_add(text_bytes_len(SKILL_CLOSE_TAG))
        .saturating_add(text_bytes_len(metadata_prefix));
    Some(ParsedSkillInjection {
        name: name.to_string(),
        path: path.to_string(),
        metadata_bytes,
        concrete_bytes: text_bytes_len(concrete),
    })
}

fn non_skill_contextual_user_bytes(content: &[ContentItem]) -> i64 {
    content
        .iter()
        .filter(|item| is_contextual_user_fragment(item))
        .filter_map(|item| match item {
            ContentItem::InputText { text }
                if parse_explicit_skill_injection_text(text).is_none() =>
            {
                Some(text_bytes_len(text))
            }
            ContentItem::InputText { .. }
            | ContentItem::InputImage { .. }
            | ContentItem::OutputText { .. } => None,
        })
        .fold(0i64, i64::saturating_add)
}

fn extract_tag<'a>(body: &'a str, tag: &str) -> Option<(&'a str, usize)> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let after_open = body.strip_prefix(open.as_str())?;
    let value_end = after_open.find(close.as_str())?;
    let value = after_open.get(..value_end)?;
    let consumed = open
        .len()
        .saturating_add(value_end)
        .saturating_add(close.len());
    Some((value.trim(), consumed))
}

fn text_bytes_len(text: &str) -> i64 {
    i64::try_from(text.len()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::classify_developer_message;
    use super::parse_explicit_skill_injections;
    use codex_protocol::models::ContentItem;
    use codex_protocol::protocol::SKILLS_INSTRUCTIONS_CLOSE_TAG;
    use codex_protocol::protocol::SKILLS_INSTRUCTIONS_OPEN_TAG;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_explicit_skill_injections_from_mixed_contextual_message() {
        let content = vec![ContentItem::InputText {
            text: "<environment_context>ctx</environment_context>".to_string(),
        }, ContentItem::InputText {
            text:
                "<skill>\n<name>demo</name>\n<path>/tmp/demo/SKILL.md</path>\nbody text\n</skill>"
                    .to_string(),
        }];

        let parsed = parse_explicit_skill_injections(content.as_slice());

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "demo");
        assert_eq!(parsed[0].path, "/tmp/demo/SKILL.md");
        assert_eq!(parsed[0].concrete_bytes, 9);
        assert!(parsed[0].metadata_bytes > 0);
    }

    #[test]
    fn classifies_skills_instructions_as_skill_metadata() {
        let content = vec![ContentItem::InputText {
            text: format!(
                "{SKILLS_INSTRUCTIONS_OPEN_TAG}\n## Skills\n- demo: description\n{SKILLS_INSTRUCTIONS_CLOSE_TAG}"
            ),
        }];

        let usage = classify_developer_message(content.as_slice());

        assert!(usage.skills_metadata > 0);
        assert_eq!(usage.tools_metadata, 0);
        assert_eq!(usage.llm_messages, 0);
    }
}
