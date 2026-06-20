use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use codex_network_proxy_api::parse_network_host_port;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::Path;
use tracing::warn;

pub use codex_network_proxy_api::NetworkDomainPermission;
pub use codex_network_proxy_api::NetworkDomainPermissionEntry;
pub use codex_network_proxy_api::NetworkDomainPermissions;
pub use codex_network_proxy_api::NetworkMode;
pub use codex_network_proxy_api::NetworkProxyConfig;
pub use codex_network_proxy_api::NetworkProxySettings;
pub use codex_network_proxy_api::NetworkUnixSocketPermission;
pub use codex_network_proxy_api::NetworkUnixSocketPermissions;

/// Clamp non-loopback bind addresses to loopback unless explicitly allowed.
fn clamp_non_loopback(
    addr: SocketAddr,
    allow_non_loopback: bool,
    name: &str,
    override_setting_name: &str,
) -> SocketAddr {
    if addr.ip().is_loopback() {
        return addr;
    }

    if allow_non_loopback {
        warn!("DANGEROUS: {name} listening on non-loopback address {addr}");
        return addr;
    }

    warn!(
        "{name} requested non-loopback bind ({addr}); clamping to 127.0.0.1:{port} (set {override_setting_name} to override)",
        port = addr.port()
    );
    SocketAddr::from(([127, 0, 0, 1], addr.port()))
}

pub(crate) fn clamp_bind_addrs(
    http_addr: SocketAddr,
    socks_addr: SocketAddr,
    cfg: &NetworkProxySettings,
) -> (SocketAddr, SocketAddr) {
    let http_addr = clamp_non_loopback(
        http_addr,
        cfg.dangerously_allow_non_loopback_proxy,
        "HTTP proxy",
        "dangerously_allow_non_loopback_proxy",
    );
    let socks_addr = clamp_non_loopback(
        socks_addr,
        cfg.dangerously_allow_non_loopback_proxy,
        "SOCKS5 proxy",
        "dangerously_allow_non_loopback_proxy",
    );
    if cfg.allow_unix_sockets().is_empty() && !cfg.dangerously_allow_all_unix_sockets {
        return (http_addr, socks_addr);
    }

    // `x-unix-socket` is intentionally a local escape hatch. If the proxy is reachable from
    // outside the machine, it can become a remote bridge into local daemons
    // (e.g. docker.sock). To avoid footguns, enforce loopback binding whenever unix sockets
    // are enabled.
    if cfg.dangerously_allow_non_loopback_proxy && !http_addr.ip().is_loopback() {
        warn!(
            "unix socket proxying is enabled; ignoring dangerously_allow_non_loopback_proxy and clamping HTTP proxy to loopback"
        );
    }
    if cfg.dangerously_allow_non_loopback_proxy && !socks_addr.ip().is_loopback() {
        warn!(
            "unix socket proxying is enabled; ignoring dangerously_allow_non_loopback_proxy and clamping SOCKS5 proxy to loopback"
        );
    }
    (
        SocketAddr::from(([127, 0, 0, 1], http_addr.port())),
        SocketAddr::from(([127, 0, 0, 1], socks_addr.port())),
    )
}

