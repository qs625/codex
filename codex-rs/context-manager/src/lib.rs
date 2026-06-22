mod contextual_user_message;
mod fragment;
mod history;
mod normalize;

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

pub(crate) fn error_or_panic(message: impl std::string::ToString) {
    if cfg!(debug_assertions) {
        panic!("{}", message.to_string());
    } else {
        tracing::error!("{}", message.to_string());
    }
}
