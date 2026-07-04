use std::sync::Arc;

use bytes::Bytes;
use transport_client::HttpTransport;
use transport_client::RequestBody;
use transport_client_types::RequestTelemetry;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use http::header::CONTENT_TYPE;
use http::header::LOCATION;
use model_service_api::ApiError;
use model_service_api::Provider;
use model_service_api::RealtimeCallResponse;
use model_service_api::RealtimeSessionConfig;
use model_service_api::SharedAuthProvider;
use model_service_api::session_update_session_json;
use serde::Serialize;
use serde_json::Value;
use serde_json::to_string;
use serde_json::to_value;
use tracing::instrument;
use tracing::trace;

use crate::endpoint_session::EndpointSession;

const MULTIPART_BOUNDARY: &str = "codex-realtime-call-boundary";
const MULTIPART_CONTENT_TYPE: &str = "multipart/form-data; boundary=codex-realtime-call-boundary";

pub struct RealtimeCallClient<T: HttpTransport> {
    session: EndpointSession<T>,
}

#[derive(Serialize)]
struct BackendRealtimeCallRequest<'a> {
    sdp: &'a str,
    session: &'a Value,
}

impl<T: HttpTransport> RealtimeCallClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
        }
    }

    pub fn with_telemetry(self, request: Option<Arc<dyn RequestTelemetry>>) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
        }
    }

    fn path() -> &'static str {
        "realtime/calls"
    }

    fn uses_backend_request_shape(&self) -> bool {
        self.session.provider().base_url.contains("/backend-api")
    }

    #[instrument(
        name = "realtime_call.create",
        level = "info",
        skip_all,
        fields(
            http.method = "POST",
            api.path = "realtime/calls"
        )
    )]
    pub async fn create(&self, sdp: String) -> Result<RealtimeCallResponse, ApiError> {
        self.create_with_headers(sdp, HeaderMap::new()).await
    }

    pub async fn create_with_session(
        &self,
        sdp: String,
        session_config: RealtimeSessionConfig,
    ) -> Result<RealtimeCallResponse, ApiError> {
        self.create_with_session_and_headers(sdp, session_config, HeaderMap::new())
            .await
    }

    pub async fn create_with_headers(
        &self,
        sdp: String,
        extra_headers: HeaderMap,
    ) -> Result<RealtimeCallResponse, ApiError> {
        let resp = self
            .session
            .execute_with(
                Method::POST,
                Self::path(),
                extra_headers,
                /*body*/ None,
                |req| {
                    req.headers
                        .insert(CONTENT_TYPE, HeaderValue::from_static("application/sdp"));
                    req.body = Some(RequestBody::Raw(Bytes::from(sdp.clone())));
                },
            )
            .await?;

        let sdp = decode_sdp_response(resp.body.as_ref())?;
        let call_id = decode_call_id_from_location(&resp.headers)?;

        Ok(RealtimeCallResponse { sdp, call_id })
    }

    pub async fn create_with_session_and_headers(
        &self,
        sdp: String,
        session_config: RealtimeSessionConfig,
        extra_headers: HeaderMap,
    ) -> Result<RealtimeCallResponse, ApiError> {
        trace!(target: "model_service::realtime_websocket::wire", "realtime call request SDP: {sdp}");
        let mut session = realtime_session_json(session_config)?;
        if let Some(session) = session.as_object_mut() {
            session.remove("id");
        }
        if self.uses_backend_request_shape() {
            let body = to_value(BackendRealtimeCallRequest {
                sdp: &sdp,
                session: &session,
            })
            .map_err(|err| ApiError::Stream(format!("failed to encode realtime call: {err}")))?;
            let resp = self
                .session
                .execute(Method::POST, Self::path(), extra_headers, Some(body))
                .await?;
            let sdp = decode_sdp_response(resp.body.as_ref())?;
            let call_id = decode_call_id_from_location(&resp.headers)?;
            return Ok(RealtimeCallResponse { sdp, call_id });
        }

        let session = to_string(&session).map_err(|err| ApiError::InvalidRequest {
            message: err.to_string(),
        })?;
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"sdp\"\r\n");
        body.extend_from_slice(b"Content-Type: application/sdp\r\n\r\n");
        body.extend_from_slice(sdp.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"session\"\r\n");
        body.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
        body.extend_from_slice(session.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}--\r\n").as_bytes());

        let resp = self
            .session
            .execute_with(
                Method::POST,
                Self::path(),
                extra_headers,
                /*body*/ None,
                |req| {
                    req.headers.insert(
                        CONTENT_TYPE,
                        HeaderValue::from_static(MULTIPART_CONTENT_TYPE),
                    );
                    req.body = Some(RequestBody::Raw(Bytes::from(body.clone())));
                },
            )
            .await?;

        let sdp = decode_sdp_response(resp.body.as_ref())?;
        let call_id = decode_call_id_from_location(&resp.headers)?;

        Ok(RealtimeCallResponse { sdp, call_id })
    }
}

