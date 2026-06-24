use codex_network_proxy_api::SharedNetworkProxyRuntime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkApprovalMode {
    Immediate,
    Deferred,
}

#[derive(Clone, Debug)]
pub struct NetworkApprovalSpec<Trigger> {
    pub network: Option<SharedNetworkProxyRuntime>,
    pub mode: NetworkApprovalMode,
    pub trigger: Trigger,
    pub command: String,
}
