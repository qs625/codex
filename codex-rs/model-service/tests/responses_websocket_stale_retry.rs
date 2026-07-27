use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use futures::StreamExt;
use model_service::DefaultModelProviderFactory;
use model_service::ModelClient;
use model_service::ModelClientSession;
use model_service::Prompt;
use model_service_api::ApiError;
use model_service_api::ApiRuntimeFactory;
use model_service_api::ArcMonitorClientRuntime;
use model_service_api::ChatCompletionsClientRuntime;
use model_service_api::CompactClientRuntime;
use model_service_api::DisabledApiRuntimeFactory;
use model_service_api::MemoriesClientRuntime;
use model_service_api::ModelProviderInfo;
use model_service_api::Provider;
use model_service_api::RealtimeCallClientRuntime;
use model_service_api::RealtimeWebsocketClientRuntime;
use model_service_api::ResponseEvent;
use model_service_api::ResponseStream as ApiResponseStream;
use model_service_api::ResponsesClientRuntime;
use model_service_api::ResponsesStreamRuntimeRequest;
use model_service_api::ResponsesWebsocketConnectRequest;
use model_service_api::ResponsesWebsocketConnectionRuntime;
use model_service_api::ResponsesWebsocketConnectorRuntime;
use model_service_api::ResponsesWsRequest;
use model_service_api::SharedAuthProvider;
use model_service_api::WireApi;
use protocol::SessionId;
use protocol::ThreadId;
use protocol::models::BaseInstructions;
use protocol::openai_models::ModelInfo;
use protocol::protocol::SessionSource;
use rollout_trace_api::InferenceTraceContext;
use serde_json::json;
use session_telemetry_api::DisabledSessionTelemetryFactory;
use session_telemetry_api::SessionTelemetryCreateParams;
use session_telemetry_api::SessionTelemetryFactory;
use session_telemetry_api::SharedSessionTelemetry;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedWebsocketRequest {
    connection_id: usize,
    connection_reused: bool,
}

#[derive(Debug, Default)]
struct FakeWebsocketState {
    next_connection_id: AtomicUsize,
    http_requests: AtomicUsize,
    requests: Mutex<Vec<RecordedWebsocketRequest>>,
    stream_responses: Mutex<VecDeque<Vec<Result<ResponseEvent, ApiError>>>>,
}

impl FakeWebsocketState {
    fn new(stream_responses: Vec<Vec<Result<ResponseEvent, ApiError>>>) -> Arc<Self> {
        Arc::new(Self {
            stream_responses: Mutex::new(stream_responses.into()),
            ..Default::default()
        })
    }

    fn record_request(&self, connection_id: usize, connection_reused: bool) {
        self.requests
            .lock()
            .expect("requests mutex")
            .push(RecordedWebsocketRequest {
                connection_id,
                connection_reused,
            });
    }

    fn requests(&self) -> Vec<RecordedWebsocketRequest> {
        self.requests.lock().expect("requests mutex").clone()
    }

    fn connect_count(&self) -> usize {
        self.next_connection_id.load(Ordering::SeqCst)
    }

    fn record_http_request(&self) {
        self.http_requests.fetch_add(1, Ordering::SeqCst);
    }

    fn http_request_count(&self) -> usize {
        self.http_requests.load(Ordering::SeqCst)
    }
}

struct FakeApiRuntimeFactory {
    websocket: Arc<FakeWebsocketState>,
}

impl ApiRuntimeFactory for FakeApiRuntimeFactory {
    fn responses_websocket_connector(
        &self,
        _provider: Provider,
        _auth: SharedAuthProvider,
    ) -> Box<dyn ResponsesWebsocketConnectorRuntime> {
        Box::new(FakeWebsocketConnector {
            state: Arc::clone(&self.websocket),
        })
    }

    fn compact_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn CompactClientRuntime> {
        DisabledApiRuntimeFactory.compact_client(provider, auth)
    }

    fn memories_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn MemoriesClientRuntime> {
        DisabledApiRuntimeFactory.memories_client(provider, auth)
    }

    fn chat_completions_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn ChatCompletionsClientRuntime> {
        DisabledApiRuntimeFactory.chat_completions_client(provider, auth)
    }

    fn realtime_call_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn RealtimeCallClientRuntime> {
        DisabledApiRuntimeFactory.realtime_call_client(provider, auth)
    }

    fn realtime_websocket_client(
        &self,
        provider: Provider,
    ) -> Box<dyn RealtimeWebsocketClientRuntime> {
        DisabledApiRuntimeFactory.realtime_websocket_client(provider)
    }

    fn responses_client(
        &self,
        _provider: Provider,
        _auth: SharedAuthProvider,
    ) -> Box<dyn ResponsesClientRuntime> {
        Box::new(FakeResponsesClient {
            state: Arc::clone(&self.websocket),
        })
    }

