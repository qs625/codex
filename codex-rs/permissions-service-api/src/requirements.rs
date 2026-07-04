use codex_config_permissions::NetworkDomainPermissionToml;
use codex_config_permissions::NetworkDomainPermissionsToml;
use codex_config_permissions::NetworkUnixSocketPermissionToml;
use codex_config_permissions::NetworkUnixSocketPermissionsToml;
use multimap::MultiMap;
use protocol::config_types::SandboxMode;
use protocol::models::PermissionProfile;
use serde::Deserialize;
use serde::Serialize;
use serde::de::Error as _;
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;

use crate::Decision;
use crate::Policy;
use crate::RuleRef;
use crate::rule::PatternToken;
use crate::rule::PrefixPattern;
use crate::rule::PrefixRule;

#[derive(Debug, Clone)]
pub struct RequirementsExecPolicy {
    policy: Policy,
}

impl RequirementsExecPolicy {
    pub fn new(policy: Policy) -> Self {
        Self { policy }
    }
}

impl PartialEq for RequirementsExecPolicy {
    fn eq(&self, other: &Self) -> bool {
        policy_fingerprint(&self.policy) == policy_fingerprint(&other.policy)
    }
}

impl Eq for RequirementsExecPolicy {}

impl AsRef<Policy> for RequirementsExecPolicy {
    fn as_ref(&self) -> &Policy {
        &self.policy
    }
}

fn policy_fingerprint(policy: &Policy) -> Vec<String> {
    let mut entries = Vec::new();
    for (program, rules) in policy.rules().iter_all() {
        for rule in rules {
            entries.push(format!("{program}:{rule:?}"));
        }
    }
    entries.sort();
    entries
}

/// TOML representation of `[rules]` within `requirements.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RequirementsExecPolicyToml {
    pub prefix_rules: Vec<RequirementsExecPolicyPrefixRuleToml>,
}

/// A TOML representation of the `prefix_rule(...)` Starlark builtin.
///
/// This mirrors the builtin defined in `execpolicy/src/parser.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RequirementsExecPolicyPrefixRuleToml {
    pub pattern: Vec<RequirementsExecPolicyPatternTokenToml>,
    pub decision: Option<RequirementsExecPolicyDecisionToml>,
    pub justification: Option<String>,
}

/// TOML-friendly representation of a pattern token.
///
/// Starlark supports either a string token or a list of alternative tokens at
/// each position, but TOML arrays cannot mix strings and arrays. Using an
/// array of tables sidesteps that restriction.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RequirementsExecPolicyPatternTokenToml {
    pub token: Option<String>,
    pub any_of: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequirementsExecPolicyDecisionToml {
    Allow,
    Prompt,
    Forbidden,
}

impl RequirementsExecPolicyDecisionToml {
    fn as_decision(self) -> Decision {
        match self {
            Self::Allow => Decision::Allow,
            Self::Prompt => Decision::Prompt,
            Self::Forbidden => Decision::Forbidden,
        }
    }
}

#[derive(Debug, Error)]
pub enum RequirementsExecPolicyParseError {
    #[error("rules prefix_rules cannot be empty")]
    EmptyPrefixRules,

    #[error("rules prefix_rule at index {rule_index} has an empty pattern")]
    EmptyPattern { rule_index: usize },

    #[error(
        "rules prefix_rule at index {rule_index} has an invalid pattern token at index {token_index}: {reason}"
    )]
    InvalidPatternToken {
        rule_index: usize,
        token_index: usize,
        reason: String,
    },

    #[error("rules prefix_rule at index {rule_index} has an empty justification")]
    EmptyJustification { rule_index: usize },

    #[error("rules prefix_rule at index {rule_index} is missing a decision")]
    MissingDecision { rule_index: usize },

    #[error(
        "rules prefix_rule at index {rule_index} has decision 'allow', which is not permitted in requirements.toml: Codex merges these rules with other config and uses the most restrictive result (use 'prompt' or 'forbidden')"
    )]
    AllowDecisionNotAllowed { rule_index: usize },
}

