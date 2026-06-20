use crate::config;
use crate::http_proxy;
use crate::network_policy::NetworkPolicyDecider;
use crate::runtime::BlockedRequestObserver;
use crate::runtime::ConfigState;
use crate::runtime::unix_socket_permissions_supported;
use crate::socks5;
use crate::state::NetworkProxyState;
use anyhow::Context;
use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct Args {}

#[derive(Debug)]
struct ReservedListeners {
    http: Mutex<Option<StdTcpListener>>,
    socks: Mutex<Option<StdTcpListener>>,
}

impl ReservedListeners {
    fn new(http: StdTcpListener, socks: Option<StdTcpListener>) -> Self {
        Self {
            http: Mutex::new(Some(http)),
            socks: Mutex::new(socks),
        }
    }

    fn take_http(&self) -> Option<StdTcpListener> {
        let mut guard = self
            .http
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.take()
    }

    fn take_socks(&self) -> Option<StdTcpListener> {
        let mut guard = self
            .socks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.take()
    }
}

struct ReservedListenerSet {
    http_listener: StdTcpListener,
    socks_listener: Option<StdTcpListener>,
}

impl ReservedListenerSet {
    fn new(http_listener: StdTcpListener, socks_listener: Option<StdTcpListener>) -> Self {
        Self {
            http_listener,
            socks_listener,
        }
    }

    fn http_addr(&self) -> Result<SocketAddr> {
        self.http_listener
            .local_addr()
            .context("failed to read reserved HTTP proxy address")
    }

    fn socks_addr(&self, default_addr: SocketAddr) -> Result<SocketAddr> {
        self.socks_listener
            .as_ref()
            .map_or(Ok(default_addr), |listener| {
                listener
                    .local_addr()
                    .context("failed to read reserved SOCKS5 proxy address")
            })
    }

    fn into_reserved_listeners(self) -> Arc<ReservedListeners> {
        Arc::new(ReservedListeners::new(
            self.http_listener,
            self.socks_listener,
        ))
    }
}

#[derive(Clone)]
pub struct NetworkProxyBuilder {
    state: Option<Arc<NetworkProxyState>>,
    http_addr: Option<SocketAddr>,
    socks_addr: Option<SocketAddr>,
    managed_by_codex: bool,
    policy_decider: Option<Arc<dyn NetworkPolicyDecider>>,
    blocked_request_observer: Option<Arc<dyn BlockedRequestObserver>>,
}

impl Default for NetworkProxyBuilder {
    fn default() -> Self {
        Self {
            state: None,
            http_addr: None,
            socks_addr: None,
            managed_by_codex: true,
            policy_decider: None,
            blocked_request_observer: None,
        }
    }
}

impl NetworkProxyBuilder {
    pub fn state(mut self, state: Arc<NetworkProxyState>) -> Self {
        self.state = Some(state);
        self
    }

    pub fn http_addr(mut self, addr: SocketAddr) -> Self {
        self.http_addr = Some(addr);
        self
    }

    pub fn socks_addr(mut self, addr: SocketAddr) -> Self {
        self.socks_addr = Some(addr);
        self
    }

    pub fn managed_by_codex(mut self, managed_by_codex: bool) -> Self {
        self.managed_by_codex = managed_by_codex;
        self
    }

    pub fn policy_decider<D>(mut self, decider: D) -> Self
    where
        D: NetworkPolicyDecider,
    {
        self.policy_decider = Some(Arc::new(decider));
        self
    }

    pub fn policy_decider_arc(mut self, decider: Arc<dyn NetworkPolicyDecider>) -> Self {
        self.policy_decider = Some(decider);
        self
    }

    pub fn blocked_request_observer<O>(mut self, observer: O) -> Self
    where
        O: BlockedRequestObserver,
    {
        self.blocked_request_observer = Some(Arc::new(observer));
        self
    }

    pub fn blocked_request_observer_arc(
        mut self,
        observer: Arc<dyn BlockedRequestObserver>,
    ) -> Self {
        self.blocked_request_observer = Some(observer);
        self
    }

