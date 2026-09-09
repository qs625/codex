use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use http::HeaderMap;
use http::StatusCode;
use model_service::ResponsesClient;
use model_service_api::AuthProvider;
use model_service_api::Compression;
use model_service_api::Provider;
use model_service_api::ResponseEvent;
use model_service_api::ResponsesApiRequest;
use model_service_api::ResponsesOptions;
use model_service_api::RetryConfig;
use pretty_assertions::assert_eq;
use protocol::error::ModelInputItemKind;
use protocol::error::ModelInputItemReference;
use protocol::models::FunctionCallOutputPayload;
use protocol::models::ResponseItem;
use serde_json::Value;
use transport_client::HttpTransport;
use transport_client::Request;
use transport_client::Response;
use transport_client::StreamResponse;
use transport_client::TransportError;

#[derive(Clone)]
struct FixtureSseTransport {
    body: String,
}

impl FixtureSseTransport {
    fn new(body: String) -> Self {
        Self { body }
    }
}

#[derive(Clone)]
struct DelayedSseTransport {
    chunks: Vec<(Duration, String)>,
}

#[async_trait]
impl HttpTransport for DelayedSseTransport {
    async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
        Err(TransportError::Build("execute should not run".to_string()))
    }

    async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
        let chunks = self.chunks.clone();
        let stream = futures::stream::unfold(chunks.into_iter(), |mut chunks| async move {
            let (delay, chunk) = chunks.next()?;
            tokio::time::sleep(delay).await;
            Some((Ok::<Bytes, TransportError>(Bytes::from(chunk)), chunks))
        });
        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            bytes: Box::pin(stream),
        })
    }
}

#[async_trait]
impl HttpTransport for FixtureSseTransport {
    async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
        Err(TransportError::Build("execute should not run".to_string()))
    }

    async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
        let stream = futures::stream::iter(vec![Ok::<Bytes, TransportError>(Bytes::from(
            self.body.clone(),
        ))]);
        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            bytes: Box::pin(stream),
        })
    }
}

#[derive(Clone, Default)]
struct NoAuth;

impl AuthProvider for NoAuth {
    fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
}

fn provider(name: &str) -> Provider {
    Provider {
        name: name.to_string(),
        base_url: "https://example.com/v1".to_string(),
        query_params: None,
        headers: HeaderMap::new(),
        retry: RetryConfig {
            max_attempts: 1,
            base_delay: Duration::from_millis(1),
            retry_429: false,
            retry_5xx: false,
            retry_transport: true,
        },
        stream_idle_timeout: Duration::from_millis(50),
    }
}

fn build_responses_body(events: Vec<Value>) -> String {
    let mut body = String::new();
    for e in events {
        let kind = e
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("fixture event missing type in SSE fixture: {e}"));
        if e.as_object().map(|o| o.len() == 1).unwrap_or(false) {
            body.push_str(&format!("event: {kind}\n\n"));
        } else {
            body.push_str(&format!("event: {kind}\ndata: {e}\n\n"));
        }
    }
    body
}

fn build_response_chunk(event: Value) -> String {
    build_responses_body(vec![event])
}

