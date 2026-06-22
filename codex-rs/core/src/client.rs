pub use codex_model_client::*;

#[cfg(test)]
pub(crate) const WEBSOCKET_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(
    codex_model_provider_info::DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS,
);
