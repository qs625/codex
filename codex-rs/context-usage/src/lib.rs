use codex_context_manager::ContextManager;
use codex_context_manager::estimate_response_item_model_visible_bytes;
use codex_context_manager::is_contextual_dev_message_content;
use codex_context_manager::is_contextual_user_fragment;
use codex_context_manager::is_contextual_user_message_content;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::models::ContentItem;
use protocol::models::FunctionCallOutputContentItem;
use protocol::models::LocalShellAction;
use protocol::models::ResponseItem;
use protocol::protocol::APPS_INSTRUCTIONS_CLOSE_TAG;
use protocol::protocol::APPS_INSTRUCTIONS_OPEN_TAG;
use protocol::protocol::COLLABORATION_MODE_CLOSE_TAG;
use protocol::protocol::COLLABORATION_MODE_OPEN_TAG;
use protocol::protocol::PLUGINS_INSTRUCTIONS_CLOSE_TAG;
use protocol::protocol::PLUGINS_INSTRUCTIONS_OPEN_TAG;
use protocol::protocol::REALTIME_CONVERSATION_CLOSE_TAG;
use protocol::protocol::REALTIME_CONVERSATION_OPEN_TAG;
use protocol::protocol::SKILLS_INSTRUCTIONS_CLOSE_TAG;
use protocol::protocol::SKILLS_INSTRUCTIONS_OPEN_TAG;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::ThreadContextUsageCategoryBreakdown;
use protocol::protocol::ThreadContextUsageLoadedSkills;
use protocol::protocol::ThreadContextUsageSkill;
use protocol::protocol::ThreadContextUsageToolBreakdown;
use protocol::protocol::ThreadContextUsageToolBucket;
use protocol::protocol::ThreadSkill;
use protocol::protocol::ThreadSkillKind;
use serde_json::Value;
use skill_service_api::SkillLoadOutcome;
use skill_service_api::detect_implicit_skill_invocation_for_command;
use std::collections::HashMap;

const SKILL_OPEN_TAG: &str = "<skill>";
const SKILL_CLOSE_TAG: &str = "</skill>";

#[derive(Clone)]
struct PendingSkillOutput {
    path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolBreakdownBucketId {
    ApplyPatch,
    FileOperations,
    Commands,
    InterAgent,
    SearchMedia,
    OtherTools,
}

#[derive(Default)]
struct ToolBreakdownAccumulator {
    breakdown: ThreadContextUsageToolBreakdown,
    pending_call_buckets: HashMap<String, ToolBreakdownBucketId>,
}

impl ToolBreakdownAccumulator {
    fn add_input(&mut self, bucket: ToolBreakdownBucketId, bytes: i64) {
        let bucket = self.bucket_mut(bucket);
        bucket.input = bucket.input.saturating_add(bytes);
    }

    fn add_output(&mut self, bucket: ToolBreakdownBucketId, bytes: i64) {
        let bucket = self.bucket_mut(bucket);
        bucket.output = bucket.output.saturating_add(bytes);
    }

    fn remember_call(&mut self, call_id: impl Into<String>, bucket: ToolBreakdownBucketId) {
        self.pending_call_buckets.insert(call_id.into(), bucket);
    }

    fn output_bucket_for_call(&self, call_id: &str) -> ToolBreakdownBucketId {
        self.pending_call_buckets
            .get(call_id)
            .copied()
            .unwrap_or(ToolBreakdownBucketId::OtherTools)
    }

    fn into_breakdown(self) -> ThreadContextUsageToolBreakdown {
        self.breakdown
    }