    pub async fn build(self) -> Result<NetworkProxy> {
        let state = self.state.ok_or_else(|| {
            anyhow::anyhow!(
                "NetworkProxyBuilder requires a state; supply one via builder.state(...)"
            )
        })?;
        state
            .set_blocked_request_observer(self.blocked_request_observer.clone())
            .await;
        let current_cfg = state.current_cfg().await?;
        let (requested_http_addr, requested_socks_addr, reserved_listeners) = if self
            .managed_by_codex
        {
            let runtime = config::resolve_runtime(&current_cfg)?;
            #[cfg(target_os = "windows")]
            let (managed_http_addr, managed_socks_addr) = config::clamp_bind_addrs(
                runtime.http_addr,
                runtime.socks_addr,
                &current_cfg.network,
            );
            #[cfg(target_os = "windows")]
            let reserved = reserve_windows_managed_listeners(
                managed_http_addr,
                managed_socks_addr,
                current_cfg.network.enable_socks5,
            )
            .context("reserve managed loopback proxy listeners")?;
            #[cfg(not(target_os = "windows"))]
            let reserved = reserve_loopback_ephemeral_listeners(current_cfg.network.enable_socks5)
                .context("reserve managed loopback proxy listeners")?;
            let http_addr = reserved.http_addr()?;
            let socks_addr = reserved.socks_addr(runtime.socks_addr)?;
            (
                http_addr,
                socks_addr,
                Some(reserved.into_reserved_listeners()),
            )
        } else {
            let runtime = config::resolve_runtime(&current_cfg)?;
            (
                self.http_addr.unwrap_or(runtime.http_addr),
                self.socks_addr.unwrap_or(runtime.socks_addr),
                None,
            )
        };

        // Reapply bind clamping for caller overrides so unix-socket proxying stays loopback-only.
        let (http_addr, socks_addr) = config::clamp_bind_addrs(
            requested_http_addr,
            requested_socks_addr,
            &current_cfg.network,
        );

        Ok(NetworkProxy {
            state,
            http_addr,
            socks_addr,
            socks_enabled: current_cfg.network.enable_socks5,
            runtime_settings: Arc::new(RwLock::new(NetworkProxyRuntimeSettings::from_config(
                &current_cfg,
            ))),
            reserved_listeners,
            policy_decider: self.policy_decider,
        })
    }
}

fn reserve_loopback_ephemeral_listeners(
    reserve_socks_listener: bool,
) -> Result<ReservedListenerSet> {
    let http_listener =
        reserve_loopback_ephemeral_listener().context("reserve HTTP proxy listener")?;
    let socks_listener = if reserve_socks_listener {
        Some(reserve_loopback_ephemeral_listener().context("reserve SOCKS5 proxy listener")?)
    } else {
        None
    };
    Ok(ReservedListenerSet::new(http_listener, socks_listener))
}

#[cfg(target_os = "windows")]
fn reserve_windows_managed_listeners(
    http_addr: SocketAddr,
    socks_addr: SocketAddr,
    reserve_socks_listener: bool,
) -> Result<ReservedListenerSet> {
    let http_addr = windows_managed_loopback_addr(http_addr);
    let socks_addr = windows_managed_loopback_addr(socks_addr);

    match try_reserve_windows_managed_listeners(http_addr, socks_addr, reserve_socks_listener) {
        Ok(listeners) => Ok(listeners),
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
            warn!("managed Windows proxy ports are busy; falling back to ephemeral loopback ports");
            reserve_loopback_ephemeral_listeners(reserve_socks_listener)
                .context("reserve fallback loopback proxy listeners")
        }
        Err(err) => Err(err).context("reserve Windows managed proxy listeners"),
    }
}

#[cfg(target_os = "windows")]
fn try_reserve_windows_managed_listeners(
    http_addr: SocketAddr,
    socks_addr: SocketAddr,
    reserve_socks_listener: bool,
) -> std::io::Result<ReservedListenerSet> {
    let http_listener = StdTcpListener::bind(http_addr)?;
    let socks_listener = if reserve_socks_listener {
        Some(StdTcpListener::bind(socks_addr)?)
    } else {
        None
    };
    Ok(ReservedListenerSet::new(http_listener, socks_listener))
}

#[cfg(target_os = "windows")]
fn windows_managed_loopback_addr(addr: SocketAddr) -> SocketAddr {
    if !addr.ip().is_loopback() {
        warn!(
            "managed Windows proxies must bind to loopback; clamping {addr} to 127.0.0.1:{}",
            addr.port()
        );
    }
    SocketAddr::from(([127, 0, 0, 1], addr.port()))
}

