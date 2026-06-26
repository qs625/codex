#[cfg(test)]
pub(crate) use codex_agent_runtime::AGENT_TYPE_UNAVAILABLE_ERROR;
pub(crate) use codex_agent_runtime::apply_role_to_config;

#[cfg(test)]
#[path = "role_tests.rs"]
mod tests;