    fn bucket_mut(&mut self, bucket: ToolBreakdownBucketId) -> &mut ThreadContextUsageToolBucket {
        match bucket {
            ToolBreakdownBucketId::ApplyPatch => &mut self.breakdown.apply_patch,
            ToolBreakdownBucketId::FileOperations => &mut self.breakdown.file_operations,
            ToolBreakdownBucketId::Commands => &mut self.breakdown.commands,
            ToolBreakdownBucketId::InterAgent => &mut self.breakdown.inter_agent,
            ToolBreakdownBucketId::SearchMedia => &mut self.breakdown.search_media,
            ToolBreakdownBucketId::OtherTools => &mut self.breakdown.other_tools,
        }
    }
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

pub struct ContextUsageSkillDetection<'a> {
    pub outcome: &'a SkillLoadOutcome,
    pub cwd: &'a AbsolutePathBuf,
    pub total_count: Option<u32>,
}

pub fn build_thread_context_usage(
    history: &ContextManager,
    thread_skills: &[ThreadSkill],
    skill_detection: Option<ContextUsageSkillDetection<'_>>,
    is_summary_message: fn(&str) -> bool,
) -> ThreadContextUsage {
    build_thread_context_usage_inner(history, thread_skills, skill_detection, is_summary_message)
}

pub fn build_thread_context_usage_from_history(
    history: &ContextManager,
    thread_skills: &[ThreadSkill],
    is_summary_message: fn(&str) -> bool,
) -> ThreadContextUsage {
    build_thread_context_usage_inner(history, thread_skills, None, is_summary_message)
}

fn build_thread_context_usage_inner(
    history: &ContextManager,
    thread_skills: &[ThreadSkill],
    skill_detection: Option<ContextUsageSkillDetection<'_>>,
    is_summary_message: fn(&str) -> bool,
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
    let mut tool_breakdown = ToolBreakdownAccumulator::default();
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
                        } else if content.iter().any(|item| match item {
                            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                                is_summary_message(text)
                            }
                            ContentItem::InputImage { .. } => false,
                        }) {
                            categories.compact = categories.compact.saturating_add(item_bytes);
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
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                categories.tool_calls = categories
                    .tool_calls
                    .saturating_add(item_bytes);
                let bucket = classify_shell_command(action.command.as_slice());
                tool_breakdown.add_input(bucket, item_bytes);
                if let Some(call_id) = call_id.as_ref() {
                    tool_breakdown.remember_call(call_id.clone(), bucket);
                }
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
                call_id,
                name,
                arguments,
                ..
            } => {
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                categories.tool_calls = categories
                    .tool_calls
                    .saturating_add(item_bytes);
                let bucket = classify_function_call(name.as_str(), arguments.as_str());
                tool_breakdown.add_input(bucket, item_bytes);
                tool_breakdown.remember_call(call_id.clone(), bucket);
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
                let bucket = tool_breakdown.output_bucket_for_call(call_id);
                tool_breakdown.add_output(bucket, function_call_output_bytes(output));
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
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                categories.tool_calls = categories.tool_calls.saturating_add(item_bytes);
                let bucket = classify_tool_name(name.as_str(), Some(input.as_str()));
                tool_breakdown.add_input(bucket, item_bytes);
                tool_breakdown.remember_call(call_id.clone(), bucket);
            }
            ResponseItem::CustomToolCallOutput {
                call_id,
                name,
                output,
            } => {
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                categories.tool_calls = categories.tool_calls.saturating_add(item_bytes);
                let bucket = name
                    .as_deref()
                    .map(|name| classify_tool_name(name, None))
                    .unwrap_or_else(|| tool_breakdown.output_bucket_for_call(call_id));
                tool_breakdown.add_output(bucket, function_call_output_bytes(output));
            }
            ResponseItem::ToolSearchCall { call_id, .. } => {
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                categories.tool_calls = categories.tool_calls.saturating_add(item_bytes);
                tool_breakdown.add_input(ToolBreakdownBucketId::SearchMedia, item_bytes);
                if let Some(call_id) = call_id.as_ref() {
                    tool_breakdown
                        .remember_call(call_id.clone(), ToolBreakdownBucketId::SearchMedia);
                }
            }
            ResponseItem::ToolSearchOutput { call_id, tools, .. } => {
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                categories.tool_calls = categories.tool_calls.saturating_add(item_bytes);
                let bucket = call_id
                    .as_deref()
                    .map(|call_id| tool_breakdown.output_bucket_for_call(call_id))
                    .unwrap_or(ToolBreakdownBucketId::SearchMedia);
                let output_bytes = serde_json::to_string(tools)
                    .map(|text| text_bytes_len(text.as_str()))
                    .unwrap_or(item_bytes);
                tool_breakdown.add_output(bucket, output_bytes);
            }
            ResponseItem::WebSearchCall { .. } | ResponseItem::ImageGenerationCall { .. } => {
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                categories.tool_calls = categories.tool_calls.saturating_add(item_bytes);
                tool_breakdown.add_input(ToolBreakdownBucketId::SearchMedia, item_bytes);
            }
            ResponseItem::CommandWait { .. }
            | ResponseItem::CommandWriteStdin { .. }
            | ResponseItem::WorkflowRunProgress { .. }
            | ResponseItem::CommandExecutionNotification { .. }
            | ResponseItem::EventCommandEvent { .. }
            | ResponseItem::EventDrivenTool { .. }
            | ResponseItem::ThreadGoalUpdate { .. } => {
                categories.tools_metadata = categories
                    .tools_metadata
                    .saturating_add(estimate_response_item_model_visible_bytes(item));
            }
            ResponseItem::InterAgentCommunication { .. } => {
                let item_bytes = estimate_response_item_model_visible_bytes(item);
                categories.llm_messages = categories.llm_messages.saturating_add(item_bytes);
                tool_breakdown.add_output(ToolBreakdownBucketId::InterAgent, item_bytes);
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
        tool_breakdown: tool_breakdown.into_breakdown(),
    }
}

