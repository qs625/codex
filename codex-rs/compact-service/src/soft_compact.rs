use compact_service_api::CompactWindowSummary;
use compact_service_api::SoftCompactDecision;
use compact_service_api::SoftCompactInputs;
use protocol::models::ResponseItem;

const MEMORY_CHECKPOINT_PREFIX: &str = "Memory checkpoint:";
const SOFT_COMPACT_LOWER_BOUND: f64 = 0.70;
const HARD_COMPACT_BOUND: f64 = 0.85;
const HEAVY_TOOL_OUTPUT_BYTES: usize = 24_000;
const HEAVY_FILE_READ_SEARCH_COUNT: usize = 6;
const MIN_TURNS_SINCE_LAST_COMPACT: usize = 2;

pub(super) fn summarize_compact_window(
    items: &[ResponseItem],
    summary_prefix: &str,
) -> CompactWindowSummary {
    let recent_real_user_messages =
        codex_context_manager::collect_compaction_user_messages(items, Some(summary_prefix))
            .into_iter()
            .filter(|message| !message.starts_with(MEMORY_CHECKPOINT_PREFIX))
            .collect::<Vec<_>>();
    let turns_since_last_compact = recent_real_user_messages.len();
    let recent_file_read_search_count = items
        .iter()
        .filter(|item| is_file_read_or_search_tool(item))
        .count();
    let recent_tool_output_bytes = items.iter().map(tool_output_bytes).sum::<usize>();

    CompactWindowSummary {
        recent_real_user_messages,
        turns_since_last_compact,
        recent_file_read_search_count,
        recent_tool_output_bytes,
    }
}

pub(super) fn evaluate_soft_compact(inputs: SoftCompactInputs) -> SoftCompactDecision {
    if inputs.usage_ratio < SOFT_COMPACT_LOWER_BOUND {
        return decision(false, "usage below soft compact threshold");
    }
    if inputs.usage_ratio >= HARD_COMPACT_BOUND {
        return decision(true, "usage reached hard compact threshold");
    }
    if !inputs.cooldown_turns_satisfied && !inputs.cooldown_bytes_satisfied {
        return decision(false, "soft compact cooldown is still active");
    }
    if inputs.current_work_completeness < 0.8 {
        return decision(true, "local current-work memory is incomplete");
    }
    if inputs.recent_tool_output_bytes >= HEAVY_TOOL_OUTPUT_BYTES {
        return decision(true, "recent tool output volume is high");
    }
    if inputs.recent_file_read_search_count >= HEAVY_FILE_READ_SEARCH_COUNT {
        return decision(true, "recent file read/search activity is high");
    }
    if inputs.turns_since_last_compact < MIN_TURNS_SINCE_LAST_COMPACT {
        return decision(false, "too little new user progress since last compact");
    }
    decision(
        true,
        "soft compact window exceeded with enough new progress",
    )
}

fn decision(should_compact: bool, reason: &str) -> SoftCompactDecision {
    SoftCompactDecision {
        should_compact,
        reason: reason.to_string(),
    }
}

fn is_file_read_or_search_tool(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::FunctionCall {
            name, arguments, ..
        } => {
            matches!(name.as_str(), "open" | "find" | "view_image" | "search")
                || (name == "exec_command"
                    && ["rg ", "sed ", "cat ", "git show", "rg\n"]
                        .iter()
                        .any(|needle| arguments.contains(needle)))
        }
        _ => false,
    }
}

fn tool_output_bytes(item: &ResponseItem) -> usize {
    match item {
        ResponseItem::FunctionCallOutput { output, .. }
        | ResponseItem::CustomToolCallOutput { output, .. } => output
            .body
            .to_text()
            .map(|text| text.len())
            .unwrap_or_default(),
        _ => 0,
    }
}