pub struct RuntimeConfig {
    pub http_addr: SocketAddr,
    pub socks_addr: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnixStyleAbsolutePath(String);

impl UnixStyleAbsolutePath {
    fn parse(value: &str) -> Option<Self> {
        value.starts_with('/').then(|| Self(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValidatedUnixSocketPath {
    Native(AbsolutePathBuf),
    UnixStyleAbsolute(UnixStyleAbsolutePath),
}

impl ValidatedUnixSocketPath {
    pub(crate) fn parse(socket_path: &str) -> Result<Self> {
        let path = Path::new(socket_path);
        if path.is_absolute() {
            let path = AbsolutePathBuf::from_absolute_path(path)
                .with_context(|| format!("failed to normalize unix socket path {socket_path:?}"))?;
            return Ok(Self::Native(path));
        }

        if let Some(path) = UnixStyleAbsolutePath::parse(socket_path) {
            return Ok(Self::UnixStyleAbsolute(path));
        }

        bail!("expected an absolute path, got {socket_path:?}");
    }
}

pub(crate) fn validate_unix_socket_allowlist_paths(cfg: &NetworkProxyConfig) -> Result<()> {
    for (index, socket_path) in cfg.network.allow_unix_sockets().iter().enumerate() {
        ValidatedUnixSocketPath::parse(socket_path)
            .with_context(|| format!("invalid network.allow_unix_sockets[{index}]"))?;
    }
    Ok(())
}

pub fn resolve_runtime(cfg: &NetworkProxyConfig) -> Result<RuntimeConfig> {
    validate_unix_socket_allowlist_paths(cfg)?;

    let http_addr = resolve_addr(&cfg.network.proxy_url, /*default_port*/ 3128)
        .with_context(|| format!("invalid network.proxy_url: {}", cfg.network.proxy_url))?;
    let socks_addr = resolve_addr(&cfg.network.socks_url, /*default_port*/ 8081)
        .with_context(|| format!("invalid network.socks_url: {}", cfg.network.socks_url))?;
    let (http_addr, socks_addr) = clamp_bind_addrs(http_addr, socks_addr, &cfg.network);

    Ok(RuntimeConfig {
        http_addr,
        socks_addr,
    })
}

fn resolve_addr(url: &str, default_port: u16) -> Result<SocketAddr> {
    let addr_parts = parse_network_host_port(url, default_port)?;
    let host = if addr_parts.host.eq_ignore_ascii_case("localhost") {
        "127.0.0.1".to_string()
    } else {
        addr_parts.host
    };
    match host.parse::<IpAddr>() {
        Ok(ip) => Ok(SocketAddr::new(ip, addr_parts.port)),
        Err(_) => Ok(SocketAddr::from(([127, 0, 0, 1], addr_parts.port))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;

    fn settings_with_unix_sockets(unix_sockets: &[&str]) -> NetworkProxySettings {
        let mut settings = NetworkProxySettings::default();
        if !unix_sockets.is_empty() {
            settings.set_allow_unix_sockets(
                unix_sockets
                    .iter()
                    .map(|path| (*path).to_string())
                    .collect(),
            );
        }
        settings
    }

    #[test]
    fn network_proxy_settings_default_matches_local_use_baseline() {
        assert_eq!(
            NetworkProxySettings::default(),
            NetworkProxySettings {
                enabled: false,
                proxy_url: "http://127.0.0.1:3128".to_string(),
                enable_socks5: true,
                socks_url: "http://127.0.0.1:8081".to_string(),
                enable_socks5_udp: true,
                allow_upstream_proxy: true,
                dangerously_allow_non_loopback_proxy: false,
                dangerously_allow_all_unix_sockets: false,
                mode: NetworkMode::Full,
                domains: None,
                unix_sockets: None,
                allow_local_binding: false,
                mitm: false,
            }
        );
    }

    #[test]
    fn partial_network_config_uses_struct_defaults_for_missing_fields() {
        let config: NetworkProxyConfig = serde_json::from_str(
            r#"{
                "network": {
                    "enabled": true
                }
            }"#,
        )
        .unwrap();
        let expected = NetworkProxySettings {
            enabled: true,
            ..NetworkProxySettings::default()
        };

        assert_eq!(config.network, expected);
    }

    #[test]
    fn set_allowed_domains_preserves_existing_deny_for_same_pattern() {
        let mut settings = NetworkProxySettings::default();
        settings.set_denied_domains(vec!["example.com".to_string()]);

        settings.set_allowed_domains(vec!["example.com".to_string()]);

        assert_eq!(settings.allowed_domains(), None);
        assert_eq!(
            settings.denied_domains(),
            Some(vec!["example.com".to_string()])
        );
    }

    #[test]
    fn network_domain_permissions_serialize_to_effective_map_shape() {
        let mut settings = NetworkProxySettings::default();
        settings.set_denied_domains(vec!["example.com".to_string()]);
        settings.set_allowed_domains(vec!["example.com".to_string()]);
        let config = NetworkProxyConfig { network: settings };

        let value = serde_json::to_value(&config).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "network": {
                    "enabled": false,
                    "proxy_url": "http://127.0.0.1:3128",
                    "enable_socks5": true,
                    "socks_url": "http://127.0.0.1:8081",
                    "enable_socks5_udp": true,
                    "allow_upstream_proxy": true,
                    "dangerously_allow_non_loopback_proxy": false,
                    "dangerously_allow_all_unix_sockets": false,
                    "mode": "full",
                    "domains": {
                        "example.com": "deny",
                    },
                    "unix_sockets": null,
                    "allow_local_binding": false,
                    "mitm": false,
                }
            })
        );
    }

    #[test]
    fn resolve_addr_maps_localhost_to_loopback() {
        assert_eq!(
            resolve_addr("localhost", /*default_port*/ 3128).unwrap(),
            "127.0.0.1:3128".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn resolve_addr_parses_ip_literals() {
        assert_eq!(
            resolve_addr("1.2.3.4", /*default_port*/ 80).unwrap(),
            "1.2.3.4:80".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn resolve_addr_parses_ipv6_literals() {
        assert_eq!(
            resolve_addr("http://[::1]:8080", /*default_port*/ 3128).unwrap(),
            "[::1]:8080".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn resolve_addr_falls_back_to_loopback_for_hostnames() {
        assert_eq!(
            resolve_addr("http://example.com:5555", /*default_port*/ 3128).unwrap(),
            "127.0.0.1:5555".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn clamp_bind_addrs_allows_non_loopback_when_enabled() {
        let cfg = NetworkProxySettings {
            dangerously_allow_non_loopback_proxy: true,
            ..Default::default()
        };
        let http_addr = "0.0.0.0:3128".parse::<SocketAddr>().unwrap();
        let socks_addr = "0.0.0.0:8081".parse::<SocketAddr>().unwrap();

        let (http_addr, socks_addr) = clamp_bind_addrs(http_addr, socks_addr, &cfg);

        assert_eq!(http_addr, "0.0.0.0:3128".parse::<SocketAddr>().unwrap());
        assert_eq!(socks_addr, "0.0.0.0:8081".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn clamp_bind_addrs_forces_loopback_when_unix_sockets_enabled() {
        let cfg = {
            let mut settings = settings_with_unix_sockets(&["/tmp/docker.sock"]);
            settings.dangerously_allow_non_loopback_proxy = true;
            settings
        };
        let http_addr = "0.0.0.0:3128".parse::<SocketAddr>().unwrap();
        let socks_addr = "0.0.0.0:8081".parse::<SocketAddr>().unwrap();

        let (http_addr, socks_addr) = clamp_bind_addrs(http_addr, socks_addr, &cfg);

        assert_eq!(http_addr, "127.0.0.1:3128".parse::<SocketAddr>().unwrap());
        assert_eq!(socks_addr, "127.0.0.1:8081".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn clamp_bind_addrs_forces_loopback_when_all_unix_sockets_enabled() {
        let cfg = NetworkProxySettings {
            dangerously_allow_non_loopback_proxy: true,
            dangerously_allow_all_unix_sockets: true,
            ..Default::default()
        };
        let http_addr = "0.0.0.0:3128".parse::<SocketAddr>().unwrap();
        let socks_addr = "0.0.0.0:8081".parse::<SocketAddr>().unwrap();

        let (http_addr, socks_addr) = clamp_bind_addrs(http_addr, socks_addr, &cfg);

        assert_eq!(http_addr, "127.0.0.1:3128".parse::<SocketAddr>().unwrap());
        assert_eq!(socks_addr, "127.0.0.1:8081".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn resolve_runtime_rejects_relative_allow_unix_sockets_entries() {
        let cfg = NetworkProxyConfig {
            network: settings_with_unix_sockets(&["relative.sock"]),
        };

        let err = match resolve_runtime(&cfg) {
            Ok(runtime) => panic!(
                "relative allow_unix_sockets should fail, but resolve_runtime succeeded: {:?}",
                runtime.http_addr
            ),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("network.allow_unix_sockets[0]"),
            "error should point at the invalid allow_unix_sockets entry: {err:#}"
        );
    }

    #[test]
    fn resolve_runtime_accepts_unix_style_absolute_allow_unix_sockets_entries() {
        let cfg = NetworkProxyConfig {
            network: settings_with_unix_sockets(&["/private/tmp/example.sock"]),
        };

        assert!(
            resolve_runtime(&cfg).is_ok(),
            "unix-style absolute allow_unix_sockets entry should be accepted"
        );
    }
}