fn reserve_loopback_ephemeral_listener() -> Result<StdTcpListener> {
    StdTcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .context("bind loopback ephemeral port")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkProxyRuntimeSettings {
    allow_local_binding: bool,
    allow_unix_sockets: Arc<[String]>,
    dangerously_allow_all_unix_sockets: bool,
}

impl NetworkProxyRuntimeSettings {
    fn from_config(config: &config::NetworkProxyConfig) -> Self {
        Self {
            allow_local_binding: config.network.allow_local_binding,
            allow_unix_sockets: config.network.allow_unix_sockets().into(),
            dangerously_allow_all_unix_sockets: config.network.dangerously_allow_all_unix_sockets,
        }
    }
}

#[derive(Clone)]
pub struct NetworkProxy {
    state: Arc<NetworkProxyState>,
    http_addr: SocketAddr,
    socks_addr: SocketAddr,
    socks_enabled: bool,
    runtime_settings: Arc<RwLock<NetworkProxyRuntimeSettings>>,
    reserved_listeners: Option<Arc<ReservedListeners>>,
    policy_decider: Option<Arc<dyn NetworkPolicyDecider>>,
}

impl std::fmt::Debug for NetworkProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid logging internal state (config contents, derived globsets, etc.) which can be noisy
        // and may contain sensitive paths.
        f.debug_struct("NetworkProxy")
            .field("http_addr", &self.http_addr)
            .field("socks_addr", &self.socks_addr)
            .finish_non_exhaustive()
    }
}

impl PartialEq for NetworkProxy {
    fn eq(&self, other: &Self) -> bool {
        self.http_addr == other.http_addr
            && self.socks_addr == other.socks_addr
            && self.runtime_settings() == other.runtime_settings()
    }
}

impl Eq for NetworkProxy {}

impl NetworkProxy {
    pub fn builder() -> NetworkProxyBuilder {
        NetworkProxyBuilder::default()
    }

    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    pub fn socks_addr(&self) -> SocketAddr {
        self.socks_addr
    }

    pub async fn current_cfg(&self) -> Result<config::NetworkProxyConfig> {
        self.state.current_cfg().await
    }

    pub async fn add_allowed_domain(&self, host: &str) -> Result<()> {
        self.state.add_allowed_domain(host).await
    }

    pub async fn add_denied_domain(&self, host: &str) -> Result<()> {
        self.state.add_denied_domain(host).await
    }

    pub fn allow_local_binding(&self) -> bool {
        self.runtime_settings().allow_local_binding
    }

    pub fn allow_unix_sockets(&self) -> Arc<[String]> {
        self.runtime_settings().allow_unix_sockets
    }

    pub fn dangerously_allow_all_unix_sockets(&self) -> bool {
        self.runtime_settings().dangerously_allow_all_unix_sockets
    }

    pub fn runtime_snapshot(&self) -> codex_network_proxy_api::NetworkProxyRuntimeSnapshot {
        let settings = self.runtime_settings();
        codex_network_proxy_api::NetworkProxyRuntimeSnapshot {
            http_addr: self.http_addr,
            socks_addr: self.socks_addr,
            socks_enabled: self.socks_enabled,
            allow_local_binding: settings.allow_local_binding,
            allow_unix_sockets: settings.allow_unix_sockets.iter().cloned().collect(),
            dangerously_allow_all_unix_sockets: settings.dangerously_allow_all_unix_sockets,
        }
    }

    pub fn apply_to_env(&self, env: &mut HashMap<String, String>) {
        self.runtime_snapshot().apply_to_env(env);
    }

    pub async fn replace_config_state(&self, new_state: ConfigState) -> Result<()> {
        let current_cfg = self.state.current_cfg().await?;
        anyhow::ensure!(
            new_state.config.network.enabled == current_cfg.network.enabled,
            "cannot update network.enabled on a running proxy"
        );
        anyhow::ensure!(
            new_state.config.network.proxy_url == current_cfg.network.proxy_url,
            "cannot update network.proxy_url on a running proxy"
        );
        anyhow::ensure!(
            new_state.config.network.socks_url == current_cfg.network.socks_url,
            "cannot update network.socks_url on a running proxy"
        );
        anyhow::ensure!(
            new_state.config.network.enable_socks5 == current_cfg.network.enable_socks5,
            "cannot update network.enable_socks5 on a running proxy"
        );
        anyhow::ensure!(
            new_state.config.network.enable_socks5_udp == current_cfg.network.enable_socks5_udp,
            "cannot update network.enable_socks5_udp on a running proxy"
        );

        let settings = NetworkProxyRuntimeSettings::from_config(&new_state.config);
        self.state.replace_config_state(new_state).await?;
        let mut guard = self
            .runtime_settings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = settings;
        Ok(())
    }

