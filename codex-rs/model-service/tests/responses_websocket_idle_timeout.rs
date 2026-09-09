use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use http::HeaderMap;
use model_service::ResponsesWebsocketClient;
use model_service_api::AuthProvider;
use model_service_api::Provider;
use model_service_api::ResponseEvent;
use model_service_api::ResponseProcessedWsRequest;
use model_service_api::ResponsesWsRequest;
use model_service_api::RetryConfig;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tungstenite::extensions::ExtensionsConfig;
use tungstenite::extensions::compression::deflate::DeflateConfig;
use tungstenite::protocol::WebSocketConfig;

#[derive(Clone, Default)]
struct NoAuth;

impl AuthProvider for NoAuth {
    fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
}

async fn spawn_responses_ws_server<Handler, Fut>(
    handler: Handler,
) -> (String, tokio::task::JoinHandle<()>)
where
    Handler:
        FnOnce(tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket listener");
    let addr = listener.local_addr().expect("read listener address");
    let server = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept websocket connection");
        let mut extensions = ExtensionsConfig::default();
        extensions.permessage_deflate = Some(DeflateConfig::default());
        let mut config = WebSocketConfig::default();
        config.extensions = extensions;
        let websocket = accept_async_with_config(stream, Some(config))
            .await
            .expect("complete websocket handshake");
        handler(websocket).await;
    });
    (format!("http://{addr}/v1"), server)
}

fn provider(base_url: String) -> Provider {
    Provider {
        name: "test".to_string(),
        base_url,
        query_params: Some(HashMap::new()),
        headers: HeaderMap::new(),
        retry: RetryConfig {
            max_attempts: 1,
            base_delay: Duration::from_millis(1),
            retry_429: false,
            retry_5xx: false,
            retry_transport: false,
        },
        stream_idle_timeout: Duration::from_millis(50),
    }
}

fn request() -> ResponsesWsRequest {
    ResponsesWsRequest::ResponseProcessed(ResponseProcessedWsRequest {
        response_id: "previous-response".to_string(),
    })
}

fn item_event(text: &str) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}]
            }
        })
        .to_string()
        .into(),
    )
}

fn completed_event(response_id: &str) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "response.completed",
            "response": {"id": response_id}
        })
        .to_string()
        .into(),
    )
}

fn text_delta_event() -> Message {
    Message::Text(
        serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "x"
        })
        .to_string()
        .into(),
    )
}

#[tokio::test]
async fn websocket_control_chatter_does_not_extend_idle_timeout() {
    let (base_url, server) = spawn_responses_ws_server(|mut websocket| async move {
        websocket
            .next()
            .await
            .expect("request message")
            .expect("valid request message");
        websocket
            .send(item_event("partial"))
            .await
            .expect("send response item");
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if websocket
                .send(Message::Text(
                    serde_json::json!({"type": "keepalive"}).to_string().into(),
                ))
                .await
                .is_err()
            {
                return;
            }
        }
    })
    .await;

    let client = ResponsesWebsocketClient::new(provider(base_url), Arc::new(NoAuth));
    let connection = client
        .connect(HeaderMap::new(), HeaderMap::new(), None, None)
        .await
        .expect("connect responses websocket");
    let mut stream = connection
        .stream_request(request(), false)
        .await
        .expect("start response stream");

    assert!(matches!(
        stream.next().await,
        Some(Ok(ResponseEvent::OutputItemDone(_)))
    ));
    let error = tokio::time::timeout(Duration::from_millis(250), stream.next())
        .await
        .expect("logical response idle timeout should terminate the stream")
        .expect("stream should yield a timeout error")
        .expect_err("missing response.completed should be an error");
    assert!(matches!(
        error,
        model_service_api::ApiError::Stream(message)
            if message == "idle timeout waiting for websocket"
    ));
    assert!(connection.is_closed().await);

    server.await.expect("websocket server task");
}

#[tokio::test]
async fn websocket_progress_resets_idle_timeout() {
    let (base_url, server) = spawn_responses_ws_server(|mut websocket| async move {
        websocket
            .next()
            .await
            .expect("request message")
            .expect("valid request message");
        websocket
            .send(item_event("Hello"))
            .await
            .expect("send first response item");
        tokio::time::sleep(Duration::from_millis(30)).await;
        websocket
            .send(item_event("World"))
            .await
            .expect("send second response item");
        tokio::time::sleep(Duration::from_millis(30)).await;
        websocket
            .send(completed_event("resp-progress"))
            .await
            .expect("send response completion");
    })
    .await;

    let client = ResponsesWebsocketClient::new(provider(base_url), Arc::new(NoAuth));
    let connection = client
        .connect(HeaderMap::new(), HeaderMap::new(), None, None)
        .await
        .expect("connect responses websocket");
    let mut stream = connection
        .stream_request(request(), false)
        .await
        .expect("start response stream");

    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_millis(250), async {
        while let Some(event) = stream.next().await {
            events.push(event.expect("progressing stream should succeed"));
        }
    })
    .await
    .expect("progress should keep the logical response alive");

    assert_eq!(events.len(), 3);
    assert!(matches!(
        events.last(),
        Some(ResponseEvent::Completed { response_id, .. }) if response_id == "resp-progress"
    ));

    server.await.expect("websocket server task");
}

#[tokio::test]
async fn websocket_backpressure_preserves_terminal_error_and_releases_connection() {
    let (base_url, server) = spawn_responses_ws_server(|mut websocket| async move {
        websocket
            .next()
            .await
            .expect("request message")
            .expect("valid request message");
        for _ in 0..1601 {
            if websocket.send(text_delta_event()).await.is_err() {
                return;
            }
        }
    })
    .await;

    let client = ResponsesWebsocketClient::new(provider(base_url), Arc::new(NoAuth));
    let connection = client
        .connect(HeaderMap::new(), HeaderMap::new(), None, None)
        .await
        .expect("connect responses websocket");
    let mut stream = connection
        .stream_request(request(), false)
        .await
        .expect("start response stream");

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(connection.is_closed().await);

    for _ in 0..1600 {
        assert!(matches!(
            stream.next().await,
            Some(Ok(ResponseEvent::OutputTextDelta(_)))
        ));
    }
    let error = stream
        .next()
        .await
        .expect("reserved terminal slot should contain an error")
        .expect_err("backpressure timeout should remain typed");
    assert!(matches!(
        error,
        model_service_api::ApiError::Stream(message)
            if message == "idle timeout waiting for websocket"
    ));

    server.await.expect("websocket server task");
}
