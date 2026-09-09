use crate::has_non_contextual_dev_message_content;
use crate::is_contextual_dev_message_content;
use crate::is_contextual_user_message_content;
use crate::normalize;
use codex_utils_image::base64_image_data_url_payload;
use codex_utils_image::estimate_original_image_data_url_bytes;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::approx_token_count;
use codex_utils_output_truncation::approx_tokens_from_byte_count_i64;
use codex_utils_output_truncation::truncate_function_output_items_with_policy;
use codex_utils_output_truncation::truncate_text;
use protocol::error::ModelContextQuarantineReference;
use protocol::error::ModelInputItemKind;
use protocol::error::ModelInputItemReference;
use protocol::models::BaseInstructions;
use protocol::models::ContentItem;
use protocol::models::FunctionCallOutputBody;
use protocol::models::FunctionCallOutputContentItem;
use protocol::models::FunctionCallOutputPayload;
use protocol::models::ImageDetail;
use protocol::models::ResponseItem;
use protocol::models::model_context_item_fingerprint;
use protocol::models::model_context_quarantine_notice;
use protocol::openai_models::InputModality;
use protocol::protocol::TokenUsage;
use protocol::protocol::TokenUsageInfo;
use protocol::protocol::TurnContextItem;
use std::borrow::Cow;
use std::ops::Deref;