    fn runtime_settings(&self) -> NetworkProxyRuntimeSettings {
        self.runtime_settings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub async fn run(&self) -> Result<NetworkProxyHandle> {
        let current_cfg = self.state.current_cfg().await?;
        if !current_cfg.network.enabled {
            warn!("network.enabled is false; skipping proxy listeners");
            return Ok(NetworkProxyHandle::noop());
        }

        if !unix_socket_permissions_supported() {
            warn!(
                "allowUnixSockets and dangerouslyAllowAllUnixSockets are macOS-only; requests will be rejected on this platform"
            );
        }

        let reserved_listeners = self.reserved_listeners.as_ref();
        let http_listener = reserved_listeners.and_then(|listeners| listeners.take_http());
        let socks_listener = reserved_listeners.and_then(|listeners| listeners.take_socks());

        let http_state = self.state.clone();
        let http_decider = self.policy_decider.clone();
        let http_addr = self.http_addr;
        let http_task = tokio::spawn(async move {
            match http_listener {
                Some(listener) => {
                    http_proxy::run_http_proxy_with_std_listener(http_state, listener, http_decider)
                        .await
                }
                None => http_proxy::run_http_proxy(http_state, http_addr, http_decider).await,
            }
        });

        let socks_task = if current_cfg.network.enable_socks5 {
            let socks_state = self.state.clone();
            let socks_decider = self.policy_decider.clone();
            let socks_addr = self.socks_addr;
            let enable_socks5_udp = current_cfg.network.enable_socks5_udp;
            Some(tokio::spawn(async move {
                match socks_listener {
                    Some(listener) => {
                        socks5::run_socks5_with_std_listener(
                            socks_state,
                            listener,
                            socks_decider,
                            enable_socks5_udp,
                        )
                        .await
                    }
                    None => {
                        socks5::run_socks5(
                            socks_state,
                            socks_addr,
                            socks_decider,
                            enable_socks5_udp,
                        )
                        .await
                    }
                }
            }))
        } else {
            None
        };

        Ok(NetworkProxyHandle {
            http_task: Some(http_task),
            socks_task,
            completed: false,
        })
    }
}

pub struct NetworkProxyHandle {
    http_task: Option<JoinHandle<Result<()>>>,
    socks_task: Option<JoinHandle<Result<()>>>,
    completed: bool,
}

impl NetworkProxyHandle {
    fn noop() -> Self {
        Self {
            http_task: Some(tokio::spawn(async { Ok(()) })),
            socks_task: None,
            completed: true,
        }
    }

    pub async fn wait(mut self) -> Result<()> {
        let http_task = self.http_task.take().context("missing http proxy task")?;
        let socks_task = self.socks_task.take();
        let http_result = http_task.await;
        let socks_result = match socks_task {
            Some(task) => Some(task.await),
            None => None,
        };
        self.completed = true;
        http_result??;
        if let Some(socks_result) = socks_result {
            socks_result??;
        }
        Ok(())
    }

