pub use codex_tool_config::can_request_original_image_detail;
pub use codex_tool_config::normalize_output_image_detail;
pub use codex_tool_config::sanitize_original_image_detail;

#[cfg(test)]
#[path = "image_detail_tests.rs"]
mod tests;
