use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

pub const PROXY_ACTIVE_ENV_KEY: &str = "CODEX_NETWORK_PROXY_ACTIVE";
pub const ALLOW_LOCAL_BINDING_ENV_KEY: &str = "CODEX_NETWORK_ALLOW_LOCAL_BINDING";
pub const ELECTRON_GET_USE_PROXY_ENV_KEY: &str = "ELECTRON_GET_USE_PROXY";
pub const PROXY_ENV_KEYS: &[&str] = &[
    PROXY_ACTIVE_ENV_KEY,
    ALLOW_LOCAL_BINDING_ENV_KEY,
    ELECTRON_GET_USE_PROXY_ENV_KEY,
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "http_proxy",
    "https_proxy",
    "YARN_HTTP_PROXY",
    "YARN_HTTPS_PROXY",
    "npm_config_http_proxy",
    "npm_config_https_proxy",
    "npm_config_proxy",
    "NPM_CONFIG_HTTP_PROXY",
    "NPM_CONFIG_HTTPS_PROXY",
    "NPM_CONFIG_PROXY",
    "BUNDLE_HTTP_PROXY",
    "BUNDLE_HTTPS_PROXY",
    "PIP_PROXY",
    "DOCKER_HTTP_PROXY",
    "DOCKER_HTTPS_PROXY",
    "WS_PROXY",
    "WSS_PROXY",
    "ws_proxy",
    "wss_proxy",
    "NO_PROXY",
    "no_proxy",
    "npm_config_noproxy",
    "NPM_CONFIG_NOPROXY",
    "YARN_NO_PROXY",
    "BUNDLE_NO_PROXY",
    "ALL_PROXY",
    "all_proxy",
    "FTP_PROXY",
    "ftp_proxy",
];

pub const PROXY_URL_ENV_KEYS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "WS_PROXY",
    "WSS_PROXY",
    "ALL_PROXY",
    "FTP_PROXY",
    "YARN_HTTP_PROXY",
    "YARN_HTTPS_PROXY",
    "NPM_CONFIG_HTTP_PROXY",
    "NPM_CONFIG_HTTPS_PROXY",
    "NPM_CONFIG_PROXY",
    "BUNDLE_HTTP_PROXY",
    "BUNDLE_HTTPS_PROXY",
    "PIP_PROXY",
    "DOCKER_HTTP_PROXY",
    "DOCKER_HTTPS_PROXY",
];

pub const ALL_PROXY_ENV_KEYS: &[&str] = &["ALL_PROXY", "all_proxy"];

const FTP_PROXY_ENV_KEYS: &[&str] = &["FTP_PROXY", "ftp_proxy"];
const WEBSOCKET_PROXY_ENV_KEYS: &[&str] = &["WS_PROXY", "WSS_PROXY", "ws_proxy", "wss_proxy"];

pub const NO_PROXY_ENV_KEYS: &[&str] = &[
    "NO_PROXY",
    "no_proxy",
    "npm_config_noproxy",
    "NPM_CONFIG_NOPROXY",
    "YARN_NO_PROXY",
    "BUNDLE_NO_PROXY",
];

pub const DEFAULT_NO_PROXY_VALUE: &str = concat!(
    "localhost,127.0.0.1,::1,",
    "10.0.0.0/8,",
    "172.16.0.0/12,",
    "192.168.0.0/16"
);

#[cfg(target_os = "macos")]
pub const PROXY_GIT_SSH_COMMAND_ENV_KEY: &str = "GIT_SSH_COMMAND";

#[cfg(target_os = "macos")]
pub const CODEX_PROXY_GIT_SSH_COMMAND_MARKER: &str = "CODEX_PROXY_GIT_SSH_COMMAND=1 ";

#[cfg(target_os = "macos")]
const CODEX_PROXY_GIT_SSH_COMMAND_PREFIX: &str =
    "CODEX_PROXY_GIT_SSH_COMMAND=1 ssh -o ProxyCommand='nc -X 5 -x ";
#[cfg(target_os = "macos")]
const CODEX_PROXY_GIT_SSH_COMMAND_SUFFIX: &str = " %h %p'";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicyDecision {
    Deny,
    Ask,
}

impl NetworkPolicyDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Ask => "ask",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDecisionSource {
    BaselinePolicy,
    ModeGuard,
    ProxyState,
    Decider,
}

impl NetworkDecisionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BaselinePolicy => "baseline_policy",
            Self::ModeGuard => "mode_guard",
            Self::ProxyState => "proxy_state",
            Self::Decider => "decider",
        }
    }
}

const DEFAULT_NETWORK_POLICY_DENIED_REASON: &str = "policy_denied";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkProtocol {
    Http,
    HttpsConnect,
    Socks5Tcp,
    Socks5Udp,
}