#[tokio::test]
async fn property_name_error_uses_actual_sse_input_source_after_compatibility_filtering()
-> Result<()> {
    let body = build_responses_body(vec![serde_json::json!({
        "type": "response.failed",
        "response": {
            "id": "resp-invalid",
            "error": {
                "message": "Expected a string with maximum length 256",
                "type": "invalid_request_error",
                "code": "property_name_above_max_length",
                "param": "input[0].arguments.outer"
            }
        }
    })]);
    let client = ResponsesClient::new(
        FixtureSseTransport::new(body),
        provider("openai"),
        Arc::new(NoAuth),
    );
    let target = ModelInputItemReference {
        kind: ModelInputItemKind::FunctionCall,
        call_id: "sse-poison".to_string(),
    };
    let request = ResponsesApiRequest {
        model: "gpt-test".to_string(),
        instructions: String::new(),
        input: vec![
            ResponseItem::Other,
            ResponseItem::FunctionCall {
                id: None,
                name: "lookup".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
                call_id: target.call_id.clone(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: target.call_id.clone(),
                output: FunctionCallOutputPayload::from_text("done".to_string()),
            },
        ],
        input_sources: vec![None, Some(target.clone()), Some(target.clone())],
        tools: Vec::new(),
        tool_choice: "auto".to_string(),
        parallel_tool_calls: false,
        reasoning: None,
        store: false,
        stream: true,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
        chat_completions_max_tokens: None,
    };
    let mut stream = client
        .stream_request(request, ResponsesOptions::default())
        .await?;

    let error = loop {
        match stream.next().await {
            Some(Err(error)) => break error,
            Some(Ok(ResponseEvent::RateLimits(_))) => {}
            Some(Ok(event)) => panic!("unexpected SSE event: {event:?}"),
            None => panic!("expected structured SSE error"),
        }
    };
    let model_service_api::ApiError::InvalidModelInput(details) = error else {
        panic!("expected invalid model input");
    };
    assert_eq!(
        details.error_type.as_deref(),
        Some("invalid_request_error")
    );
    assert_eq!(
        details.code.as_deref(),
        Some("property_name_above_max_length")
    );
    assert_eq!(details.param.as_deref(), Some("input[0].arguments.outer"));
    assert_eq!(details.input_index, Some(0));
    assert_eq!(details.source, Some(target));

    Ok(())
}

#[tokio::test]
async fn responses_stream_parses_items_and_completed_end_to_end() -> Result<()> {
    let item1 = serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "Hello"}]
        }
    });

    let item2 = serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "World"}]
        }
    });

    let completed = serde_json::json!({
        "type": "response.completed",
        "response": { "id": "resp1" }
    });

    let body = build_responses_body(vec![item1, item2, completed]);
    let transport = FixtureSseTransport::new(body);
    let client = ResponsesClient::new(transport, provider("openai"), Arc::new(NoAuth));

    let mut stream = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
            Vec::new(),
        )
        .await?;

    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        events.push(ev?);
    }

    let events: Vec<ResponseEvent> = events
        .into_iter()
        .filter(|ev| !matches!(ev, ResponseEvent::RateLimits(_)))
        .collect();

    assert_eq!(events.len(), 3);

    match &events[0] {
        ResponseEvent::OutputItemDone(ResponseItem::Message { role, .. }) => {
            assert_eq!(role, "assistant");
        }
        other => panic!("unexpected first event: {other:?}"),
    }

    match &events[1] {
        ResponseEvent::OutputItemDone(ResponseItem::Message { role, .. }) => {
            assert_eq!(role, "assistant");
        }
        other => panic!("unexpected second event: {other:?}"),
    }

    match &events[2] {
        ResponseEvent::Completed {
            response_id,
            token_usage,
            end_turn,
        } => {
            assert_eq!(response_id, "resp1");
            assert!(token_usage.is_none());
            assert!(end_turn.is_none());
        }
        other => panic!("unexpected third event: {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn responses_stream_control_chatter_does_not_extend_idle_timeout() -> Result<()> {
    let item = serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "partial"}]
        }
    });
    let control = serde_json::json!({"type": "keepalive"});
    let transport = DelayedSseTransport {
        chunks: vec![
            (Duration::ZERO, build_response_chunk(item)),
            (
                Duration::from_millis(20),
                build_response_chunk(control.clone()),
            ),
            (
                Duration::from_millis(20),
                build_response_chunk(control.clone()),
            ),
            (Duration::from_millis(20), build_response_chunk(control)),
        ],
    };
    let client = ResponsesClient::new(transport, provider("openai"), Arc::new(NoAuth));
    let mut stream = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
            Vec::new(),
        )
        .await?;

    loop {
        match stream.next().await {
            Some(Ok(ResponseEvent::OutputItemDone(_))) => break,
            Some(Ok(ResponseEvent::RateLimits(_))) => {}
            other => panic!("unexpected event before response item: {other:?}"),
        }
    }
    let error = tokio::time::timeout(Duration::from_millis(250), stream.next())
        .await
        .expect("logical response idle timeout should terminate the stream")
        .expect("stream should yield a timeout error")
        .expect_err("missing response.completed should be an error");
    assert!(matches!(
        error,
        model_service_api::ApiError::Stream(message)
            if message == "idle timeout waiting for SSE"
    ));

    Ok(())
}

#[tokio::test]
async fn responses_stream_progress_resets_idle_timeout() -> Result<()> {
    let item1 = serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "Hello"}]
        }
    });
    let item2 = serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "World"}]
        }
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "response": { "id": "resp-progress" }
    });
    let transport = DelayedSseTransport {
        chunks: vec![
            (Duration::ZERO, build_response_chunk(item1)),
            (Duration::from_millis(30), build_response_chunk(item2)),
            (Duration::from_millis(30), build_response_chunk(completed)),
        ],
    };
    let client = ResponsesClient::new(transport, provider("openai"), Arc::new(NoAuth));
    let mut stream = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
            Vec::new(),
        )
        .await?;

    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_millis(250), async {
        while let Some(event) = stream.next().await {
            events.push(event.expect("progressing stream should succeed"));
        }
    })
    .await
    .expect("progress should keep the logical response alive");

    let events: Vec<ResponseEvent> = events
        .into_iter()
        .filter(|event| !matches!(event, ResponseEvent::RateLimits(_)))
        .collect();
    assert_eq!(events.len(), 3);
    assert!(matches!(
        events.last(),
        Some(ResponseEvent::Completed { response_id, .. }) if response_id == "resp-progress"
    ));

    Ok(())
}

#[tokio::test]
async fn responses_stream_backpressure_preserves_terminal_error() -> Result<()> {
    let delta = serde_json::json!({
        "type": "response.output_text.delta",
        "delta": "x"
    });
    let body = build_responses_body((0..1601).map(|_| delta.clone()).collect());
    let transport = FixtureSseTransport::new(body);
    let client = ResponsesClient::new(transport, provider("openai"), Arc::new(NoAuth));
    let mut stream = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
            Vec::new(),
        )
        .await?;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut ordinary_events = 0;
    let error = loop {
        match stream
            .next()
            .await
            .expect("reserved terminal slot should contain an error")
        {
            Ok(_) => ordinary_events += 1,
            Err(error) => break error,
        }
    };
    assert_eq!(ordinary_events, 1600);
    assert!(matches!(
        error,
        model_service_api::ApiError::Stream(message)
            if message == "idle timeout waiting for SSE"
    ));

    Ok(())
}
