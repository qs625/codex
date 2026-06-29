mod discoverable;
mod injection;
mod mentions;
mod render;
mod request_plugin_install_service;
#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use codex_plugin_types::PluginCapabilitySummary;
pub use request_plugin_install_service::RequestPluginInstallService;

pub(crate) use discoverable::list_tool_suggest_discoverable_plugins;
pub(crate) use injection::build_plugin_injections;
pub(crate) use render::render_explicit_plugin_instructions;

pub(crate) use mentions::build_connector_slug_counts;
pub(crate) use mentions::build_skill_name_counts;
pub(crate) use mentions::collect_explicit_app_ids;
pub(crate) use mentions::collect_explicit_plugin_mentions;
