use crate::config::NetworkProxyConfig;
use crate::mitm::MitmState;
use crate::mitm::MitmUpstreamConfig;
use crate::policy::compile_allowlist_globset;
use crate::policy::compile_denylist_globset;
use crate::policy::is_global_wildcard_domain_pattern;
use crate::runtime::ConfigState;
use codex_network_proxy_api::NetworkProxyConstraintError;
use codex_network_proxy_api::NetworkProxyConstraints;
use std::sync::Arc;

pub use crate::runtime::BlockedRequest;
pub use crate::runtime::BlockedRequestArgs;
pub use crate::runtime::NetworkProxyAuditMetadata;
pub use crate::runtime::NetworkProxyState;
#[cfg(test)]
pub(crate) use crate::runtime::network_proxy_state_for_policy;

pub fn build_config_state(
    config: NetworkProxyConfig,
    constraints: NetworkProxyConstraints,
) -> anyhow::Result<ConfigState> {
    crate::config::validate_unix_socket_allowlist_paths(&config)?;
    let allowed_domains = config.network.allowed_domains().unwrap_or_default();
    let denied_domains = config.network.denied_domains().unwrap_or_default();
    validate_non_global_wildcard_domain_patterns("network.denied_domains", &denied_domains)
        .map_err(anyhow::Error::from)?;
    let deny_set = compile_denylist_globset(&denied_domains)?;
    let allow_set = compile_allowlist_globset(&allowed_domains)?;
    let mitm = if config.network.mitm {
        Some(Arc::new(MitmState::new(MitmUpstreamConfig {
            allow_upstream_proxy: config.network.allow_upstream_proxy,
            allow_local_binding: config.network.allow_local_binding,
        })?))
    } else {
        None
    };
    Ok(ConfigState {
        config,
        allow_set,
        deny_set,
        mitm,
        constraints,
        blocked: std::collections::VecDeque::new(),
        blocked_total: 0,
    })
}

fn validate_non_global_wildcard_domain_patterns(
    field_name: &'static str,
    patterns: &[String],
) -> Result<(), NetworkProxyConstraintError> {
    if let Some(pattern) = patterns
        .iter()
        .find(|pattern| is_global_wildcard_domain_pattern(pattern))
    {
        return Err(NetworkProxyConstraintError::InvalidValue {
            field_name,
            candidate: pattern.trim().to_string(),
            allowed: "exact hosts or scoped wildcards like *.example.com or **.example.com"
                .to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {}