    pub async fn shutdown(mut self) -> Result<()> {
        abort_tasks(self.http_task.take(), self.socks_task.take()).await;
        self.completed = true;
        Ok(())
    }
}

async fn abort_task(task: Option<JoinHandle<Result<()>>>) {
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
}

async fn abort_tasks(
    http_task: Option<JoinHandle<Result<()>>>,
    socks_task: Option<JoinHandle<Result<()>>>,
) {
    abort_task(http_task).await;
    abort_task(socks_task).await;
}

impl Drop for NetworkProxyHandle {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let http_task = self.http_task.take();
        let socks_task = self.socks_task.take();
        tokio::spawn(async move {
            abort_tasks(http_task, socks_task).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NetworkProxySettings;
    use crate::state::network_proxy_state_for_policy;
    use codex_network_proxy_api::ALLOW_LOCAL_BINDING_ENV_KEY;
    use codex_network_proxy_api::DEFAULT_NO_PROXY_VALUE;
    use codex_network_proxy_api::ELECTRON_GET_USE_PROXY_ENV_KEY;
    use codex_network_proxy_api::NetworkProxyRuntimeSnapshot;
    use codex_network_proxy_api::PROXY_ACTIVE_ENV_KEY;
    use codex_network_proxy_api::has_proxy_url_env_vars;
    use codex_network_proxy_api::proxy_url_env_value;
    use pretty_assertions::assert_eq;
    use std::net::IpAddr;
    use std::net::Ipv4Addr;

    const GIT_SSH_COMMAND_ENV_KEY: &str = "GIT_SSH_COMMAND";

    fn codex_proxy_git_ssh_command(socks_addr: SocketAddr) -> String {
        format!("CODEX_PROXY_GIT_SSH_COMMAND=1 ssh -o ProxyCommand='nc -X 5 -x {socks_addr} %h %p'")
    }

    fn apply_proxy_env_overrides(
        env: &mut HashMap<String, String>,
        http_addr: SocketAddr,
        socks_addr: SocketAddr,
        socks_enabled: bool,
        allow_local_binding: bool,
    ) {
        NetworkProxyRuntimeSnapshot {
            http_addr,
            socks_addr,
            socks_enabled,
            allow_local_binding,
            allow_unix_sockets: Vec::new(),
            dangerously_allow_all_unix_sockets: false,
        }
        .apply_to_env(env);
    }

    #[tokio::test]
    async fn managed_proxy_builder_uses_loopback_ports() {
        let http_listener = StdTcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let http_addr = http_listener.local_addr().unwrap();
        let socks_listener = StdTcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let socks_addr = socks_listener.local_addr().unwrap();
        drop(http_listener);
        drop(socks_listener);

        let state = Arc::new(network_proxy_state_for_policy(NetworkProxySettings {
            proxy_url: format!("http://{http_addr}"),
            socks_url: format!("http://{socks_addr}"),
            ..NetworkProxySettings::default()
        }));
        let proxy = match NetworkProxy::builder().state(state).build().await {
            Ok(proxy) => proxy,
            Err(err) => {
                if err
                    .chain()
                    .any(|cause| cause.to_string().contains("Operation not permitted"))
                {
                    return;
                }
                panic!("failed to build managed proxy: {err:#}");
            }
        };

        assert!(proxy.http_addr.ip().is_loopback());
        assert!(proxy.socks_addr.ip().is_loopback());
        #[cfg(target_os = "windows")]
        {
            assert_eq!(proxy.http_addr, http_addr);
            assert_eq!(proxy.socks_addr, socks_addr);
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert_ne!(proxy.http_addr.port(), 0);
            assert_ne!(proxy.socks_addr.port(), 0);
        }
    }

    #[tokio::test]
    async fn non_codex_managed_proxy_builder_uses_configured_ports() {
        let settings = NetworkProxySettings {
            proxy_url: "http://127.0.0.1:43128".to_string(),
            socks_url: "http://127.0.0.1:48081".to_string(),
            ..NetworkProxySettings::default()
        };
        let state = Arc::new(network_proxy_state_for_policy(settings));
        let proxy = NetworkProxy::builder()
            .state(state)
            .managed_by_codex(/*managed_by_codex*/ false)
            .build()
            .await
            .unwrap();

        assert_eq!(
            proxy.http_addr,
            "127.0.0.1:43128".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            proxy.socks_addr,
            "127.0.0.1:48081".parse::<SocketAddr>().unwrap()
        );
    }

    #[tokio::test]
    async fn managed_proxy_builder_does_not_reserve_socks_listener_when_disabled() {
        let settings = NetworkProxySettings {
            enable_socks5: false,
            proxy_url: "http://127.0.0.1:43128".to_string(),
            socks_url: "http://127.0.0.1:43129".to_string(),
            ..NetworkProxySettings::default()
        };
        let state = Arc::new(network_proxy_state_for_policy(settings));
        let proxy = match NetworkProxy::builder().state(state).build().await {
            Ok(proxy) => proxy,
            Err(err) => {
                if err
                    .chain()
                    .any(|cause| cause.to_string().contains("Operation not permitted"))
                {
                    return;
                }
                panic!("failed to build managed proxy: {err:#}");
            }
        };

        assert!(proxy.http_addr.ip().is_loopback());
        assert_ne!(proxy.http_addr.port(), 0);
        assert_eq!(
            proxy.socks_addr,
            "127.0.0.1:43129".parse::<SocketAddr>().unwrap()
        );
        assert!(
            proxy
                .reserved_listeners
                .as_ref()
                .expect("managed builder should reserve listeners")
                .take_socks()
                .is_none()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_managed_loopback_addr_clamps_non_loopback_inputs() {
        assert_eq!(
            windows_managed_loopback_addr("0.0.0.0:3128".parse::<SocketAddr>().unwrap()),
            "127.0.0.1:3128".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            windows_managed_loopback_addr("[::]:8081".parse::<SocketAddr>().unwrap()),
            "127.0.0.1:8081".parse::<SocketAddr>().unwrap()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn reserve_windows_managed_listeners_falls_back_when_http_port_is_busy() {
        let occupied = StdTcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let busy_port = occupied.local_addr().unwrap().port();

        let reserved = reserve_windows_managed_listeners(
            SocketAddr::from(([127, 0, 0, 1], busy_port)),
            SocketAddr::from(([127, 0, 0, 1], 48081)),
            /*reserve_socks_listener*/ false,
        )
        .unwrap();

        assert!(reserved.socks_listener.is_none());
        assert!(
            reserved
                .http_listener
                .local_addr()
                .unwrap()
                .ip()
                .is_loopback()
        );
        assert_ne!(
            reserved.http_listener.local_addr().unwrap().port(),
            busy_port
        );
    }

    #[test]
    fn proxy_url_env_value_resolves_lowercase_aliases() {
        let mut env = HashMap::new();
        env.insert(
            "http_proxy".to_string(),
            "http://127.0.0.1:3128".to_string(),
        );

        assert_eq!(
            proxy_url_env_value(&env, "HTTP_PROXY"),
            Some("http://127.0.0.1:3128")
        );
    }

    #[test]
    fn has_proxy_url_env_vars_detects_lowercase_aliases() {
        let mut env = HashMap::new();
        env.insert(
            "all_proxy".to_string(),
            "socks5h://127.0.0.1:8081".to_string(),
        );

        assert_eq!(has_proxy_url_env_vars(&env), true);
    }

    #[test]
    fn has_proxy_url_env_vars_detects_websocket_proxy_keys() {
        let mut env = HashMap::new();
        env.insert("wss_proxy".to_string(), "http://127.0.0.1:3128".to_string());

        assert_eq!(has_proxy_url_env_vars(&env), true);
    }

    #[test]
    fn apply_proxy_env_overrides_sets_common_tool_vars() {
        let mut env = HashMap::new();
        apply_proxy_env_overrides(
            &mut env,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 3128),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8081),
            /*socks_enabled*/ true,
            /*allow_local_binding*/ false,
        );

        assert_eq!(
            env.get("HTTP_PROXY"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(
            env.get("WS_PROXY"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(
            env.get("WSS_PROXY"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(
            env.get("npm_config_proxy"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(
            env.get("ALL_PROXY"),
            Some(&"socks5h://127.0.0.1:8081".to_string())
        );
        assert_eq!(
            env.get("FTP_PROXY"),
            Some(&"socks5h://127.0.0.1:8081".to_string())
        );
        assert_eq!(
            env.get("NO_PROXY"),
            Some(&DEFAULT_NO_PROXY_VALUE.to_string())
        );
        let no_proxy = env.get("NO_PROXY").expect("NO_PROXY should be set");
        assert!(no_proxy.contains("10.0.0.0/8"));
        assert!(no_proxy.contains("172.16.0.0/12"));
        assert!(no_proxy.contains("192.168.0.0/16"));
        assert!(!no_proxy.contains("169.254.0.0/16"));
        assert_eq!(env.get(PROXY_ACTIVE_ENV_KEY), Some(&"1".to_string()));
        assert_eq!(env.get(ALLOW_LOCAL_BINDING_ENV_KEY), Some(&"0".to_string()));
        assert_eq!(
            env.get(ELECTRON_GET_USE_PROXY_ENV_KEY),
            Some(&"true".to_string())
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            env.get(GIT_SSH_COMMAND_ENV_KEY),
            Some(
                &"CODEX_PROXY_GIT_SSH_COMMAND=1 ssh -o ProxyCommand='nc -X 5 -x 127.0.0.1:8081 %h %p'"
                    .to_string()
            )
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(env.get(GIT_SSH_COMMAND_ENV_KEY), None);
    }

    #[test]
    fn apply_proxy_env_overrides_sets_only_expected_env_keys() {
        let mut env = HashMap::new();
        apply_proxy_env_overrides(
            &mut env,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 3128),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8081),
            /*socks_enabled*/ true,
            /*allow_local_binding*/ false,
        );

        for key in env.keys() {
            let is_managed_git_ssh_key =
                cfg!(target_os = "macos") && key == GIT_SSH_COMMAND_ENV_KEY;
            assert!(
                codex_network_proxy_api::PROXY_ENV_KEYS.contains(&key.as_str())
                    || is_managed_git_ssh_key,
                "proxy env writer set unexpected key: {key}"
            );
        }
    }

    #[test]
    fn apply_proxy_env_overrides_uses_http_for_all_proxy_without_socks() {
        let mut env = HashMap::new();
        apply_proxy_env_overrides(
            &mut env,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 3128),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8081),
            /*socks_enabled*/ false,
            /*allow_local_binding*/ true,
        );

        assert_eq!(
            env.get("ALL_PROXY"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(env.get(ALLOW_LOCAL_BINDING_ENV_KEY), Some(&"1".to_string()));
    }

    #[test]
    fn apply_proxy_env_overrides_uses_plain_http_proxy_url() {
        let mut env = HashMap::new();
        apply_proxy_env_overrides(
            &mut env,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 3128),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8081),
            /*socks_enabled*/ true,
            /*allow_local_binding*/ false,
        );

        assert_eq!(
            env.get("HTTP_PROXY"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(
            env.get("HTTPS_PROXY"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(
            env.get("WS_PROXY"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(
            env.get("WSS_PROXY"),
            Some(&"http://127.0.0.1:3128".to_string())
        );
        assert_eq!(
            env.get("ALL_PROXY"),
            Some(&"socks5h://127.0.0.1:8081".to_string())
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            env.get(GIT_SSH_COMMAND_ENV_KEY),
            Some(
                &"CODEX_PROXY_GIT_SSH_COMMAND=1 ssh -o ProxyCommand='nc -X 5 -x 127.0.0.1:8081 %h %p'"
                    .to_string()
            )
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(env.get(GIT_SSH_COMMAND_ENV_KEY), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn apply_proxy_env_overrides_preserves_existing_git_ssh_command() {
        let mut env = HashMap::new();
        env.insert(
            GIT_SSH_COMMAND_ENV_KEY.to_string(),
            "ssh -o ProxyCommand='tsh proxy ssh --cluster=dev %r@%h:%p'".to_string(),
        );
        apply_proxy_env_overrides(
            &mut env,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 3128),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8081),
            /*socks_enabled*/ true,
            /*allow_local_binding*/ false,
        );

        assert_eq!(
            env.get(GIT_SSH_COMMAND_ENV_KEY),
            Some(&"ssh -o ProxyCommand='tsh proxy ssh --cluster=dev %r@%h:%p'".to_string())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn apply_proxy_env_overrides_preserves_unmarked_git_ssh_command_with_proxy_shape() {
        let mut env = HashMap::new();
        env.insert(
            GIT_SSH_COMMAND_ENV_KEY.to_string(),
            "ssh -o ProxyCommand='nc -X 5 -x 127.0.0.1:8081 %h %p'".to_string(),
        );
        apply_proxy_env_overrides(
            &mut env,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 3128),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 48081),
            /*socks_enabled*/ true,
            /*allow_local_binding*/ false,
        );

        assert_eq!(
            env.get(GIT_SSH_COMMAND_ENV_KEY),
            Some(&"ssh -o ProxyCommand='nc -X 5 -x 127.0.0.1:8081 %h %p'".to_string())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn apply_proxy_env_overrides_refreshes_previous_codex_proxy_git_ssh_command() {
        let mut env = HashMap::new();
        env.insert(
            GIT_SSH_COMMAND_ENV_KEY.to_string(),
            codex_proxy_git_ssh_command(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8081)),
        );

        apply_proxy_env_overrides(
            &mut env,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 43128),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 48081),
            /*socks_enabled*/ true,
            /*allow_local_binding*/ false,
        );

        assert_eq!(
            env.get(GIT_SSH_COMMAND_ENV_KEY),
            Some(&codex_proxy_git_ssh_command(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                48081,
            )))
        );
    }
}