fn classify_function_call(name: &str, arguments: &str) -> ToolBreakdownBucketId {
    if is_exec_command_tool_name(name)
        && let Some(command) = extract_command_string(arguments)
    {
        return classify_shell_command_string(command.as_str());
    }
    classify_tool_name(name, Some(arguments))
}

fn classify_tool_name(name: &str, input: Option<&str>) -> ToolBreakdownBucketId {
    let normalized = normalize_identifier(name);
    if normalized.contains("applypatch") {
        return ToolBreakdownBucketId::ApplyPatch;
    }
    if is_inter_agent_tool_name(normalized.as_str()) {
        return ToolBreakdownBucketId::InterAgent;
    }
    if is_exec_command_tool_name(normalized.as_str()) {
        return input
            .and_then(extract_command_string)
            .map(|command| classify_shell_command_string(command.as_str()))
            .unwrap_or(ToolBreakdownBucketId::Commands);
    }
    if is_search_or_media_tool_name(normalized.as_str()) {
        return ToolBreakdownBucketId::SearchMedia;
    }
    if is_file_operation_tool_name(normalized.as_str()) {
        return ToolBreakdownBucketId::FileOperations;
    }
    ToolBreakdownBucketId::OtherTools
}

fn classify_shell_command(command: &[String]) -> ToolBreakdownBucketId {
    classify_shell_command_string(command.join(" ").as_str())
}

fn classify_shell_command_string(command: &str) -> ToolBreakdownBucketId {
    let normalized = command.trim();
    if normalized.is_empty() {
        return ToolBreakdownBucketId::Commands;
    }
    let tokens = shell_like_tokens(normalized);
    let command_token = tokens
        .iter()
        .map(String::as_str)
        .find(|token| *token != "rtk" && !is_env_assignment(token))
        .unwrap_or_default();
    let command_name = command_token.rsplit('/').next().unwrap_or(command_token);
    let first = normalize_identifier(command_name);
    if first == "git" {
        return classify_git_command(tokens.as_slice());
    }
    if matches!(
        first.as_str(),
        "sed" | "rg" | "grep" | "find" | "ls" | "cat" | "nl" | "wc" | "head" | "tail" | "stat"
    ) {
        return ToolBreakdownBucketId::FileOperations;
    }
    ToolBreakdownBucketId::Commands
}

fn classify_git_command(tokens: &[String]) -> ToolBreakdownBucketId {
    let subcommand = tokens
        .iter()
        .map(String::as_str)
        .skip_while(|token| *token != "git")
        .nth(1)
        .unwrap_or_default();
    if matches!(
        subcommand,
        "diff" | "show" | "status" | "log" | "grep" | "ls-files" | "branch"
    ) {
        ToolBreakdownBucketId::FileOperations
    } else {
        ToolBreakdownBucketId::Commands
    }
}

fn shell_like_tokens(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(|token| token.trim_matches(|ch| ch == '"' || ch == '\'').to_string())
        .collect()
}

fn is_env_assignment(token: &str) -> bool {
    token
        .split_once('=')
        .is_some_and(|(name, _)| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        })
}

fn is_exec_command_tool_name(name: &str) -> bool {
    let normalized = normalize_identifier(name);
    normalized == "execcommand" || normalized == "localshellcall"
}

fn is_inter_agent_tool_name(normalized: &str) -> bool {
    normalized.contains("collab")
        || normalized.contains("interagent")
        || normalized.contains("spawnagent")
        || normalized.contains("followuptask")
        || normalized.contains("listagents")
        || normalized.contains("closeagent")
}

fn is_search_or_media_tool_name(normalized: &str) -> bool {
    normalized.contains("search")
        || normalized.contains("web")
        || normalized.contains("image")
        || normalized.contains("screenshot")
        || normalized.contains("browser")
}

fn is_file_operation_tool_name(normalized: &str) -> bool {
    normalized.contains("read")
        || normalized.contains("open")
        || normalized.contains("find")
        || normalized.contains("file")
        || normalized.contains("document")
}