    fn arc_monitor_client(&self) -> Box<dyn ArcMonitorClientRuntime> {
        DisabledApiRuntimeFactory.arc_monitor_client()
    }
}

struct FakeResponsesClient {
    state: Arc<FakeWebsocketState>,
}

impl ResponsesClientRuntime for FakeResponsesClient {
    fn stream_request(
        &self,
        _request: ResponsesStreamRuntimeRequest,
    ) -> model_service_api::ApiRuntimeFuture<'_, Result<ApiResponseStream, ApiError>> {
        Box::pin(async move {
            self.state.record_http_request();
            Ok(completed_response_stream())
        })
    }
}

struct FakeWebsocketConnector {
    state: Arc<FakeWebsocketState>,
}

impl ResponsesWebsocketConnectorRuntime for FakeWebsocketConnector {
    fn connect(
        &self,
        _request: ResponsesWebsocketConnectRequest,
    ) -> model_service_api::ApiRuntimeFuture<
        '_,
        Result<Box<dyn ResponsesWebsocketConnectionRuntime>, ApiError>,
    > {
        Box::pin(async move {
            let connection_id = self.state.next_connection_id.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeWebsocketConnection {
                state: Arc::clone(&self.state),
                connection_id,
            })
                as Box<dyn ResponsesWebsocketConnectionRuntime>)
        })
    }
}

struct FakeWebsocketConnection {
    state: Arc<FakeWebsocketState>,
    connection_id: usize,
}

impl ResponsesWebsocketConnectionRuntime for FakeWebsocketConnection {
    fn is_closed(&self) -> model_service_api::ApiRuntimeFuture<'_, bool> {
        Box::pin(async { false })
    }

    fn send_response_processed(
        &self,
        _response_id: String,
    ) -> model_service_api::ApiRuntimeFuture<'_, Result<(), ApiError>> {
        Box::pin(async { Ok(()) })
    }

    fn stream_request(
        &self,
        _request: ResponsesWsRequest,
        connection_reused: bool,
    ) -> model_service_api::ApiRuntimeFuture<'_, Result<ApiResponseStream, ApiError>> {
        Box::pin(async move {
            self.state
                .record_request(self.connection_id, connection_reused);
            if self.connection_id == 0 && connection_reused {
                return Err(ApiError::Stream(
                    "failed to send websocket request: websocket closed".to_string(),
                ));
            }
            let stream_items = self
                .state
                .stream_responses
                .lock()
                .expect("stream responses mutex")
                .pop_front()
                .unwrap_or_else(completed_response_items);
            Ok(response_stream(stream_items))
        })
    }
}

fn completed_response_items() -> Vec<Result<ResponseEvent, ApiError>> {
    vec![Ok(ResponseEvent::Completed {
        response_id: "resp_1".to_string(),
        token_usage: None,
        end_turn: Some(true),
    })]
}

fn completed_response_stream() -> ApiResponseStream {
    response_stream(completed_response_items())
}

fn response_stream(items: Vec<Result<ResponseEvent, ApiError>>) -> ApiResponseStream {
    ApiResponseStream::new(futures::stream::iter(items), None)
}

fn test_provider_info() -> ModelProviderInfo {
    ModelProviderInfo {
        name: "openai".to_string(),
        base_url: Some("https://example.test".to_string()),
        env_key: None,
        env_key_instructions: None,
        experimental_bearer_token: None,
        auth: None,
        aws: None,
        wire_api: WireApi::Responses,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: Some(0),
        stream_max_retries: Some(0),
        stream_idle_timeout_ms: Some(5_000),
        websocket_connect_timeout_ms: None,
        requires_openai_auth: false,
        supports_websockets: true,
    }
}

fn test_model_info() -> ModelInfo {
    serde_json::from_value(json!({
        "slug": "gpt-test",
        "display_name": "gpt-test",
        "description": null,
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 0,
        "availability_nux": null,
        "upgrade": null,
        "base_instructions": "base instructions",
        "supports_reasoning_summaries": false,
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "context_window": 272000,
        "max_context_window": 272000,
        "auto_compact_token_limit": null,
        "experimental_supported_tools": []
    }))
    .expect("model info")
}

fn test_client(websocket: Arc<FakeWebsocketState>) -> ModelClient {
    ModelClient::new(
        None,
        SessionId::new(),
        ThreadId::new(),
        "test-installation".to_string(),
        Arc::new(FakeApiRuntimeFactory { websocket }),
        Arc::new(DefaultModelProviderFactory),
        test_provider_info(),
        SessionSource::Exec,
        None,
        HashMap::new(),
        false,
        false,
        None,
        None,
    )
}

fn test_telemetry() -> SharedSessionTelemetry {
    DisabledSessionTelemetryFactory.create(SessionTelemetryCreateParams {
        conversation_id: ThreadId::new(),
        model: "gpt-test".to_string(),
        slug: "gpt-test".to_string(),
        account_id: None,
        account_email: None,
        auth_mode: None,
        auth_env: Default::default(),
        originator: "model-service-test".to_string(),
        log_user_prompts: false,
        terminal_type: "test".to_string(),
        session_source: SessionSource::Exec,
        metrics_service_name: None,
    })
}

