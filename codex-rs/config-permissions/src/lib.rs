use std::collections::BTreeMap;

use codex_network_proxy_api::NetworkDomainPermission as ProxyNetworkDomainPermission;
use codex_network_proxy_api::NetworkMode;
use codex_network_proxy_api::NetworkProxyConfig;
use codex_network_proxy_api::NetworkUnixSocketPermission as ProxyNetworkUnixSocketPermission;
use codex_network_proxy_api::normalize_host;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::permissions::FileSystemAccessMode;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde::de::Error as _;
use serde::de::value::Error as ValueDeserializerError;
use serde::de::value::StrDeserializer;

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct PermissionsToml {
    #[serde(flatten)]
    pub entries: BTreeMap<String, PermissionProfileToml>,
}

impl PermissionsToml {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PermissionProfileToml {
    pub workspace_roots: Option<WorkspaceRootsToml>,
    pub filesystem: Option<FilesystemPermissionsToml>,
    pub network: Option<NetworkToml>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct WorkspaceRootsToml {
    #[serde(flatten)]
    pub entries: BTreeMap<String, bool>,
}

impl WorkspaceRootsToml {
    pub fn enabled_roots(&self) -> impl Iterator<Item = &String> {
        self.entries
            .iter()
            .filter_map(|(path, enabled)| (*enabled).then_some(path))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct FilesystemPermissionsToml {
    /// Optional maximum depth for expanding unreadable glob patterns on
    /// platforms that snapshot glob matches before sandbox startup.
    #[schemars(range(min = 1))]
    pub glob_scan_max_depth: Option<usize>,
    #[serde(flatten)]
    pub entries: BTreeMap<String, FilesystemPermissionToml>,
}

impl FilesystemPermissionsToml {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(untagged)]
pub enum FilesystemPermissionToml {
    Access(FileSystemAccessMode),
    Scoped(BTreeMap<String, FileSystemAccessMode>),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct FilesystemRequirementsToml {
    pub deny_read: Option<Vec<FilesystemDenyReadPattern>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct PermissionsRequirementsToml {
    pub filesystem: Option<FilesystemRequirementsToml>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct FilesystemConstraints {
    pub deny_read: Vec<FilesystemDenyReadPattern>,
}

impl From<PermissionsRequirementsToml> for FilesystemConstraints {
    fn from(value: PermissionsRequirementsToml) -> Self {
        let deny_read = value
            .filesystem
            .and_then(|filesystem| filesystem.deny_read)
            .unwrap_or_default();
        Self { deny_read }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
#[serde(transparent)]
pub struct FilesystemDenyReadPattern(String);

impl FilesystemDenyReadPattern {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn contains_glob(&self) -> bool {
        self.0.chars().any(is_glob_metacharacter)
    }

    pub fn from_input(input: &str) -> Result<Self, String> {
        if !input.chars().any(is_glob_metacharacter) {
            let path = deserialize_absolute_path(input)?;
            return Ok(Self(path.to_string_lossy().into_owned()));
        }

        let (directory_prefix, suffix) = split_glob_pattern(input);
        let normalized_prefix = if directory_prefix.is_empty() {
            deserialize_absolute_path(".")?
        } else {
            deserialize_absolute_path(directory_prefix)?
        };
        let normalized_prefix = normalized_prefix.to_string_lossy();
        let normalized = if suffix.is_empty() {
            normalized_prefix.into_owned()
        } else if normalized_prefix == "/" {
            format!("/{suffix}")
        } else {
            format!("{normalized_prefix}/{suffix}")
        };
        Ok(Self(normalized))
    }
}

impl From<AbsolutePathBuf> for FilesystemDenyReadPattern {
    fn from(value: AbsolutePathBuf) -> Self {
        Self(value.to_string_lossy().into_owned())
    }
}

impl<'de> Deserialize<'de> for FilesystemDenyReadPattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let input = String::deserialize(deserializer)?;
        Self::from_input(&input).map_err(D::Error::custom)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct NetworkDomainPermissionsToml {
    #[serde(flatten)]
    pub entries: BTreeMap<String, NetworkDomainPermissionToml>,
}

impl NetworkDomainPermissionsToml {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn allowed_domains(&self) -> Option<Vec<String>> {
        let allowed_domains: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, permission)| matches!(permission, NetworkDomainPermissionToml::Allow))
            .map(|(pattern, _)| pattern.clone())
            .collect();
        (!allowed_domains.is_empty()).then_some(allowed_domains)
    }

    pub fn denied_domains(&self) -> Option<Vec<String>> {
        let denied_domains: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, permission)| matches!(permission, NetworkDomainPermissionToml::Deny))
            .map(|(pattern, _)| pattern.clone())
            .collect();
        (!denied_domains.is_empty()).then_some(denied_domains)
    }
}

#[derive(
    Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum NetworkDomainPermissionToml {
    Allow,
    Deny,
}

impl std::fmt::Display for NetworkDomainPermissionToml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let permission = match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        };
        f.write_str(permission)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct NetworkUnixSocketPermissionsToml {
    #[serde(flatten)]
    pub entries: BTreeMap<String, NetworkUnixSocketPermissionToml>,
}

impl NetworkUnixSocketPermissionsToml {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn allow_unix_sockets(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, permission)| matches!(permission, NetworkUnixSocketPermissionToml::Allow))
            .map(|(path, _)| path.clone())
            .collect()
    }
}

#[derive(
    Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum NetworkUnixSocketPermissionToml {
    Allow,
    None,
}

impl std::fmt::Display for NetworkUnixSocketPermissionToml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let permission = match self {
            Self::Allow => "allow",
            Self::None => "none",
        };
        f.write_str(permission)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct NetworkToml {
    pub enabled: Option<bool>,
    pub proxy_url: Option<String>,
    pub enable_socks5: Option<bool>,
    pub socks_url: Option<String>,
    pub enable_socks5_udp: Option<bool>,
    pub allow_upstream_proxy: Option<bool>,
    pub dangerously_allow_non_loopback_proxy: Option<bool>,
    pub dangerously_allow_all_unix_sockets: Option<bool>,
    #[schemars(with = "Option<NetworkModeSchema>")]
    pub mode: Option<NetworkMode>,
    pub domains: Option<NetworkDomainPermissionsToml>,
    pub unix_sockets: Option<NetworkUnixSocketPermissionsToml>,
    pub allow_local_binding: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum NetworkModeSchema {
    Limited,
    Full,
}

impl NetworkToml {
    pub fn apply_to_network_proxy_config(&self, config: &mut NetworkProxyConfig) {
        if let Some(enabled) = self.enabled {
            config.network.enabled = enabled;
        }
        if let Some(proxy_url) = self.proxy_url.as_ref() {
            config.network.proxy_url = proxy_url.clone();
        }
        if let Some(enable_socks5) = self.enable_socks5 {
            config.network.enable_socks5 = enable_socks5;
        }
        if let Some(socks_url) = self.socks_url.as_ref() {
            config.network.socks_url = socks_url.clone();
        }
        if let Some(enable_socks5_udp) = self.enable_socks5_udp {
            config.network.enable_socks5_udp = enable_socks5_udp;
        }
        if let Some(allow_upstream_proxy) = self.allow_upstream_proxy {
            config.network.allow_upstream_proxy = allow_upstream_proxy;
        }
        if let Some(dangerously_allow_non_loopback_proxy) =
            self.dangerously_allow_non_loopback_proxy
        {
            config.network.dangerously_allow_non_loopback_proxy =
                dangerously_allow_non_loopback_proxy;
        }
        if let Some(dangerously_allow_all_unix_sockets) = self.dangerously_allow_all_unix_sockets {
            config.network.dangerously_allow_all_unix_sockets = dangerously_allow_all_unix_sockets;
        }
        if let Some(mode) = self.mode {
            config.network.mode = mode;
        }
        if let Some(domains) = self.domains.as_ref() {
            overlay_network_domain_permissions(config, domains);
        }
        if let Some(unix_sockets) = self.unix_sockets.as_ref() {
            let mut proxy_unix_sockets = config.network.unix_sockets.take().unwrap_or_default();
            for (path, permission) in &unix_sockets.entries {
                let permission = match permission {
                    NetworkUnixSocketPermissionToml::Allow => {
                        ProxyNetworkUnixSocketPermission::Allow
                    }
                    NetworkUnixSocketPermissionToml::None => ProxyNetworkUnixSocketPermission::None,
                };
                proxy_unix_sockets.entries.insert(path.clone(), permission);
            }
            config.network.unix_sockets =
                (!proxy_unix_sockets.entries.is_empty()).then_some(proxy_unix_sockets);
        }
        if let Some(allow_local_binding) = self.allow_local_binding {
            config.network.allow_local_binding = allow_local_binding;
        }
    }

    pub fn to_network_proxy_config(&self) -> NetworkProxyConfig {
        let mut config = NetworkProxyConfig::default();
        self.apply_to_network_proxy_config(&mut config);
        config
    }
}

pub fn overlay_network_domain_permissions(
    config: &mut NetworkProxyConfig,
    domains: &NetworkDomainPermissionsToml,
) {
    for (pattern, permission) in &domains.entries {
        let permission = match permission {
            NetworkDomainPermissionToml::Allow => ProxyNetworkDomainPermission::Allow,
            NetworkDomainPermissionToml::Deny => ProxyNetworkDomainPermission::Deny,
        };
        config
            .network
            .upsert_domain_permission(pattern.clone(), permission, normalize_host);
    }
}

fn deserialize_absolute_path(input: &str) -> Result<AbsolutePathBuf, String> {
    AbsolutePathBuf::deserialize(StrDeserializer::<ValueDeserializerError>::new(input))
        .map_err(|err| err.to_string())
}

fn split_glob_pattern(input: &str) -> (&str, &str) {
    let Some(first_glob) = input.find(is_glob_metacharacter) else {
        return ("", input);
    };
    let separator_index = input[..first_glob]
        .char_indices()
        .rev()
        .find(|(_, ch)| is_path_separator(*ch))
        .map(|(index, _)| index);

    match separator_index {
        Some(0) => ("/", &input[1..]),
        Some(index)
            if cfg!(windows)
                && index == 2
                && input.as_bytes().get(1) == Some(&b':')
                && input.as_bytes().get(2).is_some() =>
        {
            (&input[..=index], &input[index + 1..])
        }
        Some(index) => (&input[..index], &input[index + 1..]),
        None => ("", input),
    }
}

fn is_path_separator(ch: char) -> bool {
    if cfg!(windows) {
        ch == '/' || ch == '\\'
    } else {
        ch == '/'
    }
}

fn is_glob_metacharacter(ch: char) -> bool {
    matches!(ch, '*' | '?' | '[')
}