fn normalize_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn extract_command_string(arguments: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(arguments).ok()?;
    value
        .get("cmd")
        .and_then(Value::as_str)
        .or_else(|| value.get("command").and_then(Value::as_str))
        .map(str::to_string)
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

fn function_call_output_bytes(output: &protocol::models::FunctionCallOutputPayload) -> i64 {
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
    use super::build_thread_context_usage_from_history;
    use super::classify_developer_message;
    use super::parse_explicit_skill_injections;
    use codex_context_manager::ContextManager;
    use pretty_assertions::assert_eq;
    use protocol::AgentPath;
    use protocol::models::ContentItem;
    use protocol::models::FunctionCallOutputPayload;
    use protocol::models::LocalShellAction;
    use protocol::models::LocalShellExecAction;
    use protocol::models::LocalShellStatus;
    use protocol::models::ResponseItem;
    use protocol::protocol::InterAgentCommunication;
    use protocol::protocol::InterAgentOperation;
    use protocol::protocol::SKILLS_INSTRUCTIONS_CLOSE_TAG;
    use protocol::protocol::SKILLS_INSTRUCTIONS_OPEN_TAG;
    use protocol::protocol::ThreadContextUsage;
    use protocol::protocol::TruncationPolicy;
    use serde_json::json;
    use std::collections::HashMap;

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

    #[test]
    fn tool_breakdown_tracks_apply_patch_input_and_output() {
        let usage = usage_for_items(vec![
            ResponseItem::FunctionCall {
                id: None,
                name: "apply_patch".to_string(),
                namespace: None,
                arguments: "*** Begin Patch\n*** End Patch".to_string(),
                call_id: "call-apply".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-apply".to_string(),
                output: FunctionCallOutputPayload::from_text("patched".to_string()),
            },
        ]);

        assert!(usage.tool_breakdown.apply_patch.input > 0);
        assert_eq!(usage.tool_breakdown.apply_patch.output, 7);
        assert_eq!(usage.tool_breakdown.commands.input, 0);
    }

    #[test]
    fn tool_breakdown_classifies_shell_commands() {
        let usage = usage_for_items(vec![
            ResponseItem::FunctionCall {
                id: None,
                name: "exec_command".to_string(),
                namespace: None,
                arguments: json!({ "cmd": "rtk rg -n ThreadContextUsage codex-rs" }).to_string(),
                call_id: "call-rg".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-rg".to_string(),
                output: FunctionCallOutputPayload::from_text("match".to_string()),
            },
            ResponseItem::LocalShellCall {
                id: None,
                call_id: Some("call-cargo".to_string()),
                status: LocalShellStatus::Completed,
                action: LocalShellAction::Exec(LocalShellExecAction {
                    command: vec![
                        "rtk".to_string(),
                        "cargo".to_string(),
                        "test".to_string(),
                        "-p".to_string(),
                        "context-usage".to_string(),
                    ],
                    timeout_ms: None,
                    working_directory: None,
                    env: Some(HashMap::new()),
                    user: None,
                }),
            },
        ]);

        assert!(usage.tool_breakdown.file_operations.input > 0);
        assert_eq!(usage.tool_breakdown.file_operations.output, 5);
        assert!(usage.tool_breakdown.commands.input > 0);
    }

    #[test]
    fn tool_breakdown_tracks_inter_agent_context_separately() {
        let usage = usage_for_items(vec![
            ResponseItem::FunctionCall {
                id: None,
                name: "followup_task".to_string(),
                namespace: None,
                arguments: json!({ "target": "/root/worker", "message": "continue" }).to_string(),
                call_id: "call-followup".to_string(),
            },
            ResponseItem::InterAgentCommunication {
                id: None,
                communication: InterAgentCommunication::new(
                    AgentPath::root(),
                    AgentPath::root().join("worker").expect("worker path"),
                    Vec::new(),
                    "done".to_string(),
                    InterAgentOperation::ChildCompletion,
                ),
            },
        ]);

        assert!(usage.tool_breakdown.inter_agent.input > 0);
        assert!(usage.tool_breakdown.inter_agent.output > 0);
        assert_eq!(usage.tool_breakdown.other_tools.input, 0);
    }

    #[test]
    fn thread_context_usage_deserializes_without_tool_breakdown() {
        let usage: ThreadContextUsage = serde_json::from_value(json!({
            "totalBytes": 10,
            "budgetUsedPercent": null,
            "categories": {
                "compact": 0,
                "skillsMetadata": 0,
                "concreteSkills": 0,
                "toolsMetadata": 0,
                "toolCalls": 0,
                "userMessages": 10,
                "llmMessages": 0,
                "reasoning": 0
            },
            "loadedSkills": {
                "loadedCount": 0,
                "totalCount": null,
                "skills": []
            }
        }))
        .expect("legacy context usage should deserialize");

        assert_eq!(usage.tool_breakdown.apply_patch.input, 0);
        assert_eq!(usage.tool_breakdown.inter_agent.output, 0);
    }

    fn usage_for_items(items: Vec<ResponseItem>) -> ThreadContextUsage {
        let mut history = ContextManager::new();
        history.record_items(items.iter(), TruncationPolicy::Tokens(10_000));
        build_thread_context_usage_from_history(&history, &[], |_| false)
    }
}
