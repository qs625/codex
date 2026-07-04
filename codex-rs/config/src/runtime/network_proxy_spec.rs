use codex_config_requirements::NetworkConstraints;
use codex_config_requirements::NetworkDomainPermissionsToml;
use codex_config_requirements::NetworkUnixSocketPermissionsToml;
use permissions_service_api::Policy;
use codex_network_proxy_api::BlockedRequestObserver;
use codex_network_proxy_api::NetworkDecision;
use codex_network_proxy_api::NetworkDomainPermission;
use codex_network_proxy_api::NetworkPolicyDecider;
use codex_network_proxy_api::NetworkProxyAuditMetadata;
use codex_network_proxy_api::NetworkProxyConfig;
use codex_network_proxy_api::NetworkProxyConstraints;
use codex_network_proxy_api::NetworkProxyRuntimeFactory;
use codex_network_proxy_api::NetworkProxyStartRequest;
use codex_network_proxy_api::SharedStartedNetworkProxyRuntime;
use codex_network_proxy_api::host_and_port_from_network_addr;
use codex_network_proxy_api::normalize_host;
use codex_network_proxy_api::validate_policy_against_constraints;
use protocol::models::PermissionProfile;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkProxySpec {
    base_config: NetworkProxyConfig,
    requirements: Option<NetworkConstraints>,
    config: NetworkProxyConfig,
    constraints: NetworkProxyConstraints,
    hard_deny_allowlist_misses: bool,
}

pub type StartedNetworkProxy = SharedStartedNetworkProxyRuntime;

impl NetworkProxySpec {
    pub(crate) fn enabled(&self) -> bool {
        self.config.network.enabled
    }

    pub fn proxy_host_and_port(&self) -> String {
        host_and_port_from_network_addr(&self.config.network.proxy_url, /*default_port*/ 3128)
    }

    pub fn socks_enabled(&self) -> bool {
        self.config.network.enable_socks5
    }