async fn stream_once(
    session: &mut ModelClientSession,
    model_info: &ModelInfo,
    telemetry: &SharedSessionTelemetry,
) -> anyhow::Result<()> {
    let mut stream = session
        .stream(
            &Prompt {
                base_instructions: BaseInstructions {
                    text: "base instructions".to_string(),
                },
                ..Default::default()
            },
            model_info,
            telemetry,
            None,
            Default::default(),
            None,
            None,
            &InferenceTraceContext::disabled(),
        )
        .await?;
    while let Some(item) = stream.next().await {
        item?;
    }
    Ok(())
}

#[tokio::test]
async fn reused_websocket_send_closed_reconnects_and_resends_once() -> anyhow::Result<()> {
    let websocket = FakeWebsocketState::new(Vec::new());
    let client = test_client(Arc::clone(&websocket));
    let model_info = test_model_info();
    let telemetry = test_telemetry();

    {
        let mut first_session = client.new_session();
        stream_once(&mut first_session, &model_info, &telemetry).await?;
    }

    let mut reused_session = client.new_session();
    stream_once(&mut reused_session, &model_info, &telemetry).await?;

    assert_eq!(websocket.connect_count(), 2);
    assert_eq!(websocket.http_request_count(), 0);
    assert_eq!(
        websocket.requests(),
        vec![
            RecordedWebsocketRequest {
                connection_id: 0,
                connection_reused: false,
            },
            RecordedWebsocketRequest {
                connection_id: 0,
                connection_reused: true,
            },
            RecordedWebsocketRequest {
                connection_id: 1,
                connection_reused: false,
            },
        ]
    );
    Ok(())
}

#[tokio::test]
async fn websocket_disconnect_before_first_event_falls_back_to_http() -> anyhow::Result<()> {
    let websocket = FakeWebsocketState::new(vec![vec![
        Ok(ResponseEvent::ServerModel("gpt-test-routed".to_string())),
        Err(ApiError::Stream(
            "WebSocket protocol error: Connection reset without closing handshake".to_string(),
        )),
    ]]);
    let client = test_client(Arc::clone(&websocket));
    let model_info = test_model_info();
    let telemetry = test_telemetry();

    let mut session = client.new_session();
    stream_once(&mut session, &model_info, &telemetry).await?;

    assert_eq!(websocket.connect_count(), 1);
    assert_eq!(websocket.http_request_count(), 1);
    assert_eq!(
        websocket.requests(),
        vec![RecordedWebsocketRequest {
            connection_id: 0,
            connection_reused: false,
        }]
    );
    Ok(())
}

#[tokio::test]
async fn websocket_disconnect_after_partial_event_does_not_replay_request() -> anyhow::Result<()> {
    let websocket = FakeWebsocketState::new(vec![vec![
        Ok(ResponseEvent::OutputTextDelta("hel".to_string())),
        Err(ApiError::Stream(
            "websocket closed by server before response.completed".to_string(),
        )),
    ]]);
    let client = test_client(Arc::clone(&websocket));
    let model_info = test_model_info();
    let telemetry = test_telemetry();

    let mut session = client.new_session();
    let err = stream_once(&mut session, &model_info, &telemetry)
        .await
        .expect_err("post-output websocket close should propagate");

    assert!(err.to_string().contains("websocket closed by server"));
    assert_eq!(websocket.connect_count(), 1);
    assert_eq!(websocket.http_request_count(), 0);
    assert_eq!(
        websocket.requests(),
        vec![RecordedWebsocketRequest {
            connection_id: 0,
            connection_reused: false,
        }]
    );
    Ok(())
}

#[tokio::test]
async fn websocket_non_disconnect_error_after_metadata_does_not_fallback() -> anyhow::Result<()> {
    let websocket = FakeWebsocketState::new(vec![vec![
        Ok(ResponseEvent::ServerModel("gpt-test-routed".to_string())),
        Err(ApiError::Stream("malformed websocket event".to_string())),
    ]]);
    let client = test_client(Arc::clone(&websocket));
    let model_info = test_model_info();
    let telemetry = test_telemetry();

    let mut session = client.new_session();
    let err = stream_once(&mut session, &model_info, &telemetry)
        .await
        .expect_err("non-disconnect stream errors should propagate");

    assert!(err.to_string().contains("malformed websocket event"));
    assert_eq!(websocket.connect_count(), 1);
    assert_eq!(websocket.http_request_count(), 0);
    assert_eq!(
        websocket.requests(),
        vec![RecordedWebsocketRequest {
            connection_id: 0,
            connection_reused: false,
        }]
    );
    Ok(())
}
