mod compact_history;
mod contextual_user_message;
mod fragment;
mod history;
mod instructions;
mod normalize;
mod updates;

pub use compact_history::COMPACT_USER_MESSAGE_MAX_TOKENS;
pub use compact_history::build_compacted_history;
pub use compact_history::build_compacted_history_with_limit;
pub use compact_history::collect_compaction_user_messages;
pub use compact_history::content_items_to_text;
pub use compact_history::insert_initial_context_before_last_real_user_or_summary;
pub use compact_history::is_compaction_summary_message;
pub use compact_history::is_legacy_compaction_warning_message;
pub use contextual_user_message::has_non_contextual_dev_message_content;
pub use contextual_user_message::is_contextual_dev_message_content;
pub use contextual_user_message::is_contextual_user_fragment;
pub use contextual_user_message::is_contextual_user_message_content;
pub use contextual_user_message::parse_visible_hook_prompt_message;
pub use fragment::ContextualUserFragment;
pub use history::ContextManager;
pub use history::TotalTokenUsageBreakdown;
pub use history::estimate_response_item_model_visible_bytes;
pub use history::is_codex_generated_item;
pub use history::is_real_user_message_boundary;
pub use history::is_user_turn_boundary;
pub use history::truncate_function_output_payload;
pub use instructions::ApprovedCommandPrefixSaved;
pub use instructions::AppsInstructions;
pub use instructions::AvailableAgentsInstructions;
pub use instructions::AvailablePluginsInstructions;
pub use instructions::AvailableSkillsInstructions;
pub use instructions::AvailableWorkflowsInstructions;
pub use instructions::CollaborationModeInstructions;
pub use instructions::EnvironmentContext;
pub use instructions::EnvironmentContextEnvironment;
pub use instructions::EnvironmentContextEnvironments;
pub use instructions::GuardianFollowupReviewReminder;
pub use instructions::HookAdditionalContext;
pub use instructions::ImageGenerationInstructions;
pub use instructions::ModelSwitchInstructions;
pub use instructions::MultiagentContext;
pub use instructions::NetworkContext;
pub use instructions::NetworkRuleSaved;
pub use instructions::PermissionsInstructions;
pub use instructions::PersonalitySpecInstructions;
pub use instructions::PluginInstructions;
pub use instructions::RealtimeEndInstructions;
pub use instructions::RealtimeStartInstructions;
pub use instructions::RealtimeStartWithInstructions;
pub use instructions::SkillInstructions;
pub use instructions::SubagentNotification;
pub use instructions::UserInstructions;
pub use instructions::UserShellCommand;
pub use updates::PreviousTurnSettingsView;
pub use updates::SettingsUpdateInput;
pub use updates::build_contextual_user_message;
pub use updates::build_developer_update_item;
pub use updates::build_initial_realtime_item;
pub use updates::build_model_instructions_update_item;
pub use updates::build_realtime_update_item;
pub use updates::build_settings_update_items;
pub use updates::personality_message_for;

pub(crate) fn error_or_panic(message: impl std::string::ToString) {
    if cfg!(debug_assertions) {
        panic!("{}", message.to_string());
    } else {
        tracing::error!("{}", message.to_string());
    }
}