impl RequirementsExecPolicyToml {
    /// Convert requirements TOML rules into the internal `.rules`
    /// representation used by the permissions service exec policy layer.
    pub fn to_policy(&self) -> Result<Policy, RequirementsExecPolicyParseError> {
        if self.prefix_rules.is_empty() {
            return Err(RequirementsExecPolicyParseError::EmptyPrefixRules);
        }

        let mut rules_by_program: MultiMap<String, RuleRef> = MultiMap::new();

        for (rule_index, rule) in self.prefix_rules.iter().enumerate() {
            if let Some(justification) = &rule.justification
                && justification.trim().is_empty()
            {
                return Err(RequirementsExecPolicyParseError::EmptyJustification { rule_index });
            }

            if rule.pattern.is_empty() {
                return Err(RequirementsExecPolicyParseError::EmptyPattern { rule_index });
            }

            let pattern_tokens = rule
                .pattern
                .iter()
                .enumerate()
                .map(|(token_index, token)| parse_pattern_token(token, rule_index, token_index))
                .collect::<Result<Vec<_>, _>>()?;

            let decision = match rule.decision {
                Some(RequirementsExecPolicyDecisionToml::Allow) => {
                    return Err(RequirementsExecPolicyParseError::AllowDecisionNotAllowed {
                        rule_index,
                    });
                }
                Some(decision) => decision.as_decision(),
                None => {
                    return Err(RequirementsExecPolicyParseError::MissingDecision { rule_index });
                }
            };
            let justification = rule.justification.clone();

            let (first_token, remaining_tokens) = pattern_tokens
                .split_first()
                .ok_or(RequirementsExecPolicyParseError::EmptyPattern { rule_index })?;

            let rest: Arc<[PatternToken]> = remaining_tokens.to_vec().into();

            for head in first_token.alternatives() {
                let rule: RuleRef = Arc::new(PrefixRule {
                    pattern: PrefixPattern {
                        first: Arc::from(head.as_str()),
                        rest: rest.clone(),
                    },
                    decision,
                    justification: justification.clone(),
                });
                rules_by_program.insert(head.clone(), rule);
            }
        }

        Ok(Policy::new(rules_by_program))
    }

    pub fn to_requirements_policy(
        &self,
    ) -> Result<RequirementsExecPolicy, RequirementsExecPolicyParseError> {
        self.to_policy().map(RequirementsExecPolicy::new)
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SandboxModeRequirement {
    #[serde(rename = "read-only")]
    ReadOnly,

    #[serde(rename = "workspace-write")]
    WorkspaceWrite,

    #[serde(rename = "danger-full-access")]
    DangerFullAccess,

    #[serde(rename = "external-sandbox")]
    ExternalSandbox,
}

impl From<SandboxMode> for SandboxModeRequirement {
    fn from(mode: SandboxMode) -> Self {
        match mode {
            SandboxMode::ReadOnly => SandboxModeRequirement::ReadOnly,
            SandboxMode::WorkspaceWrite => SandboxModeRequirement::WorkspaceWrite,
            SandboxMode::DangerFullAccess => SandboxModeRequirement::DangerFullAccess,
        }
    }
}

#[derive(Serialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkRequirementsToml {
    pub enabled: Option<bool>,
    pub http_port: Option<u16>,
    pub socks_port: Option<u16>,
    pub allow_upstream_proxy: Option<bool>,
    pub dangerously_allow_non_loopback_proxy: Option<bool>,
    pub dangerously_allow_all_unix_sockets: Option<bool>,
    pub domains: Option<NetworkDomainPermissionsToml>,
    /// When true, only managed `allowed_domains` are respected while managed
    /// network enforcement is active. User allowlist entries are ignored.
    pub managed_allowed_domains_only: Option<bool>,
    pub unix_sockets: Option<NetworkUnixSocketPermissionsToml>,
    pub allow_local_binding: Option<bool>,
}

#[derive(Deserialize)]
struct RawNetworkRequirementsToml {
    enabled: Option<bool>,
    http_port: Option<u16>,
    socks_port: Option<u16>,
    allow_upstream_proxy: Option<bool>,
    dangerously_allow_non_loopback_proxy: Option<bool>,
    dangerously_allow_all_unix_sockets: Option<bool>,
    domains: Option<NetworkDomainPermissionsToml>,
    #[serde(default)]
    allowed_domains: Option<Vec<String>>,
    /// When true, only managed `allowed_domains` are respected while managed
    /// network enforcement is active. User allowlist entries are ignored.
    managed_allowed_domains_only: Option<bool>,
    #[serde(default)]
    denied_domains: Option<Vec<String>>,
    unix_sockets: Option<NetworkUnixSocketPermissionsToml>,
    #[serde(default)]
    allow_unix_sockets: Option<Vec<String>>,
    allow_local_binding: Option<bool>,
}

impl<'de> Deserialize<'de> for NetworkRequirementsToml {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawNetworkRequirementsToml::deserialize(deserializer)?;
        let RawNetworkRequirementsToml {
            enabled,
            http_port,
            socks_port,
            allow_upstream_proxy,
            dangerously_allow_non_loopback_proxy,
            dangerously_allow_all_unix_sockets,
            domains,
            allowed_domains,
            managed_allowed_domains_only,
            denied_domains,
            unix_sockets,
            allow_unix_sockets,
            allow_local_binding,
        } = raw;

