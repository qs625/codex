use crate::ChatCompletionsClient;
use crate::CompactClient;
use crate::MemoriesClient;
use crate::RealtimeCallClient;
use crate::RealtimeWebsocketClient;
use crate::ResponsesClient;
use crate::ResponsesWebsocketClient;
use transport_client::ReqwestTransport;
use transport_client::TransportError;
use transport_client::build_reqwest_client;
use model_service_api::ApiError;
use model_service_api::ApiRuntimeFactory;
use model_service_api::ApiRuntimeFuture;
use model_service_api::ArcMonitorClientRuntime;
use model_service_api::ArcMonitorRuntimeRequest;
use model_service_api::ArcMonitorRuntimeResponse;
use model_service_api::ChatCompletionsClientRuntime;
use model_service_api::ChatCompletionsRuntimeRequest;
use model_service_api::CompactClientRuntime;
use model_service_api::CompactInputRuntimeRequest;
use model_service_api::CompactionOutput;
use model_service_api::MemoriesClientRuntime;
use model_service_api::MemorySummarizeOutput;
use model_service_api::MemorySummarizeRuntimeRequest;
use model_service_api::Provider;
use model_service_api::RealtimeCallClientRuntime;
use model_service_api::RealtimeCallResponse;
use model_service_api::RealtimeCallRuntimeRequest;
use model_service_api::RealtimeWebsocketClientRuntime;
use model_service_api::ResponseStream;
use model_service_api::ResponsesClientRuntime;
use model_service_api::ResponsesStreamRuntimeRequest;
use model_service_api::ResponsesWebsocketConnectorRuntime;
use model_service_api::SharedAuthProvider;

/// Default API runtime factory backed by the concrete model-service transport clients.
pub struct DefaultApiRuntimeFactory;

impl ApiRuntimeFactory for DefaultApiRuntimeFactory {
    fn responses_websocket_connector(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn ResponsesWebsocketConnectorRuntime> {
        Box::new(ResponsesWebsocketClient::new(provider, auth))
    }

    fn compact_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn CompactClientRuntime> {
        Box::new(DefaultCompactClientRuntime { provider, auth })
    }

    fn memories_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn MemoriesClientRuntime> {
        Box::new(DefaultMemoriesClientRuntime { provider, auth })
    }

    fn chat_completions_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn ChatCompletionsClientRuntime> {
        Box::new(DefaultChatCompletionsClientRuntime { provider, auth })
    }

    fn realtime_call_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn RealtimeCallClientRuntime> {
        Box::new(DefaultRealtimeCallClientRuntime { provider, auth })
    }

    fn realtime_websocket_client(
        &self,
        provider: Provider,
    ) -> Box<dyn RealtimeWebsocketClientRuntime> {
        Box::new(RealtimeWebsocketClient::new(provider))
    }

    fn responses_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn ResponsesClientRuntime> {
        Box::new(DefaultResponsesClientRuntime { provider, auth })
    }

    fn arc_monitor_client(&self) -> Box<dyn ArcMonitorClientRuntime> {
        Box::new(DefaultArcMonitorClientRuntime)
    }
}

struct DefaultCompactClientRuntime {
    provider: Provider,
    auth: SharedAuthProvider,
}

impl CompactClientRuntime for DefaultCompactClientRuntime {
    fn compact_input<'a>(
        &'a self,
        request: CompactInputRuntimeRequest<'a>,
    ) -> ApiRuntimeFuture<'a, Result<CompactionOutput, ApiError>> {
        Box::pin(async move {
            let transport = ReqwestTransport::new(build_reqwest_client());
            CompactClient::new(transport, self.provider.clone(), self.auth.clone())
                .with_telemetry(request.request_telemetry)
                .compact_input(request.input, request.extra_headers)
                .await
        })
    }
}

struct DefaultMemoriesClientRuntime {
    provider: Provider,
    auth: SharedAuthProvider,
}

impl MemoriesClientRuntime for DefaultMemoriesClientRuntime {
    fn summarize_input<'a>(
        &'a self,
        request: MemorySummarizeRuntimeRequest<'a>,
    ) -> ApiRuntimeFuture<'a, Result<Vec<MemorySummarizeOutput>, ApiError>> {
        Box::pin(async move {
            let transport = ReqwestTransport::new(build_reqwest_client());
            MemoriesClient::new(transport, self.provider.clone(), self.auth.clone())
                .with_telemetry(request.request_telemetry)
                .summarize_input(request.input, request.extra_headers)
                .await
        })
    }
}

struct DefaultChatCompletionsClientRuntime {
    provider: Provider,
    auth: SharedAuthProvider,
}

impl ChatCompletionsClientRuntime for DefaultChatCompletionsClientRuntime {
    fn create(
        &self,
        request: ChatCompletionsRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ResponseStream, ApiError>> {
        Box::pin(async move {
            let transport = ReqwestTransport::new(build_reqwest_client());
            ChatCompletionsClient::new(
                transport,
                self.provider.clone(),
                self.auth.clone(),
                request.path,
            )
            .with_telemetry(request.request_telemetry)
            .create(request.request, request.extra_headers)
            .await
        })
    }
}

struct DefaultRealtimeCallClientRuntime {
    provider: Provider,
    auth: SharedAuthProvider,
}

impl RealtimeCallClientRuntime for DefaultRealtimeCallClientRuntime {
    fn create(
        &self,
        request: RealtimeCallRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<RealtimeCallResponse, ApiError>> {
        Box::pin(async move {
            let transport = ReqwestTransport::new(build_reqwest_client());
            RealtimeCallClient::new(transport, self.provider.clone(), self.auth.clone())
                .with_telemetry(request.request_telemetry)
                .create_with_session_and_headers(
                    request.sdp,
                    request.session_config,
                    request.extra_headers,
                )
                .await
        })
    }
}

struct DefaultResponsesClientRuntime {
    provider: Provider,
    auth: SharedAuthProvider,
}

impl ResponsesClientRuntime for DefaultResponsesClientRuntime {
    fn stream_request(
        &self,
        request: ResponsesStreamRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ResponseStream, ApiError>> {
        Box::pin(async move {
            let transport = ReqwestTransport::new(build_reqwest_client());
            ResponsesClient::new(transport, self.provider.clone(), self.auth.clone())
                .with_telemetry(request.request_telemetry, request.sse_telemetry)
                .stream_request(request.request, request.options)
                .await
        })
    }
}

struct DefaultArcMonitorClientRuntime;

impl ArcMonitorClientRuntime for DefaultArcMonitorClientRuntime {
    fn send(
        &self,
        request: ArcMonitorRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ArcMonitorRuntimeResponse, ApiError>> {
        Box::pin(async move {
            let client = build_reqwest_client();
            let mut http_request = client
                .post(&request.url)
                .timeout(request.timeout)
                .json(&request.body);
            if let Some(token) = request.bearer_token {
                http_request = http_request.bearer_auth(token);
            } else if !request.auth_headers.is_empty() {
                http_request = http_request.headers(request.auth_headers);
            }

            let response = http_request
                .send()
                .await
                .map_err(|err| ApiError::Transport(TransportError::Network(err.to_string())))?;
            let status = response.status();
            let body_text = response
                .text()
                .await
                .map_err(|err| ApiError::Transport(TransportError::Network(err.to_string())))?;

            Ok(ArcMonitorRuntimeResponse { status, body_text })
        })
    }
}