impl NetworkProtocol {
    pub const fn as_policy_protocol(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::HttpsConnect => "https_connect",
            Self::Socks5Tcp => "socks5_tcp",
            Self::Socks5Udp => "socks5_udp",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NetworkPolicyRequest {
    pub protocol: NetworkProtocol,
    pub host: String,
    pub port: u16,
    pub client_addr: Option<String>,
    pub method: Option<String>,
    pub command: Option<String>,
    pub exec_policy_hint: Option<String>,
}

pub struct NetworkPolicyRequestArgs {
    pub protocol: NetworkProtocol,
    pub host: String,
    pub port: u16,
    pub client_addr: Option<String>,
    pub method: Option<String>,
    pub command: Option<String>,
    pub exec_policy_hint: Option<String>,
}

impl NetworkPolicyRequest {
    pub fn new(args: NetworkPolicyRequestArgs) -> Self {
        let NetworkPolicyRequestArgs {
            protocol,
            host,
            port,
            client_addr,
            method,
            command,
            exec_policy_hint,
        } = args;
        Self {
            protocol,
            host,
            port,
            client_addr,
            method,
            command,
            exec_policy_hint,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkDecision {
    Allow,
    Deny {
        reason: String,
        source: NetworkDecisionSource,
        decision: NetworkPolicyDecision,
    },
}

impl NetworkDecision {
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::deny_with_source(reason, NetworkDecisionSource::Decider)
    }

    pub fn ask(reason: impl Into<String>) -> Self {
        Self::ask_with_source(reason, NetworkDecisionSource::Decider)
    }

    pub fn deny_with_source(reason: impl Into<String>, source: NetworkDecisionSource) -> Self {
        let reason = reason.into();
        let reason = if reason.is_empty() {
            DEFAULT_NETWORK_POLICY_DENIED_REASON.to_string()
        } else {
            reason
        };
        Self::Deny {
            reason,
            source,
            decision: NetworkPolicyDecision::Deny,
        }
    }

    pub fn ask_with_source(reason: impl Into<String>, source: NetworkDecisionSource) -> Self {
        let reason = reason.into();
        let reason = if reason.is_empty() {
            DEFAULT_NETWORK_POLICY_DENIED_REASON.to_string()
        } else {
            reason
        };
        Self::Deny {
            reason,
            source,
            decision: NetworkPolicyDecision::Ask,
        }
    }
}

/// Host-provided network policy decision hook used by the proxy runtime.
///
/// Implementations should return a decision for one outbound network request without blocking on
/// unrelated runtime state. The boxed-future shape keeps the trait object-safe for runtime service
/// registries and proxy builders.
pub trait NetworkPolicyDecider: Send + Sync + 'static {
    fn decide(
        &self,
        req: NetworkPolicyRequest,
    ) -> Pin<Box<dyn Future<Output = NetworkDecision> + Send + '_>>;
}

impl<D: NetworkPolicyDecider + ?Sized> NetworkPolicyDecider for Arc<D> {
    fn decide(
        &self,
        req: NetworkPolicyRequest,
    ) -> Pin<Box<dyn Future<Output = NetworkDecision> + Send + '_>> {
        (**self).decide(req)
    }
}

impl<F, Fut> NetworkPolicyDecider for F
where
    F: Fn(NetworkPolicyRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = NetworkDecision> + Send + 'static,
{
    fn decide(
        &self,
        req: NetworkPolicyRequest,
    ) -> Pin<Box<dyn Future<Output = NetworkDecision> + Send + '_>> {
        Box::pin((self)(req))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NetworkProxyConfig {
    #[serde(default)]
    pub network: NetworkProxySettings,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NetworkProxyConstraints {
    pub enabled: Option<bool>,
    pub mode: Option<NetworkMode>,
    pub allow_upstream_proxy: Option<bool>,
    pub dangerously_allow_non_loopback_proxy: Option<bool>,
    pub dangerously_allow_all_unix_sockets: Option<bool>,
    pub allowed_domains: Option<Vec<String>>,
    pub allowlist_expansion_enabled: Option<bool>,
    pub denied_domains: Option<Vec<String>>,
    pub denylist_expansion_enabled: Option<bool>,
    pub allow_unix_sockets: Option<Vec<String>>,
    pub allow_local_binding: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PartialNetworkProxyConfig {
    #[serde(default)]
    pub network: PartialNetworkConfig,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct PartialNetworkConfig {
    pub enabled: Option<bool>,
    pub mode: Option<NetworkMode>,
    pub allow_upstream_proxy: Option<bool>,
    pub dangerously_allow_non_loopback_proxy: Option<bool>,
    pub dangerously_allow_all_unix_sockets: Option<bool>,
    #[serde(default)]
    pub domains: Option<NetworkDomainPermissions>,
    #[serde(default)]
    pub unix_sockets: Option<NetworkUnixSocketPermissions>,
    pub allow_local_binding: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkProxyConstraintError {
    InvalidValue {
        field_name: &'static str,
        candidate: String,
        allowed: String,
    },
}

impl fmt::Display for NetworkProxyConstraintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue {
                field_name,
                candidate,
                allowed,
            } => write!(
                f,
                "invalid value for {field_name}: {candidate} (allowed {allowed})"
            ),
        }
    }
}

impl Error for NetworkProxyConstraintError {}

pub fn validate_policy_against_constraints(
    config: &NetworkProxyConfig,
    constraints: &NetworkProxyConstraints,
) -> Result<(), NetworkProxyConstraintError> {
    fn invalid_value(
        field_name: &'static str,
        candidate: impl Into<String>,
        allowed: impl Into<String>,
    ) -> NetworkProxyConstraintError {
        NetworkProxyConstraintError::InvalidValue {
            field_name,
            candidate: candidate.into(),
            allowed: allowed.into(),
        }
    }

    fn validate<T>(
        candidate: T,
        validator: impl FnOnce(&T) -> Result<(), NetworkProxyConstraintError>,
    ) -> Result<(), NetworkProxyConstraintError> {
        validator(&candidate)
    }

    let enabled = config.network.enabled;
    let config_allowed_domains = config.network.allowed_domains().unwrap_or_default();
    let config_denied_domains = config.network.denied_domains().unwrap_or_default();
    let denied_domain_overrides: HashSet<String> = config_denied_domains
        .iter()
        .map(|entry| entry.to_ascii_lowercase())
        .collect();
    let config_allow_unix_sockets = config.network.allow_unix_sockets();
    validate_non_global_wildcard_domain_patterns("network.denied_domains", &config_denied_domains)?;
    if let Some(max_enabled) = constraints.enabled {
        validate(enabled, move |candidate| {
            if *candidate && !max_enabled {
                Err(invalid_value(
                    "network.enabled",
                    "true",
                    "false (disabled by managed config)",
                ))
            } else {
                Ok(())
            }
        })?;
    }

    if let Some(max_mode) = constraints.mode {
        validate(config.network.mode, move |candidate| {
            if network_mode_rank(*candidate) > network_mode_rank(max_mode) {
                Err(invalid_value(
                    "network.mode",
                    format!("{candidate:?}"),
                    format!("{max_mode:?} or more restrictive"),
                ))
            } else {
                Ok(())
            }
        })?;
    }

    let allow_upstream_proxy = constraints.allow_upstream_proxy;
    validate(
        config.network.allow_upstream_proxy,
        move |candidate| match allow_upstream_proxy {
            Some(true) | None => Ok(()),
            Some(false) => {
                if *candidate {
                    Err(invalid_value(
                        "network.allow_upstream_proxy",
                        "true",
                        "false (disabled by managed config)",
                    ))
                } else {
                    Ok(())
                }
            }
        },
    )?;

    let allow_non_loopback_proxy = constraints.dangerously_allow_non_loopback_proxy;
    validate(
        config.network.dangerously_allow_non_loopback_proxy,
        move |candidate| match allow_non_loopback_proxy {
            Some(true) | None => Ok(()),
            Some(false) => {
                if *candidate {
                    Err(invalid_value(
                        "network.dangerously_allow_non_loopback_proxy",
                        "true",
                        "false (disabled by managed config)",
                    ))
                } else {
                    Ok(())
                }
            }
        },
    )?;

    let allow_all_unix_sockets = constraints
        .dangerously_allow_all_unix_sockets
        .unwrap_or(constraints.allow_unix_sockets.is_none());
    validate(
        config.network.dangerously_allow_all_unix_sockets,
        move |candidate| {
            if *candidate && !allow_all_unix_sockets {
                Err(invalid_value(
                    "network.dangerously_allow_all_unix_sockets",
                    "true",
                    "false (disabled by managed config)",
                ))
            } else {
                Ok(())
            }
        },
    )?;

    if let Some(allow_local_binding) = constraints.allow_local_binding {
        validate(config.network.allow_local_binding, move |candidate| {
            if *candidate && !allow_local_binding {
                Err(invalid_value(
                    "network.allow_local_binding",
                    "true",
                    "false (disabled by managed config)",
                ))
            } else {
                Ok(())
            }
        })?;
    }

    if let Some(allowed_domains) = &constraints.allowed_domains {
        validate_non_global_wildcard_domain_patterns("network.allowed_domains", allowed_domains)?;
        match constraints.allowlist_expansion_enabled {
            Some(true) => {
                let required_set: HashSet<String> = allowed_domains
                    .iter()
                    .map(|entry| entry.to_ascii_lowercase())
                    .collect();
                validate(config_allowed_domains, |candidate| {
                    let candidate_set: HashSet<String> = candidate
                        .iter()
                        .map(|entry| entry.to_ascii_lowercase())
                        .collect();
                    let missing: Vec<String> = required_set
                        .iter()
                        .filter(|entry| {
                            !candidate_set.contains(*entry)
                                && !denied_domain_overrides.contains(*entry)
                        })
                        .cloned()
                        .collect();
                    if missing.is_empty() {
                        Ok(())
                    } else {
                        Err(invalid_value(
                            "network.allowed_domains",
                            "missing managed allowed_domains entries",
                            format!("{missing:?}"),
                        ))
                    }
                })?;
            }
            Some(false) => {
                let required_set: HashSet<String> = allowed_domains
                    .iter()
                    .map(|entry| entry.to_ascii_lowercase())
                    .collect();
                validate(config_allowed_domains, |candidate| {
                    let candidate_set: HashSet<String> = candidate
                        .iter()
                        .map(|entry| entry.to_ascii_lowercase())
                        .collect();
                    let expected_set: HashSet<String> = required_set
                        .difference(&denied_domain_overrides)
                        .cloned()
                        .collect();
                    if candidate_set == expected_set {
                        Ok(())
                    } else {
                        Err(invalid_value(
                            "network.allowed_domains",
                            format!("{candidate:?}"),
                            "must match managed allowed_domains",
                        ))
                    }
                })?;
            }
            None => {
                let managed_patterns: Vec<DomainPattern> = allowed_domains
                    .iter()
                    .map(|entry| DomainPattern::parse_for_constraints(entry))
                    .collect();
                validate(config_allowed_domains, move |candidate| {
                    let mut invalid = Vec::new();
                    for entry in candidate {
                        let candidate_pattern = DomainPattern::parse_for_constraints(entry);
                        if !managed_patterns
                            .iter()
                            .any(|managed| managed.allows(&candidate_pattern))
                        {
                            invalid.push(entry.clone());
                        }
                    }
                    if invalid.is_empty() {
                        Ok(())
                    } else {
                        Err(invalid_value(
                            "network.allowed_domains",
                            format!("{invalid:?}"),
                            "subset of managed allowed_domains",
                        ))
                    }
                })?;
            }
        }
    }

    if let Some(denied_domains) = &constraints.denied_domains {
        validate_non_global_wildcard_domain_patterns("network.denied_domains", denied_domains)?;
        let required_set: HashSet<String> = denied_domains
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();
        match constraints.denylist_expansion_enabled {
            Some(false) => {
                validate(config_denied_domains, move |candidate| {
                    let candidate_set: HashSet<String> = candidate
                        .iter()
                        .map(|entry| entry.to_ascii_lowercase())
                        .collect();
                    if candidate_set == required_set {
                        Ok(())
                    } else {
                        Err(invalid_value(
                            "network.denied_domains",
                            format!("{candidate:?}"),
                            "must match managed denied_domains",
                        ))
                    }
                })?;
            }
            Some(true) | None => {
                validate(config_denied_domains, move |candidate| {
                    let candidate_set: HashSet<String> =
                        candidate.iter().map(|s| s.to_ascii_lowercase()).collect();
                    let missing: Vec<String> = required_set
                        .iter()
                        .filter(|entry| !candidate_set.contains(*entry))
                        .cloned()
                        .collect();
                    if missing.is_empty() {
                        Ok(())
                    } else {
                        Err(invalid_value(
                            "network.denied_domains",
                            "missing managed denied_domains entries",
                            format!("{missing:?}"),
                        ))
                    }
                })?;
            }
        }
    }

    if let Some(allow_unix_sockets) = &constraints.allow_unix_sockets {
        let allowed_set: HashSet<String> = allow_unix_sockets
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();
        validate(config_allow_unix_sockets, move |candidate| {
            let mut invalid = Vec::new();
            for entry in candidate {
                if !allowed_set.contains(&entry.to_ascii_lowercase()) {
                    invalid.push(entry.clone());
                }
            }
            if invalid.is_empty() {
                Ok(())
            } else {
                Err(invalid_value(
                    "network.allow_unix_sockets",
                    format!("{invalid:?}"),
                    "subset of managed allow_unix_sockets",
                ))
            }
        })?;
    }

    Ok(())
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

fn is_global_wildcard_domain_pattern(pattern: &str) -> bool {
    let normalized = normalize_pattern(pattern);
    expand_domain_pattern(&normalized)
        .iter()
        .any(|candidate| candidate == "*")
}

fn normalize_pattern(pattern: &str) -> String {
    let pattern = pattern.trim();
    if pattern == "*" {
        return "*".to_string();
    }

    let (prefix, remainder) = if let Some(domain) = pattern.strip_prefix("**.") {
        ("**.", domain)
    } else if let Some(domain) = pattern.strip_prefix("*.") {
        ("*.", domain)
    } else {
        ("", pattern)
    };

    let remainder = normalize_host(remainder);
    if prefix.is_empty() {
        remainder
    } else {
        format!("{prefix}{remainder}")
    }
}

#[derive(Debug, Clone)]
enum DomainPattern {
    ApexAndSubdomains(String),
    SubdomainsOnly(String),
    Exact(String),
}

impl DomainPattern {
    fn parse(input: &str) -> Self {
        let input = input.trim();
        if input.is_empty() {
            return Self::Exact(String::new());
        }
        if let Some(domain) = input.strip_prefix("**.") {
            Self::parse_domain(domain, Self::ApexAndSubdomains)
        } else if let Some(domain) = input.strip_prefix("*.") {
            Self::parse_domain(domain, Self::SubdomainsOnly)
        } else {
            Self::Exact(input.to_string())
        }
    }

    fn parse_for_constraints(input: &str) -> Self {
        let input = input.trim();
        if input.is_empty() {
            return Self::Exact(String::new());
        }
        if let Some(domain) = input.strip_prefix("**.") {
            return Self::ApexAndSubdomains(parse_domain_for_constraints(domain));
        }
        if let Some(domain) = input.strip_prefix("*.") {
            return Self::SubdomainsOnly(parse_domain_for_constraints(domain));
        }
        Self::Exact(parse_domain_for_constraints(input))
    }

    fn parse_domain(domain: &str, build: impl FnOnce(String) -> Self) -> Self {
        let domain = domain.trim();
        if domain.is_empty() {
            return Self::Exact(String::new());
        }
        build(domain.to_string())
    }

    fn allows(&self, candidate: &DomainPattern) -> bool {
        match self {
            DomainPattern::Exact(domain) => match candidate {
                DomainPattern::Exact(candidate) => domain_eq(candidate, domain),
                _ => false,
            },
            DomainPattern::SubdomainsOnly(domain) => match candidate {
                DomainPattern::Exact(candidate) => is_strict_subdomain(candidate, domain),
                DomainPattern::SubdomainsOnly(candidate) => {
                    is_subdomain_or_equal(candidate, domain)
                }
                DomainPattern::ApexAndSubdomains(candidate) => {
                    is_strict_subdomain(candidate, domain)
                }
            },
            DomainPattern::ApexAndSubdomains(domain) => match candidate {
                DomainPattern::Exact(candidate) => is_subdomain_or_equal(candidate, domain),
                DomainPattern::SubdomainsOnly(candidate) => {
                    is_subdomain_or_equal(candidate, domain)
                }
                DomainPattern::ApexAndSubdomains(candidate) => {
                    is_subdomain_or_equal(candidate, domain)
                }
            },
        }
    }
}

fn parse_domain_for_constraints(domain: &str) -> String {
    let domain = domain.trim().trim_end_matches('.');
    if domain.is_empty() {
        return String::new();
    }
    let host = if domain.starts_with('[') && domain.ends_with(']') {
        &domain[1..domain.len().saturating_sub(1)]
    } else {
        domain
    };
    if host.contains('*') || host.contains('?') || host.contains('%') {
        return domain.to_string();
    }
    normalize_host(host)
}

fn expand_domain_pattern(pattern: &str) -> Vec<String> {
    match DomainPattern::parse(pattern) {
        DomainPattern::Exact(domain) => vec![domain],
        DomainPattern::SubdomainsOnly(domain) => {
            vec![format!("?*.{domain}")]
        }
        DomainPattern::ApexAndSubdomains(domain) => {
            vec![domain.clone(), format!("?*.{domain}")]
        }
    }
}

fn normalize_domain(domain: &str) -> String {
    domain.trim_end_matches('.').to_ascii_lowercase()
}

fn domain_eq(left: &str, right: &str) -> bool {
    normalize_domain(left) == normalize_domain(right)
}

fn is_subdomain_or_equal(child: &str, parent: &str) -> bool {
    let child = normalize_domain(child);
    let parent = normalize_domain(parent);
    if child == parent {
        return true;
    }
    child.ends_with(&format!(".{parent}"))
}

fn is_strict_subdomain(child: &str, parent: &str) -> bool {
    let child = normalize_domain(child);
    let parent = normalize_domain(parent);
    child != parent && child.ends_with(&format!(".{parent}"))
}

fn network_mode_rank(mode: NetworkMode) -> u8 {
    match mode {
        NetworkMode::Limited => 0,
        NetworkMode::Full => 1,
    }
}

/// Variant order encodes effective precedence for duplicate patterns:
/// `None < Allow < Deny`, so deny wins over allow when entries conflict.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum NetworkDomainPermission {
    None,
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkDomainPermissionEntry {
    pub pattern: String,
    pub permission: NetworkDomainPermission,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkDomainPermissions {
    pub entries: Vec<NetworkDomainPermissionEntry>,
}

impl Serialize for NetworkDomainPermissions {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.effective_entries()
            .into_iter()
            .map(|entry| (entry.pattern, entry.permission))
            .collect::<BTreeMap<_, _>>()
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NetworkDomainPermissions {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = BTreeMap::<String, NetworkDomainPermission>::deserialize(deserializer)?
            .into_iter()
            .map(|(pattern, permission)| NetworkDomainPermissionEntry {
                pattern,
                permission,
            })
            .collect();
        Ok(Self { entries })
    }
}

impl NetworkDomainPermissions {
    fn effective_entries(&self) -> Vec<NetworkDomainPermissionEntry> {
        let mut order = Vec::new();
        let mut effective_permissions = BTreeMap::new();

        for entry in &self.entries {
            if !effective_permissions.contains_key(&entry.pattern) {
                order.push(entry.pattern.clone());
            }

            let permission = effective_permissions
                .entry(entry.pattern.clone())
                .or_insert(entry.permission);
            if entry.permission > *permission {
                *permission = entry.permission;
            }
        }

        order
            .into_iter()
            .filter_map(|pattern| {
                effective_permissions.remove(&pattern).map(|permission| {
                    NetworkDomainPermissionEntry {
                        pattern,
                        permission,
                    }
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkUnixSocketPermission {
    Allow,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NetworkUnixSocketPermissions {
    #[serde(flatten)]
    pub entries: BTreeMap<String, NetworkUnixSocketPermission>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NetworkProxySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_proxy_url")]
    pub proxy_url: String,
    pub enable_socks5: bool,
    #[serde(default = "default_socks_url")]
    pub socks_url: String,
    pub enable_socks5_udp: bool,
    pub allow_upstream_proxy: bool,
    #[serde(default)]
    pub dangerously_allow_non_loopback_proxy: bool,
    #[serde(default)]
    pub dangerously_allow_all_unix_sockets: bool,
    #[serde(default)]
    pub mode: NetworkMode,
    #[serde(default)]
    pub domains: Option<NetworkDomainPermissions>,
    #[serde(default)]
    pub unix_sockets: Option<NetworkUnixSocketPermissions>,
    pub allow_local_binding: bool,
    #[serde(default)]
    pub mitm: bool,
}

impl Default for NetworkProxySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_url: default_proxy_url(),
            enable_socks5: true,
            socks_url: default_socks_url(),
            enable_socks5_udp: true,
            allow_upstream_proxy: true,
            dangerously_allow_non_loopback_proxy: false,
            dangerously_allow_all_unix_sockets: false,
            mode: NetworkMode::default(),
            domains: None,
            unix_sockets: None,
            allow_local_binding: false,
            mitm: false,
        }
    }
}

impl NetworkProxySettings {
    pub fn allowed_domains(&self) -> Option<Vec<String>> {
        self.domain_entries(NetworkDomainPermission::Allow)
    }

    pub fn denied_domains(&self) -> Option<Vec<String>> {
        self.domain_entries(NetworkDomainPermission::Deny)
    }

    fn domain_entries(&self, permission: NetworkDomainPermission) -> Option<Vec<String>> {
        self.domains
            .as_ref()
            .map(|domains| {
                domains
                    .effective_entries()
                    .iter()
                    .filter(|entry| entry.permission == permission)
                    .map(|entry| entry.pattern.clone())
                    .collect()
            })
            .filter(|entries: &Vec<String>| !entries.is_empty())
    }

    pub fn allow_unix_sockets(&self) -> Vec<String> {
        self.unix_sockets
            .as_ref()
            .map(|unix_sockets| {
                unix_sockets
                    .entries
                    .iter()
                    .filter(|(_, permission)| {
                        matches!(permission, NetworkUnixSocketPermission::Allow)
                    })
                    .map(|(path, _)| path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_allowed_domains(&mut self, allowed_domains: Vec<String>) {
        self.set_domain_entries(allowed_domains, NetworkDomainPermission::Allow);
    }

    pub fn set_denied_domains(&mut self, denied_domains: Vec<String>) {
        self.set_domain_entries(denied_domains, NetworkDomainPermission::Deny);
    }

    pub fn upsert_domain_permission(
        &mut self,
        host: String,
        permission: NetworkDomainPermission,
        normalize: impl Fn(&str) -> String,
    ) {
        let mut domains = self.domains.take().unwrap_or_default();
        let normalized_host = normalize(&host);
        domains
            .entries
            .retain(|entry| normalize(&entry.pattern) != normalized_host);
        domains.entries.push(NetworkDomainPermissionEntry {
            pattern: host,
            permission,
        });
        self.domains = (!domains.entries.is_empty()).then_some(domains);
    }

    pub fn set_allow_unix_sockets(&mut self, allow_unix_sockets: Vec<String>) {
        self.set_unix_socket_entries(allow_unix_sockets, NetworkUnixSocketPermission::Allow);
    }

    fn set_domain_entries(&mut self, entries: Vec<String>, permission: NetworkDomainPermission) {
        let mut domains = self.domains.take().unwrap_or_default();
        domains
            .entries
            .retain(|entry| entry.permission != permission);
        for entry in entries {
            if !domains
                .entries
                .iter()
                .any(|existing| existing.pattern == entry && existing.permission == permission)
            {
                domains.entries.push(NetworkDomainPermissionEntry {
                    pattern: entry,
                    permission,
                });
            }
        }
        self.domains = (!domains.entries.is_empty()).then_some(domains);
    }

    fn set_unix_socket_entries(
        &mut self,
        entries: Vec<String>,
        permission: NetworkUnixSocketPermission,
    ) {
        let mut unix_sockets = self.unix_sockets.take().unwrap_or_default();
        unix_sockets
            .entries
            .retain(|_, existing| *existing != permission);
        for entry in entries {
            unix_sockets.entries.insert(entry, permission);
        }
        self.unix_sockets = (!unix_sockets.entries.is_empty()).then_some(unix_sockets);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkProxyRuntimeSnapshot {
    pub http_addr: SocketAddr,
    pub socks_addr: SocketAddr,
    pub socks_enabled: bool,
    pub allow_local_binding: bool,
    pub allow_unix_sockets: Vec<String>,
    pub dangerously_allow_all_unix_sockets: bool,
}

impl NetworkProxyRuntimeSnapshot {
    pub fn apply_to_env(&self, env: &mut HashMap<String, String>) {
        apply_proxy_env_overrides(
            env,
            self.http_addr,
            self.socks_addr,
            self.socks_enabled,
            self.allow_local_binding,
        );
    }
}

pub type SharedNetworkProxyRuntime = Arc<dyn NetworkProxyRuntime>;

pub type SharedStartedNetworkProxyRuntime = Arc<dyn StartedNetworkProxyRuntime>;

pub type SharedNetworkProxyRuntimeFactory = Arc<dyn NetworkProxyRuntimeFactory>;

/// Runtime handle for an already-started network proxy.
///
/// Implementations own concrete proxy state and process/task management. Consumers should use this
/// trait when they only need to expose proxy settings to child processes or read the current
/// effective configuration, without depending on the concrete Rama-backed proxy implementation.
pub trait NetworkProxyRuntime: fmt::Debug + Send + Sync + 'static {
    fn runtime_snapshot(&self) -> NetworkProxyRuntimeSnapshot;

    fn current_config(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<NetworkProxyConfig>> + Send + '_>>;

    fn http_addr(&self) -> SocketAddr {
        self.runtime_snapshot().http_addr
    }

    fn socks_addr(&self) -> SocketAddr {
        self.runtime_snapshot().socks_addr
    }

    fn add_allowed_domain<'a>(
        &'a self,
        host: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    fn add_denied_domain<'a>(
        &'a self,
        host: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    fn apply_to_env(&self, env: &mut HashMap<String, String>) {
        self.runtime_snapshot().apply_to_env(env);
    }
}

pub struct NetworkProxyStartRequest {
    pub config: NetworkProxyConfig,
    pub constraints: NetworkProxyConstraints,
    pub policy_decider: Option<Arc<dyn NetworkPolicyDecider>>,
    pub blocked_request_observer: Option<Arc<dyn BlockedRequestObserver>>,
    pub audit_metadata: NetworkProxyAuditMetadata,
}

/// Owner for a started proxy and its background tasks.
///
/// The handle must keep the concrete proxy runtime alive until it is dropped. Core code should only
/// keep this trait object and never depend on the concrete proxy implementation or task handles.
pub trait StartedNetworkProxyRuntime: Send + Sync + 'static {
    fn proxy(&self) -> SharedNetworkProxyRuntime;

    fn update_config(
        &self,
        config: NetworkProxyConfig,
        constraints: NetworkProxyConstraints,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
}

/// Factory for creating started network proxy runtimes from lightweight config DTOs.
pub trait NetworkProxyRuntimeFactory: Send + Sync + 'static {
    fn start(
        &self,
        request: NetworkProxyStartRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SharedStartedNetworkProxyRuntime>> + Send + '_>>;
}

pub struct DisabledNetworkProxyRuntimeFactory;

impl NetworkProxyRuntimeFactory for DisabledNetworkProxyRuntimeFactory {
    fn start(
        &self,
        _request: NetworkProxyStartRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SharedStartedNetworkProxyRuntime>> + Send + '_>>
    {
        Box::pin(async {
            Err(anyhow::anyhow!(
                "network proxy runtime factory is not configured"
            ))
        })
    }
}

pub fn proxy_url_env_value<'a>(
    env: &'a HashMap<String, String>,
    canonical_key: &str,
) -> Option<&'a str> {
    if let Some(value) = env.get(canonical_key) {
        return Some(value.as_str());
    }
    let lower_key = canonical_key.to_ascii_lowercase();
    env.get(lower_key.as_str()).map(String::as_str)
}

pub fn has_proxy_url_env_vars(env: &HashMap<String, String>) -> bool {
    PROXY_URL_ENV_KEYS
        .iter()
        .any(|key| proxy_url_env_value(env, key).is_some_and(|value| !value.trim().is_empty()))
}

fn set_env_keys(env: &mut HashMap<String, String>, keys: &[&str], value: &str) {
    for key in keys {
        env.insert((*key).to_string(), value.to_string());
    }
}

#[cfg(target_os = "macos")]
fn codex_proxy_git_ssh_command(socks_addr: SocketAddr) -> String {
    format!("{CODEX_PROXY_GIT_SSH_COMMAND_PREFIX}{socks_addr}{CODEX_PROXY_GIT_SSH_COMMAND_SUFFIX}")
}

#[cfg(target_os = "macos")]
fn is_codex_proxy_git_ssh_command(command: &str) -> bool {
    command.starts_with(CODEX_PROXY_GIT_SSH_COMMAND_PREFIX)
        && command.ends_with(CODEX_PROXY_GIT_SSH_COMMAND_SUFFIX)
}

fn apply_proxy_env_overrides(
    env: &mut HashMap<String, String>,
    http_addr: SocketAddr,
    socks_addr: SocketAddr,
    socks_enabled: bool,
    allow_local_binding: bool,
) {
    let http_proxy_url = format!("http://{http_addr}");
    let socks_proxy_url = format!("socks5h://{socks_addr}");
    env.insert(PROXY_ACTIVE_ENV_KEY.to_string(), "1".to_string());
    env.insert(
        ALLOW_LOCAL_BINDING_ENV_KEY.to_string(),
        if allow_local_binding {
            "1".to_string()
        } else {
            "0".to_string()
        },
    );

    set_env_keys(
        env,
        &[
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "http_proxy",
            "https_proxy",
            "YARN_HTTP_PROXY",
            "YARN_HTTPS_PROXY",
            "npm_config_http_proxy",
            "npm_config_https_proxy",
            "npm_config_proxy",
            "NPM_CONFIG_HTTP_PROXY",
            "NPM_CONFIG_HTTPS_PROXY",
            "NPM_CONFIG_PROXY",
            "BUNDLE_HTTP_PROXY",
            "BUNDLE_HTTPS_PROXY",
            "PIP_PROXY",
            "DOCKER_HTTP_PROXY",
            "DOCKER_HTTPS_PROXY",
        ],
        &http_proxy_url,
    );
    set_env_keys(env, WEBSOCKET_PROXY_ENV_KEYS, &http_proxy_url);
    set_env_keys(env, NO_PROXY_ENV_KEYS, DEFAULT_NO_PROXY_VALUE);
    env.insert(
        ELECTRON_GET_USE_PROXY_ENV_KEY.to_string(),
        "true".to_string(),
    );

    if socks_enabled {
        set_env_keys(env, ALL_PROXY_ENV_KEYS, &socks_proxy_url);
        set_env_keys(env, FTP_PROXY_ENV_KEYS, &socks_proxy_url);
    } else {
        set_env_keys(env, ALL_PROXY_ENV_KEYS, &http_proxy_url);
        set_env_keys(env, FTP_PROXY_ENV_KEYS, &http_proxy_url);
    }

    #[cfg(target_os = "macos")]
    if socks_enabled {
        match env.get(PROXY_GIT_SSH_COMMAND_ENV_KEY) {
            Some(command) if !is_codex_proxy_git_ssh_command(command) => {}
            _ => {
                env.insert(
                    PROXY_GIT_SSH_COMMAND_ENV_KEY.to_string(),
                    codex_proxy_git_ssh_command(socks_addr),
                );
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkHostPort {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkHostPortParseError {
    MissingHost { input: String },
}

impl fmt::Display for NetworkHostPortParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHost { input } => {
                write!(f, "missing host in network proxy address: {input}")
            }
        }
    }
}

impl Error for NetworkHostPortParseError {}

pub fn parse_network_host_port(
    value: &str,
    default_port: u16,
) -> Result<NetworkHostPort, NetworkHostPortParseError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(NetworkHostPortParseError::MissingHost {
            input: value.to_string(),
        });
    }

    // Avoid treating unbracketed IPv6 literals like "2001:db8::1" as host:port pairs.
    if matches!(trimmed.parse::<IpAddr>(), Ok(IpAddr::V6(_))) && !trimmed.starts_with('[') {
        return Ok(NetworkHostPort {
            host: trimmed.to_string(),
            port: default_port,
        });
    }

    parse_network_host_port_fallback(trimmed, default_port)
}

pub fn host_and_port_from_network_addr(value: &str, default_port: u16) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "<missing>".to_string();
    }

    let parts = match parse_network_host_port(trimmed, default_port) {
        Ok(parts) => parts,
        Err(_) => {
            return format_host_and_port(trimmed, default_port);
        }
    };

    format_host_and_port(&parts.host, parts.port)
}

fn format_host_and_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn parse_network_host_port_fallback(
    input: &str,
    default_port: u16,
) -> Result<NetworkHostPort, NetworkHostPortParseError> {
    let without_scheme = input
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(input);
    let host_port = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let host_port = host_port
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(host_port);

    if host_port.starts_with('[')
        && let Some(end) = host_port.find(']')
    {
        let host = &host_port[1..end];
        let port = host_port[end + 1..]
            .strip_prefix(':')
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or(default_port);
        if host.is_empty() {
            return Err(NetworkHostPortParseError::MissingHost {
                input: input.to_string(),
            });
        }
        return Ok(NetworkHostPort {
            host: host.to_string(),
            port,
        });
    }

    // Only treat `host:port` as such when there's a single `:`. This avoids
    // accidentally interpreting unbracketed IPv6 addresses as `host:port`.
    if host_port.bytes().filter(|b| *b == b':').count() == 1
        && let Some((host, port)) = host_port.rsplit_once(':')
    {
        if host.is_empty() {
            return Err(NetworkHostPortParseError::MissingHost {
                input: input.to_string(),
            });
        }
        return Ok(NetworkHostPort {
            host: host.to_string(),
            port: port.parse::<u16>().ok().unwrap_or(default_port),
        });
    }

    if host_port.is_empty() {
        return Err(NetworkHostPortParseError::MissingHost {
            input: input.to_string(),
        });
    }
    Ok(NetworkHostPort {
        host: host_port.to_string(),
        port: default_port,
    })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    /// Limited (read-only) access: only GET/HEAD/OPTIONS are allowed for HTTP. HTTPS CONNECT is
    /// blocked unless MITM is enabled so the proxy can enforce method policy on inner requests.
    /// SOCKS5 remains blocked in limited mode.
    Limited,
    /// Full network access: all HTTP methods are allowed, and HTTPS CONNECTs are tunneled without
    /// MITM interception.
    #[default]
    Full,
}

impl NetworkMode {
    pub fn allows_method(self, method: &str) -> bool {
        match self {
            Self::Full => true,
            Self::Limited => matches!(method, "GET" | "HEAD" | "OPTIONS"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkProxyAuditMetadata {
    pub conversation_id: Option<String>,
    pub app_version: Option<String>,
    pub user_account_id: Option<String>,
    pub auth_mode: Option<String>,
    pub originator: Option<String>,
    pub user_email: Option<String>,
    pub terminal_type: Option<String>,
    pub model: Option<String>,
    pub slug: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockedRequest {
    pub host: String,
    pub reason: String,
    pub client: Option<String>,
    pub method: Option<String>,
    pub mode: Option<NetworkMode>,
    pub protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub timestamp: i64,
}

pub struct BlockedRequestArgs {
    pub host: String,
    pub reason: String,
    pub client: Option<String>,
    pub method: Option<String>,
    pub mode: Option<NetworkMode>,
    pub protocol: String,
    pub decision: Option<String>,
    pub source: Option<String>,
    pub port: Option<u16>,
}

impl BlockedRequest {
    pub fn new(args: BlockedRequestArgs) -> Self {
        let BlockedRequestArgs {
            host,
            reason,
            client,
            method,
            mode,
            protocol,
            decision,
            source,
            port,
        } = args;
        Self {
            host,
            reason,
            client,
            method,
            mode,
            protocol,
            decision,
            source,
            port,
            timestamp: unix_timestamp_seconds(),
        }
    }
}

/// Host-provided observer for proxy-blocked network requests.
pub trait BlockedRequestObserver: Send + Sync + 'static {
    fn on_blocked_request(
        &self,
        request: BlockedRequest,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

impl<O: BlockedRequestObserver + ?Sized> BlockedRequestObserver for Arc<O> {
    fn on_blocked_request(
        &self,
        request: BlockedRequest,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        (**self).on_blocked_request(request)
    }
}

impl<F, Fut> BlockedRequestObserver for F
where
    F: Fn(BlockedRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn on_blocked_request(
        &self,
        request: BlockedRequest,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin((self)(request))
    }
}

fn unix_timestamp_seconds() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX)
}

fn default_proxy_url() -> String {
    "http://127.0.0.1:3128".to_string()
}

fn default_socks_url() -> String {
    "http://127.0.0.1:8081".to_string()
}

/// Normalize host fragments for policy matching (trim whitespace, strip ports/brackets, lowercase).
pub fn normalize_host(host: &str) -> String {
    let host = host.trim();
    if host.starts_with('[')
        && let Some(end) = host.find(']')
    {
        return normalize_dns_host_or_ip_literal(&host[1..end]);
    }

    // The proxy stack should typically hand us a host without a port, but be
    // defensive and strip `:port` when there is exactly one `:`.
    if host.bytes().filter(|b| *b == b':').count() == 1 {
        let host = host.split(':').next().unwrap_or_default();
        return normalize_dns_host_or_ip_literal(host);
    }

    // Avoid mangling unbracketed IPv6 literals, but strip trailing dots so fully qualified domain
    // names are treated the same as their dotless variants.
    normalize_dns_host_or_ip_literal(host)
}

fn normalize_dns_host_or_ip_literal(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    if let Some(ip) = normalize_ip_literal(host) {
        return ip;
    }
    host.to_string()
}

fn normalize_ip_literal(host: &str) -> Option<String> {
    if host.parse::<IpAddr>().is_ok() {
        return Some(host.to_string());
    }
    for delimiter in ["%25", "%"] {
        if let Some((ip, scope)) = host.split_once(delimiter)
            && ip.parse::<IpAddr>().is_ok()
        {
            return Some(format!("{ip}%{scope}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_network_host_port_defaults_for_empty_string() {
        assert!(parse_network_host_port("", /*default_port*/ 1234).is_err());
    }

    #[test]
    fn parse_network_host_port_defaults_for_whitespace() {
        assert!(parse_network_host_port("   ", /*default_port*/ 5555).is_err());
    }

    #[test]
    fn parse_network_host_port_parses_host_port_without_scheme() {
        assert_eq!(
            parse_network_host_port("127.0.0.1:8080", /*default_port*/ 3128).unwrap(),
            NetworkHostPort {
                host: "127.0.0.1".to_string(),
                port: 8080,
            }
        );
    }

    #[test]
    fn parse_network_host_port_parses_host_port_with_scheme_and_path() {
        assert_eq!(
            parse_network_host_port(
                "http://example.com:8080/some/path",
                /*default_port*/ 3128
            )
            .unwrap(),
            NetworkHostPort {
                host: "example.com".to_string(),
                port: 8080,
            }
        );
    }

    #[test]
    fn parse_network_host_port_parses_host_port_with_query_and_fragment() {
        assert_eq!(
            parse_network_host_port(
                "https://example.com:4443?token=redacted#section",
                /*default_port*/ 3128
            )
            .unwrap(),
            NetworkHostPort {
                host: "example.com".to_string(),
                port: 4443,
            }
        );
    }

    #[test]
    fn parse_network_host_port_strips_userinfo() {
        assert_eq!(
            parse_network_host_port(
                "http://user:pass@host.example:5555",
                /*default_port*/ 3128
            )
            .unwrap(),
            NetworkHostPort {
                host: "host.example".to_string(),
                port: 5555,
            }
        );
    }

    #[test]
    fn parse_network_host_port_parses_ipv6_with_brackets() {
        assert_eq!(
            parse_network_host_port("http://[::1]:9999", /*default_port*/ 3128).unwrap(),
            NetworkHostPort {
                host: "::1".to_string(),
                port: 9999,
            }
        );
    }

    #[test]
    fn parse_network_host_port_does_not_treat_unbracketed_ipv6_as_host_port() {
        assert_eq!(
            parse_network_host_port("2001:db8::1", /*default_port*/ 3128).unwrap(),
            NetworkHostPort {
                host: "2001:db8::1".to_string(),
                port: 3128,
            }
        );
    }

    #[test]
    fn parse_network_host_port_falls_back_to_default_port_when_port_is_invalid() {
        assert_eq!(
            parse_network_host_port("example.com:notaport", /*default_port*/ 3128).unwrap(),
            NetworkHostPort {
                host: "example.com".to_string(),
                port: 3128,
            }
        );
    }

    #[test]
    fn host_and_port_from_network_addr_defaults_for_empty_string() {
        assert_eq!(
            host_and_port_from_network_addr("", /*default_port*/ 1234),
            "<missing>"
        );
    }

    #[test]
    fn host_and_port_from_network_addr_formats_ipv6() {
        assert_eq!(
            host_and_port_from_network_addr("http://[::1]:8080", /*default_port*/ 3128),
            "[::1]:8080"
        );
    }
}
