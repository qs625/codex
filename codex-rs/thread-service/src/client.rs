pub use model_service::*;

#[cfg(test)]
pub(crate) const WEBSOCKET_CONNECT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(model_service_api::DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS);
