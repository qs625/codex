use super::*;

/// Resolve the web search mode from explicit config and feature flags.
pub(super) fn resolve_web_search_mode(
    config_toml: &ConfigToml,
    config_profile: &ConfigProfile,
    features: &Features,
) -> Option<WebSearchMode> {
    if let Some(mode) = config_profile.web_search.or(config_toml.web_search) {
        return Some(mode);
    }
    if features.enabled(Feature::WebSearchCached) {
        return Some(WebSearchMode::Cached);
    }
    if features.enabled(Feature::WebSearchRequest) {
        return Some(WebSearchMode::Live);
    }
    None
}

pub(super) fn resolve_web_search_config(
    config_toml: &ConfigToml,
    config_profile: &ConfigProfile,
) -> Option<WebSearchConfig> {
    let base = config_toml
        .tools
        .as_ref()
        .and_then(|tools| tools.web_search.as_ref());
    let profile = config_profile
        .tools
        .as_ref()
        .and_then(|tools| tools.web_search.as_ref());

    match (base, profile) {
        (None, None) => None,
        (Some(base), None) => Some(base.clone().into()),
        (None, Some(profile)) => Some(profile.clone().into()),
        (Some(base), Some(profile)) => Some(base.merge(profile).into()),
    }
}

pub(super) fn resolve_multi_agent_v2_config(
    config_toml: &ConfigToml,
    config_profile: &ConfigProfile,
) -> MultiAgentV2Config {
    let base = multi_agent_v2_toml_config(config_toml.features.as_ref());
    let profile = multi_agent_v2_toml_config(config_profile.features.as_ref());
    let default = MultiAgentV2Config::default();

    let max_concurrent_threads_per_session = profile
        .and_then(|config| config.max_concurrent_threads_per_session)
        .or_else(|| base.and_then(|config| config.max_concurrent_threads_per_session))
        .unwrap_or(default.max_concurrent_threads_per_session);
    let min_wait_timeout_ms = profile
        .and_then(|config| config.min_wait_timeout_ms)
        .or_else(|| base.and_then(|config| config.min_wait_timeout_ms))
        .unwrap_or(default.min_wait_timeout_ms);
    let max_wait_timeout_ms = profile
        .and_then(|config| config.max_wait_timeout_ms)
        .or_else(|| base.and_then(|config| config.max_wait_timeout_ms))
        .unwrap_or(default.max_wait_timeout_ms);
    let default_wait_timeout_ms = profile
        .and_then(|config| config.default_wait_timeout_ms)
        .or_else(|| base.and_then(|config| config.default_wait_timeout_ms))
        .unwrap_or(default.default_wait_timeout_ms);
    let usage_hint_enabled = profile
        .and_then(|config| config.usage_hint_enabled)
        .or_else(|| base.and_then(|config| config.usage_hint_enabled))
        .unwrap_or(default.usage_hint_enabled);
    let usage_hint_text = profile
        .and_then(|config| config.usage_hint_text.as_ref())
        .or_else(|| base.and_then(|config| config.usage_hint_text.as_ref()))
        .cloned()
        .or(default.usage_hint_text);
    let root_agent_usage_hint_text = profile
        .and_then(|config| config.root_agent_usage_hint_text.as_ref())
        .or_else(|| base.and_then(|config| config.root_agent_usage_hint_text.as_ref()))
        .cloned()
        .or(default.root_agent_usage_hint_text);
    let subagent_usage_hint_text = profile
        .and_then(|config| config.subagent_usage_hint_text.as_ref())
        .or_else(|| base.and_then(|config| config.subagent_usage_hint_text.as_ref()))
        .cloned()
        .or(default.subagent_usage_hint_text);
    let hide_spawn_agent_metadata = profile
        .and_then(|config| config.hide_spawn_agent_metadata)
        .or_else(|| base.and_then(|config| config.hide_spawn_agent_metadata))
        .unwrap_or(default.hide_spawn_agent_metadata);
    let non_code_mode_only = profile
        .and_then(|config| config.non_code_mode_only)
        .or_else(|| base.and_then(|config| config.non_code_mode_only))
        .unwrap_or(default.non_code_mode_only);

    MultiAgentV2Config {
        max_concurrent_threads_per_session,
        min_wait_timeout_ms,
        max_wait_timeout_ms,
        default_wait_timeout_ms,
        usage_hint_enabled,
        usage_hint_text,
        root_agent_usage_hint_text,
        subagent_usage_hint_text,
        hide_spawn_agent_metadata,
        non_code_mode_only,
    }
}

pub(super) fn resolve_terminal_resize_reflow_config(
    config_toml: &ConfigToml,
) -> TerminalResizeReflowConfig {
    let Some(tui) = config_toml.tui.as_ref() else {
        return TerminalResizeReflowConfig::default();
    };

    TerminalResizeReflowConfig {
        max_rows: match tui.terminal_resize_reflow_max_rows {
            Some(0) => TerminalResizeReflowMaxRows::Disabled,
            Some(rows) => TerminalResizeReflowMaxRows::Limit(rows),
            None => TerminalResizeReflowMaxRows::Auto,
        },
    }
}

fn multi_agent_v2_toml_config(features: Option<&FeaturesToml>) -> Option<&MultiAgentV2ConfigToml> {
    match features?.multi_agent_v2.as_ref()? {
        FeatureToml::Enabled(_) => None,
        FeatureToml::Config(config) => Some(config),
    }
}

pub(super) fn apps_mcp_path_override_toml_config(
    features: Option<&FeaturesToml>,
) -> Option<&AppsMcpPathOverrideConfigToml> {
    match features?.apps_mcp_path_override.as_ref()? {
        FeatureToml::Enabled(_) => None,
        FeatureToml::Config(config) => Some(config),
    }
}

pub(super) fn network_proxy_toml_config(
    features: Option<&FeaturesToml>,
) -> Option<&NetworkProxyConfigToml> {
    match features?.network_proxy.as_ref()? {
        FeatureToml::Enabled(_) => None,
        FeatureToml::Config(config) => Some(config),
    }
}

pub fn resolve_web_search_mode_for_turn(
    web_search_mode: &Constrained<WebSearchMode>,
    permission_profile: &PermissionProfile,
) -> WebSearchMode {
    let preferred = web_search_mode.value();

    if matches!(permission_profile, PermissionProfile::Disabled)
        && preferred != WebSearchMode::Disabled
    {
        for mode in [
            WebSearchMode::Live,
            WebSearchMode::Cached,
            WebSearchMode::Disabled,
        ] {
            if web_search_mode.can_set(&mode).is_ok() {
                return mode;
            }
        }
    } else {
        if web_search_mode.can_set(&preferred).is_ok() {
            return preferred;
        }
        for mode in [
            WebSearchMode::Cached,
            WebSearchMode::Live,
            WebSearchMode::Disabled,
        ] {
            if web_search_mode.can_set(&mode).is_ok() {
                return mode;
            }
        }
    }

    WebSearchMode::Disabled
}

pub(super) fn validate_multi_agent_v2_wait_timeout(label: &str, value: i64) -> std::io::Result<()> {
    if value < HARD_MIN_MULTI_AGENT_V2_TIMEOUT_MS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{label} must be at least {HARD_MIN_MULTI_AGENT_V2_TIMEOUT_MS}"),
        ));
    }
    if value > HARD_MAX_MULTI_AGENT_V2_TIMEOUT_MS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{label} must be at most {HARD_MAX_MULTI_AGENT_V2_TIMEOUT_MS}"),
        ));
    }
    Ok(())
}