fn realtime_session_json(session_config: RealtimeSessionConfig) -> Result<Value, ApiError> {
    session_update_session_json(session_config)
        .map_err(|err| ApiError::Stream(format!("failed to encode realtime call session: {err}")))
}

fn decode_sdp_response(body: &[u8]) -> Result<String, ApiError> {
    String::from_utf8(body.to_vec()).map_err(|err| {
        ApiError::Stream(format!(
            "failed to decode realtime call SDP response: {err}"
        ))
    })
}

fn decode_call_id_from_location(headers: &HeaderMap) -> Result<String, ApiError> {
    let location = headers
        .get(LOCATION)
        .ok_or_else(|| ApiError::Stream("realtime call response missing Location".to_string()))?
        .to_str()
        .map_err(|err| ApiError::Stream(format!("invalid realtime call Location: {err}")))?;
    trace!("realtime call Location: {location}");

    location
        .split('?')
        .next()
        .unwrap_or(location)
        .rsplit('/')
        .find(|segment| segment.starts_with("rtc_") && segment.len() > "rtc_".len())
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError::Stream(format!(
                "realtime call Location does not contain a call id: {location}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    use async_trait::async_trait;
    use http::StatusCode;
    use model_service_api::AuthProvider;
    use model_service_api::RealtimeEventParser;
    use model_service_api::RealtimeSessionMode;
    use model_service_api::RetryConfig;
    use pretty_assertions::assert_eq;
    use protocol::protocol::RealtimeOutputModality;
    use protocol::protocol::RealtimeVoice;

    use transport_client::Request;
    use transport_client::Response;
    use transport_client::StreamResponse;
    use transport_client::TransportError;

    #[derive(Clone)]
    struct CapturingTransport {
        last_request: Arc<Mutex<Option<Request>>>,
        response_headers: HeaderMap,
    }

    impl CapturingTransport {
        fn new() -> Self {
            Self::with_location("/v1/realtime/calls/rtc_test")
        }

        fn with_location(location: &str) -> Self {
            let mut response_headers = HeaderMap::new();
            response_headers.insert(LOCATION, HeaderValue::from_str(location).expect("location"));
            Self {
                last_request: Arc::new(Mutex::new(None)),
                response_headers,
            }
        }

        fn without_location() -> Self {
            Self {
                last_request: Arc::new(Mutex::new(None)),
                response_headers: HeaderMap::new(),
            }
        }
    }

    #[async_trait]
    impl HttpTransport for CapturingTransport {
        async fn execute(&self, req: Request) -> Result<Response, TransportError> {
            *self.last_request.lock().expect("lock request") = Some(req);
            Ok(Response {
                status: StatusCode::OK,
                headers: self.response_headers.clone(),
                body: Bytes::from_static(b"answer-sdp"),
                final_url: None,
            })
        }

        async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
            unreachable!("stream transport is not used in realtime_call tests")
        }
    }

    struct StaticAuthProvider;

    impl AuthProvider for StaticAuthProvider {
        fn add_auth_headers(&self, headers: &mut HeaderMap) {
            headers.insert("authorization", HeaderValue::from_static("Bearer test"));
        }
    }

    fn provider(base_url: &str) -> Provider {
        Provider {
            name: "test".to_string(),
            base_url: base_url.to_string(),
            query_params: None,
            headers: HeaderMap::new(),
            stream_idle_timeout: Duration::from_secs(30),
            retry: RetryConfig {
                max_attempts: 1,
                base_delay: Duration::from_millis(1),
                retry_429: false,
                retry_5xx: false,
                retry_transport: false,
            },
        }
    }

    fn session_config(session_id: &str) -> RealtimeSessionConfig {
        RealtimeSessionConfig {
            instructions: "You are helpful".to_string(),
            model: Some("gpt-realtime".to_string()),
            session_id: Some(session_id.to_string()),
            event_parser: RealtimeEventParser::RealtimeV2,
            session_mode: RealtimeSessionMode::Conversational,
            output_modality: RealtimeOutputModality::Audio,
            voice: RealtimeVoice::Alloy,
        }
    }

    #[tokio::test]
    async fn create_uses_application_sdp_body() {
        let transport = CapturingTransport::new();
        let last_request = transport.last_request.clone();
        let client = RealtimeCallClient::new(
            transport,
            provider("https://api.openai.com/v1"),
            Arc::new(StaticAuthProvider),
        );

        let response = client
            .create("offer-sdp".to_string())
            .await
            .expect("response");
        assert_eq!(
            response,
            RealtimeCallResponse {
                sdp: "answer-sdp".to_string(),
                call_id: "rtc_test".to_string(),
            }
        );

        let request = last_request
            .lock()
            .expect("lock request")
            .clone()
            .expect("request");
        assert_eq!(request.method, Method::POST);
        assert_eq!(request.url, "https://api.openai.com/v1/realtime/calls");
        assert_eq!(
            request.headers.get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/sdp"))
        );
        let body = request.body.expect("request body");
        let RequestBody::Raw(body) = body else {
            panic!("expected raw request body");
        };
        assert_eq!(body.as_ref(), b"offer-sdp");
    }

    #[tokio::test]
    async fn create_with_session_uses_backend_json_shape() {
        let transport = CapturingTransport::new();
        let last_request = transport.last_request.clone();
        let client = RealtimeCallClient::new(
            transport,
            provider("https://example.test/backend-api"),
            Arc::new(StaticAuthProvider),
        );

        let response = client
            .create_with_session("offer-sdp".to_string(), session_config("sess_backend"))
            .await
            .expect("response");
        assert_eq!(
            response,
            RealtimeCallResponse {
                sdp: "answer-sdp".to_string(),
                call_id: "rtc_test".to_string(),
            }
        );

        let request = last_request
            .lock()
            .expect("lock request")
            .clone()
            .expect("request");
        let body = request.body.expect("request body");
        let RequestBody::Json(body) = body else {
            panic!("expected json request body");
        };
        assert_eq!(body["sdp"], "offer-sdp");
        assert_eq!(body["session"]["type"], "realtime");
        assert_eq!(body["session"]["model"], "gpt-realtime");
        assert_eq!(body["session"]["id"], Value::Null);
    }

    #[tokio::test]
    async fn create_with_session_uses_multipart_shape_for_api() {
        let transport = CapturingTransport::new();
        let last_request = transport.last_request.clone();
        let client = RealtimeCallClient::new(
            transport,
            provider("https://api.openai.com/v1"),
            Arc::new(StaticAuthProvider),
        );

        client
            .create_with_session("offer-sdp".to_string(), session_config("sess_api"))
            .await
            .expect("response");

        let request = last_request
            .lock()
            .expect("lock request")
            .clone()
            .expect("request");
        assert_eq!(
            request.headers.get(CONTENT_TYPE),
            Some(&HeaderValue::from_static(MULTIPART_CONTENT_TYPE))
        );
        let body = request.body.expect("request body");
        let RequestBody::Raw(body) = body else {
            panic!("expected raw request body");
        };
        let body = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(body.contains("Content-Disposition: form-data; name=\"sdp\""));
        assert!(body.contains("offer-sdp"));
        assert!(body.contains("Content-Disposition: form-data; name=\"session\""));
        assert!(body.contains("\"type\":\"realtime\""));
        assert!(body.contains("\"model\":\"gpt-realtime\""));
        assert!(!body.contains("\"id\":\"sess_api\""));
    }

    #[tokio::test]
    async fn create_with_session_uses_transcription_session_shape() {
        let transport = CapturingTransport::new();
        let last_request = transport.last_request.clone();
        let client = RealtimeCallClient::new(
            transport,
            provider("https://api.openai.com/v1"),
            Arc::new(StaticAuthProvider),
        );

        client
            .create_with_session(
                "offer-sdp".to_string(),
                RealtimeSessionConfig {
                    instructions: "ignored".to_string(),
                    model: Some("gpt-realtime-whisper".to_string()),
                    session_id: Some("sess_transcription".to_string()),
                    event_parser: RealtimeEventParser::RealtimeV2,
                    session_mode: RealtimeSessionMode::Transcription,
                    output_modality: RealtimeOutputModality::Text,
                    voice: RealtimeVoice::Alloy,
                },
            )
            .await
            .expect("response");

        let request = last_request
            .lock()
            .expect("lock request")
            .clone()
            .expect("request");
        let body = request.body.expect("request body");
        let RequestBody::Raw(body) = body else {
            panic!("expected raw request body");
        };
        let body = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(body.contains("\"type\":\"transcription\""));
        assert!(body.contains("\"model\":\"gpt-realtime-whisper\""));
        assert!(body.contains("\"transcription\":{\"model\":\"gpt-realtime-whisper\"}"));
        assert!(!body.contains("\"instructions\""));
        assert!(!body.contains("\"output_modalities\""));
        assert!(!body.contains("\"output\""));
    }

    #[tokio::test]
    async fn create_reports_missing_location_header() {
        let client = RealtimeCallClient::new(
            CapturingTransport::without_location(),
            provider("https://api.openai.com/v1"),
            Arc::new(StaticAuthProvider),
        );

        let error = client
            .create("offer-sdp".to_string())
            .await
            .expect_err("missing location should error");
        match error {
            ApiError::Stream(message) => {
                assert_eq!(message, "realtime call response missing Location");
            }
            other => panic!("expected stream error, got {other:?}"),
        }
    }
}