/// Transcript of thread history
#[derive(Debug, Clone, Default)]
pub struct ContextManager {
    /// The oldest items are at the beginning of the vector.
    items: Vec<ResponseItem>,
    /// Bumped whenever history is rewritten, such as compaction or rollback.
    history_version: u64,
    token_info: Option<TokenUsageInfo>,
    /// Raw history length covered by `last_token_usage`.
    last_token_usage_history_len: Option<usize>,
    /// Reference context snapshot used for diffing and producing model-visible
    /// settings update items.
    ///
    /// This is the baseline for the next regular model turn, and may already
    /// match the current turn after context updates are persisted.
    ///
    /// When this is `None`, settings diffing treats the next turn as having no
    /// baseline and emits a full reinjection of context state. Rollback may
    /// also clear this when it trims a mixed initial-context developer bundle
    /// whose non-diff fragments no longer exist in the surviving history.
    reference_context_item: Option<TurnContextItem>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TotalTokenUsageBreakdown {
    pub last_api_response_total_tokens: i64,
    pub all_history_items_model_visible_bytes: i64,
    pub estimated_tokens_of_items_added_since_last_successful_api_response: i64,
    pub estimated_bytes_of_items_added_since_last_successful_api_response: i64,
}

impl ContextManager {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            history_version: 0,
            token_info: TokenUsageInfo::new_or_append(
                &None, &None, /*model_context_window*/ None,
            ),
            last_token_usage_history_len: None,
            reference_context_item: None,
        }
    }

    pub fn token_info(&self) -> Option<TokenUsageInfo> {
        self.token_info.clone()
    }

    pub fn set_token_info(&mut self, info: Option<TokenUsageInfo>) {
        self.last_token_usage_history_len = None;
        self.token_info = info;
    }

    pub fn set_recomputed_token_info(&mut self, info: TokenUsageInfo) {
        self.last_token_usage_history_len = Some(self.items.len());
        self.token_info = Some(info);
    }

    pub fn set_reference_context_item(&mut self, item: Option<TurnContextItem>) {
        self.reference_context_item = item;
    }

    pub fn reference_context_item(&self) -> Option<TurnContextItem> {
        self.reference_context_item.clone()
    }

    pub fn set_token_usage_full(&mut self, context_window: i64) {
        match &mut self.token_info {
            Some(info) => info.fill_to_context_window(context_window),
            None => {
                self.token_info = Some(TokenUsageInfo::full_context_window(context_window));
            }
        }
        self.last_token_usage_history_len = Some(self.items.len());
    }

    /// `items` is ordered from oldest to newest.
    pub fn record_items<I>(&mut self, items: I, policy: TruncationPolicy)
    where
        I: IntoIterator,
        I::Item: std::ops::Deref<Target = ResponseItem>,
    {
        for item in items {
            let item_ref = item.deref();
            if !is_api_message(item_ref) {
                continue;
            }

            let processed = self.process_item(item_ref, policy);
            self.items.push(processed);
        }
    }

    /// Returns the history prepared for sending to the model. This applies a proper
    /// normalization and drops un-suited items. When `input_modalities` does not
    /// include `InputModality::Image`, images are stripped from messages and tool
    /// outputs.
    pub fn for_prompt(mut self, input_modalities: &[InputModality]) -> Vec<ResponseItem> {
        self.items = self
            .projected_model_context_items()
            .into_iter()
            .map(|(_, item)| item.clone())
            .collect();
        self.normalize_history(input_modalities);
        self.items
    }

    /// Returns raw items in the history.
    pub fn raw_items(&self) -> &[ResponseItem] {
        &self.items
    }

    pub fn projected_model_context_items(&self) -> Vec<(usize, &ResponseItem)> {
        let quarantined = self
            .items
            .iter()
            .filter_map(|item| match item {
                ResponseItem::ModelContextQuarantine { target, .. } => Some(target.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                !quarantined
                    .iter()
                    .any(|target| response_item_belongs_to_quarantine_target(item, target))
            })
            .collect()
    }

    pub fn projected_model_input_items(&self) -> Vec<(usize, Cow<'_, ResponseItem>)> {
        self.projected_model_context_items()
            .into_iter()
            .map(|(index, item)| {
                let projected = match item {
                    ResponseItem::ModelContextQuarantine {
                        target,
                        reason,
                        error_code,
                        error_param,
                    } => Cow::Owned(model_context_quarantine_notice(
                        target,
                        reason,
                        error_code.as_deref(),
                        error_param.as_deref(),
                    )),
                    _ => Cow::Borrowed(item),
                };
                (index, projected)
            })
            .collect()
    }

    pub fn has_complete_unique_model_input_transaction(
        &self,
        target: &ModelInputItemReference,
    ) -> bool {
        if target.call_id.is_empty() || self.contains_model_context_quarantine(target) {
            return false;
        }
        let matching_calls = self
            .items
            .iter()
            .filter(|item| response_item_matches_target(item, target))
            .count();
        let matching_outputs = self
            .items
            .iter()
            .filter(|item| response_item_is_output_for_target(item, target))
            .count();
        let all_calls_with_id = self
            .items
            .iter()
            .filter(|item| model_call_id(item) == Some(target.call_id.as_str()))
            .count();
        let all_outputs_with_id = self
            .items
            .iter()
            .filter(|item| model_output_call_id(item) == Some(target.call_id.as_str()))
            .count();
        matching_calls == 1
            && matching_outputs == 1
            && all_calls_with_id == 1
            && all_outputs_with_id == 1
    }

    pub fn contains_model_context_quarantine(&self, target: &ModelInputItemReference) -> bool {
        self.items.iter().any(|item| {
            matches!(
                item,
                ResponseItem::ModelContextQuarantine {
                    target: ModelContextQuarantineReference::ToolTransaction {
                        kind,
                        call_id,
                    },
                    ..
                } if *kind == target.kind && call_id == &target.call_id
            )
        })
    }

    pub fn history_version(&self) -> u64 {
        self.history_version
    }

    pub fn estimate_token_count_with_base_instructions(
        &self,
        base_instructions: &BaseInstructions,
    ) -> Option<i64> {
        let base_tokens =
            i64::try_from(approx_token_count(&base_instructions.text)).unwrap_or(i64::MAX);

        let items_tokens = self
            .projected_model_input_items()
            .into_iter()
            .map(|(_, item)| estimate_item_token_count(item.as_ref()))
            .fold(0i64, i64::saturating_add);

        Some(base_tokens.saturating_add(items_tokens))
    }

    pub fn remove_first_item(&mut self) {
        if !self.items.is_empty() {
            // Remove the oldest item (front of the list). Items are ordered from
            // oldest → newest, so index 0 is the first entry recorded.
            let removed = self.items.remove(0);
            self.last_token_usage_history_len = None;
            // If the removed item participates in a call/output pair, also remove
            // its corresponding counterpart to keep the invariants intact without
            // running a full normalization pass.
            normalize::remove_corresponding_for(&mut self.items, &removed);
        }
    }

    pub fn remove_last_item(&mut self) -> bool {
        if let Some(removed) = self.items.pop() {
            self.last_token_usage_history_len = None;
            normalize::remove_corresponding_for(&mut self.items, &removed);
            self.history_version = self.history_version.saturating_add(1);
            true
        } else {
            false
        }
    }

    pub fn replace(&mut self, items: Vec<ResponseItem>) {
        self.items = items;
        self.last_token_usage_history_len = None;
        self.history_version = self.history_version.saturating_add(1);
    }

    /// Replace image content in the last turn if it originated from a tool output.
    /// Returns true when a tool image was replaced, false otherwise.
    pub fn replace_last_turn_images(&mut self, placeholder: &str) -> bool {
        let Some(index) = self.items.iter().rposition(|item| {
            matches!(item, ResponseItem::FunctionCallOutput { .. }) || is_user_turn_boundary(item)
        }) else {
            return false;
        };

        match &mut self.items[index] {
            ResponseItem::FunctionCallOutput { output, .. } => {
                let Some(content_items) = output.content_items_mut() else {
                    return false;
                };
                let mut replaced = false;
                let placeholder = placeholder.to_string();
                for item in content_items.iter_mut() {
                    if matches!(item, FunctionCallOutputContentItem::InputImage { .. }) {
                        *item = FunctionCallOutputContentItem::InputText {
                            text: placeholder.clone(),
                        };
                        replaced = true;
                    }
                }
                if replaced {
                    self.last_token_usage_history_len = None;
                    self.history_version = self.history_version.saturating_add(1);
                }
                replaced
            }
            ResponseItem::Message { .. }
            | ResponseItem::CommandWait { .. }
            | ResponseItem::CommandWriteStdin { .. }
            | ResponseItem::WorkflowRunProgress { .. }
            | ResponseItem::CommandExecutionNotification { .. }
            | ResponseItem::EventCommandEvent { .. }
            | ResponseItem::EventDrivenTool { .. }
            | ResponseItem::ThreadGoalUpdate { .. }
            | ResponseItem::InterAgentCommunication { .. } => false,
            _ => false,
        }
    }

    /// Drop the last `num_turns` instruction turns from this history.
    ///
    /// Instruction turns are history messages that should behave like a new prompt boundary:
    /// ordinary user messages and structured assistant inter-agent instructions.
    ///
    /// This mirrors thread-rollback semantics:
    /// - `num_turns == 0` is a no-op
    /// - if there are no user turns, this is a no-op
    /// - if `num_turns` exceeds the number of user turns, all user turns are dropped while
    ///   preserving any items that occurred before the first user message.
    ///
    /// If rollback trims a pre-turn developer message that mixes contextual fragments with
    /// persistent developer text from `build_initial_context`, this also clears
    /// `reference_context_item`. The surviving history no longer contains the full bundle that
    /// established the prior baseline, so future turns must fall back to full reinjection instead
    /// of diffing against stale state.
    pub fn drop_last_n_user_turns(&mut self, num_turns: u32) {
        if num_turns == 0 {
            return;
        }

        let snapshot = self.items.clone();
        let user_positions = user_message_positions(&snapshot);
        let Some(&first_instruction_turn_idx) = user_positions.first() else {
            self.replace(snapshot);
            return;
        };

        let n_from_end = usize::try_from(num_turns).unwrap_or(usize::MAX);
        let mut cut_idx = if n_from_end >= user_positions.len() {
            first_instruction_turn_idx
        } else {
            user_positions[user_positions.len() - n_from_end]
        };

        cut_idx =
            self.trim_pre_turn_context_updates(&snapshot, first_instruction_turn_idx, cut_idx);

        let mut retained = snapshot[..cut_idx].to_vec();
        preserve_relevant_model_context_quarantines(&snapshot, &mut retained);
        self.replace(retained);
    }

    pub fn update_token_info(&mut self, usage: &TokenUsage, model_context_window: Option<i64>) {
        self.token_info = TokenUsageInfo::new_or_append(
            &self.token_info,
            &Some(usage.clone()),
            model_context_window,
        );
        self.last_token_usage_history_len = Some(self.items.len());
    }

    fn get_non_last_reasoning_items_tokens(&self) -> i64 {
        // Get reasoning items excluding all the ones after the last instruction boundary.
        let Some(last_user_index) = self.items.iter().rposition(is_user_turn_boundary) else {
            return 0;
        };

        self.items
            .iter()
            .take(last_user_index)
            .filter(|item| {
                matches!(
                    item,
                    ResponseItem::Reasoning {
                        encrypted_content: Some(_),
                        ..
                    }
                )
            })
            .map(estimate_item_token_count)
            .fold(0i64, i64::saturating_add)
    }

    // These are local items added after the most recent model-emitted item.
    // They are not reflected in `last_token_usage.total_tokens`.
    fn token_usage_history_boundary(&self) -> usize {
        self.last_token_usage_history_len.unwrap_or_else(|| {
            self.items
                .iter()
                .rposition(is_model_generated_item)
                .map_or(self.items.len(), |index| index.saturating_add(1))
        })
    }

    #[cfg(test)]
    fn items_after_last_model_generated_item(&self) -> &[ResponseItem] {
        let start = self
            .items
            .iter()
            .rposition(is_model_generated_item)
            .map_or(self.items.len(), |index| index.saturating_add(1));
        &self.items[start..]
    }

    /// When true, the server already accounted for past reasoning tokens and
    /// the client should not re-estimate them.
    pub fn get_total_token_usage(&self, server_reasoning_included: bool) -> i64 {
        let last_tokens = self
            .token_info
            .as_ref()
            .map(|info| info.last_token_usage.total_tokens)
            .unwrap_or(0);
        let start = self.token_usage_history_boundary();
        let items_after_last_model_generated_tokens = self
            .projected_model_input_items()
            .into_iter()
            .filter(|(index, _)| *index >= start)
            .map(|(_, item)| estimate_item_token_count(item.as_ref()))
            .fold(0i64, i64::saturating_add);
        if server_reasoning_included {
            last_tokens.saturating_add(items_after_last_model_generated_tokens)
        } else {
            last_tokens
                .saturating_add(self.get_non_last_reasoning_items_tokens())
                .saturating_add(items_after_last_model_generated_tokens)
        }
    }

    pub fn get_total_token_usage_breakdown(&self) -> TotalTokenUsageBreakdown {
        let last_usage = self
            .token_info
            .as_ref()
            .map(|info| info.last_token_usage.clone())
            .unwrap_or_default();
        let start = self.token_usage_history_boundary();
        let projected_items = self.projected_model_input_items();

        TotalTokenUsageBreakdown {
            last_api_response_total_tokens: last_usage.total_tokens,
            all_history_items_model_visible_bytes: projected_items
                .iter()
                .map(|(_, item)| estimate_response_item_model_visible_bytes(item.as_ref()))
                .fold(0i64, i64::saturating_add),
            estimated_tokens_of_items_added_since_last_successful_api_response: projected_items
                .iter()
                .filter(|(index, _)| *index >= start)
                .map(|(_, item)| estimate_item_token_count(item.as_ref()))
                .fold(0i64, i64::saturating_add),
            estimated_bytes_of_items_added_since_last_successful_api_response: projected_items
                .iter()
                .filter(|(index, _)| *index >= start)
                .map(|(_, item)| estimate_response_item_model_visible_bytes(item.as_ref()))
                .fold(0i64, i64::saturating_add),
        }
    }

    /// This function enforces a couple of invariants on the in-memory history:
    /// 1. every call (function/custom) has a corresponding output entry
    /// 2. every output has a corresponding call entry
    /// 3. when images are unsupported, image content is stripped from messages and tool outputs
    fn normalize_history(&mut self, input_modalities: &[InputModality]) {
        // all function/tool calls must have a corresponding output
        normalize::ensure_call_outputs_present(&mut self.items);

        // all outputs must have a corresponding function/tool call
        normalize::remove_orphan_outputs(&mut self.items);

        // strip images when model does not support them
        normalize::strip_images_when_unsupported(input_modalities, &mut self.items);
    }

    fn process_item(&self, item: &ResponseItem, policy: TruncationPolicy) -> ResponseItem {
        let policy_with_serialization_budget = policy * 1.2;
        match item {
            ResponseItem::FunctionCallOutput { call_id, output } => {
                ResponseItem::FunctionCallOutput {
                    call_id: call_id.clone(),
                    output: truncate_function_output_payload(
                        output,
                        policy_with_serialization_budget,
                    ),
                }
            }
            ResponseItem::CustomToolCallOutput {
                call_id,
                name,
                output,
            } => ResponseItem::CustomToolCallOutput {
                call_id: call_id.clone(),
                name: name.clone(),
                output: truncate_function_output_payload(output, policy_with_serialization_budget),
            },
            ResponseItem::Message { .. }
            | ResponseItem::CommandWait { .. }
            | ResponseItem::CommandWriteStdin { .. }
            | ResponseItem::WorkflowRunProgress { .. }
            | ResponseItem::CommandExecutionNotification { .. }
            | ResponseItem::EventCommandEvent { .. }
            | ResponseItem::EventDrivenTool { .. }
            | ResponseItem::ThreadGoalUpdate { .. }
            | ResponseItem::InterAgentCommunication { .. }
            | ResponseItem::ModelContextQuarantine { .. }
            | ResponseItem::Reasoning { .. }
            | ResponseItem::LocalShellCall { .. }
            | ResponseItem::FunctionCall { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::CustomToolCall { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::Other => item.clone(),
        }
    }

    /// Walk backward from a rollback cut and trim contiguous pre-turn context-update items.
    ///
    /// Returns the adjusted cut index after removing contextual developer/user items immediately
    /// above the rolled-back turn boundary.
    ///
    /// `first_instruction_turn_idx` is the earliest rollback-eligible instruction-turn boundary
    /// in `snapshot`; the trim walk never crosses it so any session-prefix items that predate the
    /// first real turn survive rollback.
    ///
    /// `cut_idx` is the tentative slice boundary after dropping the requested number of
    /// instruction turns, before stripping contextual pre-turn items that sit immediately above
    /// that boundary.
    ///
    /// If any trimmed developer message was a mixed `build_initial_context` bundle containing both
    /// rollback-trimmable contextual fragments and persistent developer text, this also clears the
    /// stored `reference_context_item` baseline so the next real turn falls back to full
    /// reinjection.
    fn trim_pre_turn_context_updates(
        &mut self,
        snapshot: &[ResponseItem],
        first_instruction_turn_idx: usize,
        mut cut_idx: usize,
    ) -> usize {
        while cut_idx > first_instruction_turn_idx {
            match &snapshot[cut_idx - 1] {
                ResponseItem::Message { role, content, .. }
                    if role == "developer" && is_contextual_dev_message_content(content) =>
                {
                    if has_non_contextual_dev_message_content(content) {
                        // Mixed `build_initial_context` bundles are not reconstructible from
                        // steady-state diffs once trimmed, so the next real turn must fully
                        // reinject context instead of diffing against a stale baseline.
                        self.reference_context_item = None;
                    }
                    cut_idx -= 1;
                }
                ResponseItem::Message { role, content, .. }
                    if role == "user" && is_contextual_user_message_content(content) =>
                {
                    cut_idx -= 1;
                }
                _ => break,
            }
        }
        cut_idx
    }
}

pub fn truncate_function_output_payload(
    output: &FunctionCallOutputPayload,
    policy: TruncationPolicy,
) -> FunctionCallOutputPayload {
    let body = match &output.body {
        FunctionCallOutputBody::Text(content) => {
            FunctionCallOutputBody::Text(truncate_text(content, policy))
        }
        FunctionCallOutputBody::ContentItems(items) => FunctionCallOutputBody::ContentItems(
            truncate_function_output_items_with_policy(items, policy),
        ),
    };

    FunctionCallOutputPayload {
        body,
        success: output.success,
    }
}

/// API messages include every non-system item (user/assistant messages, reasoning,
/// tool calls, tool outputs, shell calls, web-search calls, and image-generation
/// calls).
fn is_api_message(message: &ResponseItem) -> bool {
    match message {
        ResponseItem::Message { role, .. } => role.as_str() != "system",
        ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::ThreadGoalUpdate { .. }
        | ResponseItem::InterAgentCommunication { .. } => true,
        ResponseItem::ModelContextQuarantine { .. } => true,
        ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. } => true,
        ResponseItem::WorkflowRunProgress { .. } | ResponseItem::Other => false,
    }
}

fn estimate_reasoning_length(encoded_len: usize) -> usize {
    encoded_len
        .saturating_mul(3)
        .checked_div(4)
        .unwrap_or(0)
        .saturating_sub(650)
}

fn estimate_item_token_count(item: &ResponseItem) -> i64 {
    let model_visible_bytes = estimate_response_item_model_visible_bytes(item);
    approx_tokens_from_byte_count_i64(model_visible_bytes)
}

/// Approximate model-visible byte cost for one image input.
///
/// The estimator later converts bytes to tokens using a 4-bytes/token heuristic
/// with ceiling division, so 7,373 bytes maps to approximately 1,844 tokens.
const RESIZED_IMAGE_BYTES_ESTIMATE: i64 = 7373;
// `codex-utils-image` owns original-detail patch counting; all other image
// inputs continue to use this fixed byte estimate.

pub fn estimate_response_item_model_visible_bytes(item: &ResponseItem) -> i64 {
    match item {
        ResponseItem::Reasoning {
            encrypted_content: Some(content),
            ..
        }
        | ResponseItem::Compaction {
            encrypted_content: content,
        }
        | ResponseItem::ContextCompaction {
            encrypted_content: Some(content),
        } => i64::try_from(estimate_reasoning_length(content.len())).unwrap_or(i64::MAX),
        item => {
            let raw = serde_json::to_string(item)
                .map(|serialized| i64::try_from(serialized.len()).unwrap_or(i64::MAX))
                .unwrap_or_default();
            let (payload_bytes, replacement_bytes) = image_data_url_estimate_adjustment(item);
            if payload_bytes == 0 || replacement_bytes == 0 {
                raw
            } else {
                // Replace raw base64 payload bytes with a per-image estimate.
                // We intentionally preserve the data URL prefix and JSON
                // wrapper bytes already included in `raw`.
                raw.saturating_sub(payload_bytes)
                    .saturating_add(replacement_bytes)
            }
        }
    }
}

/// Scans one response item for discount-eligible inline image data URLs and
/// returns:
/// - total base64 payload bytes to subtract from raw serialized size
/// - total replacement byte estimate for those images
fn image_data_url_estimate_adjustment(item: &ResponseItem) -> (i64, i64) {
    let mut payload_bytes = 0i64;
    let mut replacement_bytes = 0i64;

    let mut accumulate = |image_url: &str, detail: Option<ImageDetail>| {
        if let Some(payload_len) = base64_image_data_url_payload(image_url).map(str::len) {
            payload_bytes =
                payload_bytes.saturating_add(i64::try_from(payload_len).unwrap_or(i64::MAX));
            replacement_bytes = replacement_bytes.saturating_add(match detail {
                Some(ImageDetail::Original) => estimate_original_image_data_url_bytes(image_url)
                    .unwrap_or(RESIZED_IMAGE_BYTES_ESTIMATE),
                _ => RESIZED_IMAGE_BYTES_ESTIMATE,
            });
        }
    };

    match item {
        ResponseItem::Message { content, .. } => {
            for content_item in content {
                if let ContentItem::InputImage { image_url, detail } = content_item {
                    accumulate(image_url, *detail);
                }
            }
        }
        ResponseItem::FunctionCallOutput { output, .. }
        | ResponseItem::CustomToolCallOutput { output, .. } => {
            if let FunctionCallOutputBody::ContentItems(items) = &output.body {
                for content_item in items {
                    if let FunctionCallOutputContentItem::InputImage { image_url, detail } =
                        content_item
                    {
                        accumulate(image_url, *detail);
                    }
                }
            }
        }
        _ => {}
    }

    (payload_bytes, replacement_bytes)
}

fn is_model_generated_item(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { role, .. } => role == "assistant",
        ResponseItem::Reasoning { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. } => true,
        ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::WorkflowRunProgress { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::ThreadGoalUpdate { .. }
        | ResponseItem::InterAgentCommunication { .. }
        | ResponseItem::ModelContextQuarantine { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::Other => false,
    }
}

pub fn is_codex_generated_item(item: &ResponseItem) -> bool {
    matches!(
        item,
        ResponseItem::FunctionCallOutput { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::CustomToolCallOutput { .. }
            | ResponseItem::ModelContextQuarantine { .. }
    ) || matches!(item, ResponseItem::Message { role, .. } if role == "developer")
}

pub fn preserve_relevant_model_context_quarantines(
    previous_items: &[ResponseItem],
    retained_items: &mut Vec<ResponseItem>,
) {
    for item in previous_items {
        let ResponseItem::ModelContextQuarantine { target, .. } = item else {
            continue;
        };
        let target_survives = retained_items
            .iter()
            .any(|retained| response_item_belongs_to_quarantine_target(retained, target));
        let marker_survives = retained_items.iter().any(|retained| {
            matches!(
                retained,
                ResponseItem::ModelContextQuarantine {
                    target: retained_target,
                    ..
                } if retained_target == target
            )
        });
        if target_survives && !marker_survives {
            retained_items.push(item.clone());
        }
    }
}

fn response_item_belongs_to_quarantine_target(
    item: &ResponseItem,
    target: &ModelContextQuarantineReference,
) -> bool {
    match target {
        ModelContextQuarantineReference::ToolTransaction { kind, call_id } => {
            response_item_belongs_to_target(
                item,
                &ModelInputItemReference {
                    kind: *kind,
                    call_id: call_id.clone(),
                },
            )
        }
        ModelContextQuarantineReference::ModelItem {
            kind, fingerprint, ..
        } => {
            response_item_kind(item) == Some(*kind)
                && model_context_item_fingerprint(item).as_deref() == Some(fingerprint.as_str())
        }
    }
}

fn response_item_kind(item: &ResponseItem) -> Option<ModelInputItemKind> {
    match item {
        ResponseItem::FunctionCall { .. } => Some(ModelInputItemKind::FunctionCall),
        ResponseItem::ToolSearchCall { .. } => Some(ModelInputItemKind::ToolSearchCall),
        ResponseItem::CustomToolCall { .. } => Some(ModelInputItemKind::CustomToolCall),
        ResponseItem::LocalShellCall { .. } => Some(ModelInputItemKind::LocalShellCall),
        _ => None,
    }
}

fn response_item_matches_target(item: &ResponseItem, target: &ModelInputItemReference) -> bool {
    match (target.kind, item) {
        (ModelInputItemKind::FunctionCall, ResponseItem::FunctionCall { call_id, .. })
        | (ModelInputItemKind::CustomToolCall, ResponseItem::CustomToolCall { call_id, .. }) => {
            call_id == &target.call_id
        }
        (
            ModelInputItemKind::ToolSearchCall,
            ResponseItem::ToolSearchCall {
                call_id: Some(call_id),
                ..
            },
        )
        | (
            ModelInputItemKind::LocalShellCall,
            ResponseItem::LocalShellCall {
                call_id: Some(call_id),
                ..
            },
        ) => call_id == &target.call_id,
        _ => false,
    }
}

fn response_item_belongs_to_target(item: &ResponseItem, target: &ModelInputItemReference) -> bool {
    if response_item_matches_target(item, target) {
        return true;
    }
    response_item_is_output_for_target(item, target)
}

fn response_item_is_output_for_target(
    item: &ResponseItem,
    target: &ModelInputItemReference,
) -> bool {
    match (target.kind, item) {
        (
            ModelInputItemKind::FunctionCall | ModelInputItemKind::LocalShellCall,
            ResponseItem::FunctionCallOutput { call_id, .. },
        )
        | (
            ModelInputItemKind::CustomToolCall,
            ResponseItem::CustomToolCallOutput { call_id, .. },
        ) => call_id == &target.call_id,
        (
            ModelInputItemKind::ToolSearchCall,
            ResponseItem::ToolSearchOutput {
                call_id: Some(call_id),
                ..
            },
        ) => call_id == &target.call_id,
        _ => false,
    }
}

fn model_call_id(item: &ResponseItem) -> Option<&str> {
    match item {
        ResponseItem::FunctionCall { call_id, .. }
        | ResponseItem::CustomToolCall { call_id, .. } => Some(call_id),
        ResponseItem::ToolSearchCall {
            call_id: Some(call_id),
            ..
        }
        | ResponseItem::LocalShellCall {
            call_id: Some(call_id),
            ..
        } => Some(call_id),
        _ => None,
    }
}

fn model_output_call_id(item: &ResponseItem) -> Option<&str> {
    match item {
        ResponseItem::FunctionCallOutput { call_id, .. }
        | ResponseItem::CustomToolCallOutput { call_id, .. } => Some(call_id),
        ResponseItem::ToolSearchOutput {
            call_id: Some(call_id),
            ..
        } => Some(call_id),
        _ => None,
    }
}

pub fn is_user_turn_boundary(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::ThreadGoalUpdate { .. } => {
            return true;
        }
        ResponseItem::InterAgentCommunication { communication, .. } => {
            return communication.trigger_turn;
        }
        _ => {}
    }

    let ResponseItem::Message { role, content, .. } = item else {
        return false;
    };

    role == "user" && !is_contextual_user_message_content(content)
}

pub fn is_real_user_message_boundary(item: &ResponseItem) -> bool {
    let ResponseItem::Message { role, content, .. } = item else {
        return false;
    };

    role == "user" && !is_contextual_user_message_content(content)
}

fn user_message_positions(items: &[ResponseItem]) -> Vec<usize> {
    let mut positions = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if is_user_turn_boundary(item) {
            positions.push(idx);
        }
    }
    positions
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