    pub fn from_config_and_constraints(
        config: NetworkProxyConfig,
        requirements: Option<NetworkConstraints>,
        permission_profile: &PermissionProfile,
    ) -> std::io::Result<Self> {
        let base_config = config.clone();
        let hard_deny_allowlist_misses = requirements
            .as_ref()
            .is_some_and(Self::managed_allowed_domains_only);
        let (config, constraints) = if let Some(requirements) = requirements.as_ref() {
            Self::apply_requirements(
                config,
                requirements,
                permission_profile,
                hard_deny_allowlist_misses,
            )
        } else {
            (config, NetworkProxyConstraints::default())
        };
        validate_policy_against_constraints(&config, &constraints).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("network proxy constraints are invalid: {err}"),
            )
        })?;
        Ok(Self {
            base_config,
            requirements,
            config,
            constraints,
            hard_deny_allowlist_misses,
        })
    }

    pub async fn start_proxy(
        &self,
        factory: &dyn NetworkProxyRuntimeFactory,
        permission_profile: &PermissionProfile,
        policy_decider: Option<Arc<dyn NetworkPolicyDecider>>,
        blocked_request_observer: Option<Arc<dyn BlockedRequestObserver>>,
        enable_network_approval_flow: bool,
        audit_metadata: NetworkProxyAuditMetadata,
    ) -> std::io::Result<StartedNetworkProxy> {
        let policy_decider = if enable_network_approval_flow && !self.hard_deny_allowlist_misses {
            if let Some(policy_decider) = policy_decider {
                Some(policy_decider)
            } else if Self::managed_sandbox_active(permission_profile) {
                Some(
                    Arc::new(|_request| async { NetworkDecision::ask("not_allowed") })
                        as Arc<dyn NetworkPolicyDecider>,
                )
            } else {
                None
            }
        } else {
            None
        };
        factory
            .start(NetworkProxyStartRequest {
                config: self.config.clone(),
                constraints: self.constraints.clone(),
                policy_decider,
                blocked_request_observer,
                audit_metadata,
            })
            .await
            .map_err(|err| std::io::Error::other(format!("failed to start network proxy: {err}")))
    }

    pub fn recompute_for_permission_profile(
        &self,
        permission_profile: &PermissionProfile,
    ) -> std::io::Result<Self> {
        Self::from_config_and_constraints(
            self.base_config.clone(),
            self.requirements.clone(),
            permission_profile,
        )
    }

    pub fn with_exec_policy_network_rules(&self, exec_policy: &Policy) -> std::io::Result<Self> {
        let mut spec = self.clone();
        apply_exec_policy_network_rules(&mut spec.config, exec_policy);
        validate_policy_against_constraints(&spec.config, &spec.constraints).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("network proxy constraints are invalid: {err}"),
            )
        })?;
        Ok(spec)
    }

    pub async fn apply_to_started_proxy(
        &self,
        started_proxy: &StartedNetworkProxy,
    ) -> std::io::Result<()> {
        started_proxy
            .update_config(self.config.clone(), self.constraints.clone())
            .await
            .map_err(|err| {
                std::io::Error::other(format!("failed to update network proxy state: {err}"))
            })
    }

    fn apply_requirements(
        mut config: NetworkProxyConfig,
        requirements: &NetworkConstraints,
        permission_profile: &PermissionProfile,
        hard_deny_allowlist_misses: bool,
    ) -> (NetworkProxyConfig, NetworkProxyConstraints) {
        let mut constraints = NetworkProxyConstraints::default();
        let allowlist_expansion_enabled =
            Self::allowlist_expansion_enabled(permission_profile, hard_deny_allowlist_misses);
        let denylist_expansion_enabled = Self::denylist_expansion_enabled(permission_profile);

        if let Some(enabled) = requirements.enabled {
            config.network.enabled = enabled;
            constraints.enabled = Some(enabled);
        }
        if let Some(http_port) = requirements.http_port {
            config.network.proxy_url = format!("http://127.0.0.1:{http_port}");
        }
        if let Some(socks_port) = requirements.socks_port {
            config.network.socks_url = format!("http://127.0.0.1:{socks_port}");
        }
        if let Some(allow_upstream_proxy) = requirements.allow_upstream_proxy {
            config.network.allow_upstream_proxy = allow_upstream_proxy;
            constraints.allow_upstream_proxy = Some(allow_upstream_proxy);
        }
        if let Some(dangerously_allow_non_loopback_proxy) =
            requirements.dangerously_allow_non_loopback_proxy
        {
            config.network.dangerously_allow_non_loopback_proxy =
                dangerously_allow_non_loopback_proxy;
            constraints.dangerously_allow_non_loopback_proxy =
                Some(dangerously_allow_non_loopback_proxy);
        }
        if let Some(dangerously_allow_all_unix_sockets) =
            requirements.dangerously_allow_all_unix_sockets
        {
            config.network.dangerously_allow_all_unix_sockets = dangerously_allow_all_unix_sockets;
            constraints.dangerously_allow_all_unix_sockets =
                Some(dangerously_allow_all_unix_sockets);
        }
        let managed_allowed_domains = if hard_deny_allowlist_misses {
            Some(
                requirements
                    .domains
                    .as_ref()
                    .and_then(NetworkDomainPermissionsToml::allowed_domains)
                    .unwrap_or_default(),
            )
        } else {
            requirements
                .domains
                .as_ref()
                .and_then(NetworkDomainPermissionsToml::allowed_domains)
        };
        if let Some(managed_allowed_domains) = managed_allowed_domains {
            // Managed requirements seed the baseline allowlist. User additions
            // can extend that baseline unless managed-only mode pins the
            // effective allowlist to the managed set.
            let effective_allowed_domains = if allowlist_expansion_enabled {
                Self::merge_domain_lists(
                    managed_allowed_domains.clone(),
                    config.network.allowed_domains().as_deref().unwrap_or(&[]),
                )
            } else {
                managed_allowed_domains.clone()
            };
            config
                .network
                .set_allowed_domains(effective_allowed_domains);
            constraints.allowed_domains = Some(managed_allowed_domains);
            constraints.allowlist_expansion_enabled = Some(allowlist_expansion_enabled);
        }
        let managed_denied_domains = requirements
            .domains
            .as_ref()
            .and_then(NetworkDomainPermissionsToml::denied_domains);
        if let Some(managed_denied_domains) = managed_denied_domains {
            let effective_denied_domains = if denylist_expansion_enabled {
                Self::merge_domain_lists(
                    managed_denied_domains.clone(),
                    config.network.denied_domains().as_deref().unwrap_or(&[]),
                )
            } else {
                managed_denied_domains.clone()
            };
            config.network.set_denied_domains(effective_denied_domains);
            constraints.denied_domains = Some(managed_denied_domains);
            constraints.denylist_expansion_enabled = Some(denylist_expansion_enabled);
        }
        if requirements.unix_sockets.is_some() {
            let allow_unix_sockets = requirements
                .unix_sockets
                .as_ref()
                .map(NetworkUnixSocketPermissionsToml::allow_unix_sockets)
                .unwrap_or_default();
            config
                .network
                .set_allow_unix_sockets(allow_unix_sockets.clone());
            constraints.allow_unix_sockets = Some(allow_unix_sockets);
        }
        if let Some(allow_local_binding) = requirements.allow_local_binding {
            config.network.allow_local_binding = allow_local_binding;
            constraints.allow_local_binding = Some(allow_local_binding);
        }

        (config, constraints)
    }

    fn allowlist_expansion_enabled(
        permission_profile: &PermissionProfile,
        hard_deny_allowlist_misses: bool,
    ) -> bool {
        Self::managed_sandbox_active(permission_profile) && !hard_deny_allowlist_misses
    }

    fn managed_allowed_domains_only(requirements: &NetworkConstraints) -> bool {
        requirements.managed_allowed_domains_only.unwrap_or(false)
    }

    fn denylist_expansion_enabled(permission_profile: &PermissionProfile) -> bool {
        Self::managed_sandbox_active(permission_profile)
    }

    fn managed_sandbox_active(permission_profile: &PermissionProfile) -> bool {
        matches!(permission_profile, PermissionProfile::Managed { .. })
    }

    fn merge_domain_lists(mut managed: Vec<String>, user_entries: &[String]) -> Vec<String> {
        for entry in user_entries {
            if !managed
                .iter()
                .any(|managed_entry| managed_entry.eq_ignore_ascii_case(entry))
            {
                managed.push(entry.clone());
            }
        }
        managed
    }
}

fn apply_exec_policy_network_rules(config: &mut NetworkProxyConfig, exec_policy: &Policy) {
    let (allowed_domains, denied_domains) = exec_policy.compiled_network_domains();
    upsert_network_domains(config, allowed_domains, /*allow*/ true);
    upsert_network_domains(config, denied_domains, /*allow*/ false);
}

fn upsert_network_domains(config: &mut NetworkProxyConfig, hosts: Vec<String>, allow: bool) {
    let mut incoming = HashSet::new();
    for host in hosts {
        if incoming.insert(host.clone()) {
            config.network.upsert_domain_permission(
                host,
                if allow {
                    NetworkDomainPermission::Allow
                } else {
                    NetworkDomainPermission::Deny
                },
                normalize_host,
            );
        }
    }
}

#[cfg(test)]
#[path = "network_proxy_spec_tests.rs"]
mod tests;