        if domains.is_some() && (allowed_domains.is_some() || denied_domains.is_some()) {
            return Err(D::Error::custom(
                "`experimental_network.domains` cannot be combined with legacy `allowed_domains` or `denied_domains`",
            ));
        }

        if unix_sockets.is_some() && allow_unix_sockets.is_some() {
            return Err(D::Error::custom(
                "`experimental_network.unix_sockets` cannot be combined with legacy `allow_unix_sockets`",
            ));
        }

        Ok(Self {
            enabled,
            http_port,
            socks_port,
            allow_upstream_proxy,
            dangerously_allow_non_loopback_proxy,
            dangerously_allow_all_unix_sockets,
            domains: domains
                .or_else(|| legacy_domain_permissions_from_lists(allowed_domains, denied_domains)),
            managed_allowed_domains_only,
            unix_sockets: unix_sockets
                .or_else(|| legacy_unix_socket_permissions_from_list(allow_unix_sockets)),
            allow_local_binding,
        })
    }
}

/// Legacy list normalization is intentionally lossy: explicit empty legacy
/// lists are treated as unset when converted to the canonical network
/// permission shape.
fn legacy_domain_permissions_from_lists(
    allowed_domains: Option<Vec<String>>,
    denied_domains: Option<Vec<String>>,
) -> Option<NetworkDomainPermissionsToml> {
    let mut entries = BTreeMap::new();

    for pattern in allowed_domains.unwrap_or_default() {
        entries.insert(pattern, NetworkDomainPermissionToml::Allow);
    }

    for pattern in denied_domains.unwrap_or_default() {
        entries.insert(pattern, NetworkDomainPermissionToml::Deny);
    }

    (!entries.is_empty()).then_some(NetworkDomainPermissionsToml { entries })
}

fn legacy_unix_socket_permissions_from_list(
    allow_unix_sockets: Option<Vec<String>>,
) -> Option<NetworkUnixSocketPermissionsToml> {
    let entries = allow_unix_sockets
        .unwrap_or_default()
        .into_iter()
        .map(|path| (path, NetworkUnixSocketPermissionToml::Allow))
        .collect::<BTreeMap<_, _>>();

    (!entries.is_empty()).then_some(NetworkUnixSocketPermissionsToml { entries })
}

