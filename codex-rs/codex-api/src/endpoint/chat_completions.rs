use crate::auth::SharedAuthProvider;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use http::HeaderMap;
use http::Method;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::instrument;

const RESPONSE_STREAM_CHANNEL_CAPACITY: usize = 16;

pub struct ChatCompletionsClient<T: HttpTransport> {
    session: EndpointSession<T>,
    path: ChatCompletionsPath,
}

#[derive(Debug, Clone, Copy)]
pub enum ChatCompletionsPath {
    AppendChatCompletions,
    FullEndpoint,
}

impl ChatCompletionsPath {
    fn as_path(self) -> &'static str {
        match self {
            Self::AppendChatCompletions => "chat/completions",
            Self::FullEndpoint => "",
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ChatTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    stream: bool,
}

#[derive(Debug, Serialize, PartialEq)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<ChatMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ChatToolCall>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(untagged)]
enum ChatMessageContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatContentPart {
    Text { text: String },
    ImageUrl { image_url: ChatImageUrl },
}

#[derive(Debug, Serialize, PartialEq)]
struct ChatImageUrl {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ChatToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: ChatFunctionCall,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ChatFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize, PartialEq)]
struct ChatTool {
    #[serde(rename = "type")]
    kind: String,
    function: ChatFunctionTool,
}

#[derive(Debug, Serialize, PartialEq)]
struct ChatFunctionTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    id: Option<String>,
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatAssistantMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatAssistantMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatToolCall>,
    function_call: Option<ChatFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
}

impl<T: HttpTransport> ChatCompletionsClient<T> {
    pub fn new(
        transport: T,
        provider: Provider,
        auth: SharedAuthProvider,
        path: ChatCompletionsPath,
    ) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
            path,
        }
    }

    pub fn with_telemetry(self, request: Option<Arc<dyn RequestTelemetry>>) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
            path: self.path,
        }
    }

    #[instrument(
        name = "chat_completions.create",
        level = "info",
        skip_all,
        fields(
            transport = "chat_completions_http",
            http.method = "POST",
            api.path = self.path.as_path()
        )
    )]
    pub async fn create(
        &self,
        request: ResponsesApiRequest,
        extra_headers: HeaderMap,
    ) -> Result<ResponseStream, ApiError> {
        let body = chat_request_from_responses(request)?;
        let body = serde_json::to_value(body).map_err(|e| {
            ApiError::Stream(format!("failed to encode chat completions request: {e}"))
        })?;
        let response = self
            .session
            .execute(Method::POST, self.path.as_path(), extra_headers, Some(body))
            .await?;
        let upstream_request_id = response
            .headers
            .get("x-request-id")
            .or_else(|| response.headers.get("x-ms-request-id"))
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);
        let response: ChatCompletionsResponse =
            serde_json::from_slice(&response.body).map_err(|e| {
                ApiError::Stream(format!(
                    "failed to decode chat completions response: {e}; body: {}",
                    String::from_utf8_lossy(&response.body)
                ))
            })?;

        Ok(stream_from_chat_response(response, upstream_request_id).await)
    }
}

fn chat_request_from_responses(
    request: ResponsesApiRequest,
) -> Result<ChatCompletionsRequest, ApiError> {
    let mut messages = Vec::new();
    if !request.instructions.trim().is_empty() {
        messages.push(ChatMessage::text("system", request.instructions));
    }

    for item in request.input {
        if let Some(message) = chat_message_from_response_item(item) {
            messages.push(message);
        }
    }

    let tools = chat_tools_from_responses(request.tools);
    let has_tools = !tools.is_empty();
    Ok(ChatCompletionsRequest {
        model: request.model,
        messages,
        tools,
        tool_choice: has_tools.then_some(request.tool_choice),
        parallel_tool_calls: has_tools.then_some(request.parallel_tool_calls),
        max_tokens: request.chat_completions_max_tokens,
        stream: false,
    })
}

fn chat_message_from_response_item(item: ResponseItem) -> Option<ChatMessage> {
    match item {
        ResponseItem::Message { role, content, .. } => {
            let role = if role == "developer" { "system" } else { &role };
            Some(ChatMessage {
                role: role.to_string(),
                content: chat_content_from_content_items(content),
                tool_call_id: None,
                tool_calls: Vec::new(),
            })
        }
        ResponseItem::FunctionCall {
            name,
            arguments,
            call_id,
            ..
        } => Some(ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_call_id: None,
            tool_calls: vec![ChatToolCall {
                id: call_id,
                kind: "function".to_string(),
                function: ChatFunctionCall { name, arguments },
            }],
        }),
        ResponseItem::FunctionCallOutput { call_id, output } => Some(ChatMessage {
            role: "tool".to_string(),
            content: Some(ChatMessageContent::Text(
                output.body.to_text().unwrap_or_default(),
            )),
            tool_call_id: Some(call_id),
            tool_calls: Vec::new(),
        }),
        ResponseItem::CustomToolCall {
            call_id,
            name,
            input,
            ..
        } => Some(ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_call_id: None,
            tool_calls: vec![ChatToolCall {
                id: call_id,
                kind: "function".to_string(),
                function: ChatFunctionCall {
                    name,
                    arguments: input,
                },
            }],
        }),
        ResponseItem::CustomToolCallOutput {
            call_id, output, ..
        } => Some(ChatMessage {
            role: "tool".to_string(),
            content: Some(ChatMessageContent::Text(
                output.body.to_text().unwrap_or_default(),
            )),
            tool_call_id: Some(call_id),
            tool_calls: Vec::new(),
        }),
        ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::WorkflowRunProgress { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::ThreadGoalUpdate { .. }
        | ResponseItem::InterAgentCommunication { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => None,
    }
}

