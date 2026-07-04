mod chatgpt_cloudflare_cookies;
mod chatgpt_hosts;
mod custom_ca;
mod default_client;
mod retry;
mod sse;
mod transport;

pub use crate::chatgpt_cloudflare_cookies::with_chatgpt_cloudflare_cookie_store;
pub use crate::chatgpt_hosts::is_allowed_chatgpt_host;
pub use crate::custom_ca::BuildCustomCaTransportError;
/// Test-only subprocess hook for custom CA coverage.
///
/// This stays public only so the `custom_ca_probe` binary target can reuse the shared helper. It
/// is hidden from normal docs because ordinary callers should use
/// [`build_reqwest_client_with_custom_ca`] instead.
#[doc(hidden)]
pub use crate::custom_ca::build_reqwest_client_for_subprocess_tests;
pub use crate::custom_ca::build_reqwest_client_with_custom_ca;
pub use crate::custom_ca::maybe_build_rustls_client_config_with_custom_ca;
pub use crate::default_client::CodexHttpClient;
pub use crate::default_client::CodexRequestBuilder;
pub use crate::default_client::build_reqwest_client;
pub use crate::default_client::create_client;
pub use crate::default_client::try_build_reqwest_client;
pub use crate::default_client::CodexHttpClient as TransportHttpClient;
pub use crate::default_client::CodexRequestBuilder as TransportRequestBuilder;
pub use crate::default_client::build_reqwest_client as build_default_reqwest_client;
pub use crate::default_client::create_client as create_transport_client;
pub use crate::default_client::try_build_reqwest_client as try_build_default_reqwest_client;
pub use crate::retry::backoff;
pub use crate::retry::run_with_retry;
pub use crate::sse::sse_stream;
pub use crate::transport::ByteStream;
pub use crate::transport::HttpTransport;
pub use crate::transport::ReqwestTransport;
pub use crate::transport::StreamResponse;
pub use transport_client_types::PreparedRequestBody;
pub use transport_client_types::Request;
pub use transport_client_types::RequestBody;
pub use transport_client_types::RequestCompression;
pub use transport_client_types::RequestTelemetry;
pub use transport_client_types::Response;
pub use transport_client_types::RetryOn;
pub use transport_client_types::RetryPolicy;
pub use transport_client_types::StreamError;
pub use transport_client_types::TransportError;