/// Normalized network constraints derived from requirements TOML.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NetworkConstraints {
    pub enabled: Option<bool>,
    pub http_port: Option<u16>,
    pub socks_port: Option<u16>,
    pub allow_upstream_proxy: Option<bool>,
    pub dangerously_allow_non_loopback_proxy: Option<bool>,
    pub dangerously_allow_all_unix_sockets: Option<bool>,
    pub domains: Option<NetworkDomainPermissionsToml>,
    /// When true, only managed `allowed_domains` are respected while managed
    /// network enforcement is active. User allowlist entries are ignored.
    pub managed_allowed_domains_only: Option<bool>,
    pub unix_sockets: Option<NetworkUnixSocketPermissionsToml>,
    pub allow_local_binding: Option<bool>,
}

impl<'de> Deserialize<'de> for NetworkConstraints {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let requirements = NetworkRequirementsToml::deserialize(deserializer)?;
        Ok(requirements.into())
    }
}

impl From<NetworkRequirementsToml> for NetworkConstraints {
    fn from(value: NetworkRequirementsToml) -> Self {
        let NetworkRequirementsToml {
            enabled,
            http_port,
            socks_port,
            allow_upstream_proxy,
            dangerously_allow_non_loopback_proxy,
            dangerously_allow_all_unix_sockets,
            domains,
            managed_allowed_domains_only,
            unix_sockets,
            allow_local_binding,
        } = value;
        Self {
            enabled,
            http_port,
            socks_port,
            allow_upstream_proxy,
            dangerously_allow_non_loopback_proxy,
            dangerously_allow_all_unix_sockets,
            domains,
            managed_allowed_domains_only,
            unix_sockets,
            allow_local_binding,
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct RemoteSandboxConfigToml {
    pub hostname_patterns: Vec<String>,
    pub allowed_sandbox_modes: Vec<SandboxModeRequirement>,
}

pub fn sandbox_mode_requirement_for_permission_profile(
    permission_profile: &PermissionProfile,
) -> SandboxModeRequirement {
    match permission_profile {
        PermissionProfile::Disabled => SandboxModeRequirement::DangerFullAccess,
        PermissionProfile::External { .. } => SandboxModeRequirement::ExternalSandbox,
        PermissionProfile::Managed { .. } => {
            let file_system_policy = permission_profile.file_system_sandbox_policy();
            if file_system_policy.has_full_disk_write_access() {
                SandboxModeRequirement::DangerFullAccess
            } else if file_system_policy
                .entries
                .iter()
                .any(|entry| entry.access.can_write())
            {
                SandboxModeRequirement::WorkspaceWrite
            } else {
                SandboxModeRequirement::ReadOnly
            }
        }
    }
}

fn parse_pattern_token(
    token: &RequirementsExecPolicyPatternTokenToml,
    rule_index: usize,
    token_index: usize,
) -> Result<PatternToken, RequirementsExecPolicyParseError> {
    match (&token.token, &token.any_of) {
        (Some(single), None) => {
            if single.trim().is_empty() {
                return Err(RequirementsExecPolicyParseError::InvalidPatternToken {
                    rule_index,
                    token_index,
                    reason: "token cannot be empty".to_string(),
                });
            }
            Ok(PatternToken::Single(single.clone()))
        }
        (None, Some(alternatives)) => {
            if alternatives.is_empty() {
                return Err(RequirementsExecPolicyParseError::InvalidPatternToken {
                    rule_index,
                    token_index,
                    reason: "any_of cannot be empty".to_string(),
                });
            }
            if alternatives.iter().any(|alt| alt.trim().is_empty()) {
                return Err(RequirementsExecPolicyParseError::InvalidPatternToken {
                    rule_index,
                    token_index,
                    reason: "any_of cannot include empty tokens".to_string(),
                });
            }
            Ok(PatternToken::Alts(alternatives.clone()))
        }
        (Some(_), Some(_)) => Err(RequirementsExecPolicyParseError::InvalidPatternToken {
            rule_index,
            token_index,
            reason: "set either token or any_of, not both".to_string(),
        }),
        (None, None) => Err(RequirementsExecPolicyParseError::InvalidPatternToken {
            rule_index,
            token_index,
            reason: "set either token or any_of".to_string(),
        }),
    }
}