fn chat_content_from_content_items(content: Vec<ContentItem>) -> Option<ChatMessageContent> {
    let mut parts = Vec::new();
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                parts.push(ChatContentPart::Text { text });
            }
            ContentItem::InputImage { image_url, detail } => {
                let detail = detail.map(|detail| {
                    serde_json::to_value(detail)
                        .ok()
                        .and_then(|value| value.as_str().map(ToString::to_string))
                        .unwrap_or_else(|| "auto".to_string())
                });
                parts.push(ChatContentPart::ImageUrl {
                    image_url: ChatImageUrl {
                        url: image_url,
                        detail,
                    },
                });
            }
        }
    }

    if parts.is_empty() {
        None
    } else if matches!(parts.as_slice(), [ChatContentPart::Text { .. }]) {
        let ChatContentPart::Text { text } = parts.remove(0) else {
            unreachable!("checked above")
        };
        Some(ChatMessageContent::Text(text))
    } else {
        Some(ChatMessageContent::Parts(parts))
    }
}

fn chat_tools_from_responses(tools: Vec<Value>) -> Vec<ChatTool> {
    tools
        .into_iter()
        .filter_map(|tool| {
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                return None;
            }
            let name = tool.get("name")?.as_str()?.to_string();
            Some(ChatTool {
                kind: "function".to_string(),
                function: ChatFunctionTool {
                    name,
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    parameters: tool.get("parameters").cloned(),
                    strict: tool.get("strict").and_then(Value::as_bool),
                },
            })
        })
        .collect()
}

async fn stream_from_chat_response(
    response: ChatCompletionsResponse,
    upstream_request_id: Option<String>,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel(RESPONSE_STREAM_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        let response_id = response.id.unwrap_or_default();
        if tx_event.send(Ok(ResponseEvent::Created)).await.is_err() {
            return;
        }

        let Some(choice) = response.choices.into_iter().next() else {
            let _ = tx_event
                .send(Err(ApiError::Stream(
                    "chat completions response did not include choices".to_string(),
                )))
                .await;
            return;
        };
        let finish_reason = choice.finish_reason.clone();
        let output_items = output_items_from_choice(choice);
        if let Some(finish_reason) = finish_reason.as_deref()
            && !matches!(finish_reason, "stop" | "tool_calls" | "function_call")
        {
            let _ = tx_event
                .send(Err(ApiError::Stream(format!(
                    "chat completions response finished with {finish_reason}"
                ))))
                .await;
            return;
        }
        for item in output_items {
            if tx_event
                .send(Ok(ResponseEvent::OutputItemDone(item)))
                .await
                .is_err()
            {
                return;
            }
        }

        let _ = tx_event
            .send(Ok(ResponseEvent::Completed {
                response_id,
                token_usage: response.usage.map(TokenUsage::from),
                end_turn: Some(true),
            }))
            .await;
    });

    ResponseStream {
        rx_event,
        upstream_request_id,
    }
}

fn output_items_from_choice(choice: ChatChoice) -> Vec<ResponseItem> {
    let mut items = Vec::new();
    if let Some(content) = choice.message.content
        && !content.is_empty()
    {
        items.push(ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText { text: content }],
            phase: None,
        });
    }

    items.extend(
        choice
            .message
            .tool_calls
            .into_iter()
            .filter(|tool_call| tool_call.kind == "function")
            .map(|tool_call| ResponseItem::FunctionCall {
                id: None,
                name: tool_call.function.name,
                namespace: None,
                arguments: tool_call.function.arguments,
                call_id: tool_call.id,
            }),
    );
    if let Some(function_call) = choice.message.function_call {
        items.push(ResponseItem::FunctionCall {
            id: None,
            name: function_call.name,
            namespace: None,
            arguments: function_call.arguments,
            call_id: "legacy_function_call".to_string(),
        });
    }
    items
}

impl ChatMessage {
    fn text(role: impl Into<String>, content: String) -> Self {
        Self {
            role: role.into(),
            content: Some(ChatMessageContent::Text(content)),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }
}

impl From<ChatUsage> for TokenUsage {
    fn from(value: ChatUsage) -> Self {
        Self {
            input_tokens: value.prompt_tokens.unwrap_or_default(),
            cached_input_tokens: 0,
            output_tokens: value.completion_tokens.unwrap_or_default(),
            reasoning_output_tokens: 0,
            total_tokens: value.total_tokens.unwrap_or_default(),
        }
    }
}

#[cfg(test)]
#[path = "chat_completions_tests.rs"]
mod tests;
