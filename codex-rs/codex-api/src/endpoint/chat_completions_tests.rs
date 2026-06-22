use super::*;
use assert_matches::assert_matches;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputPayload;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn maps_responses_request_to_chat_completions_body() {
    let request = ResponsesApiRequest {
        model: "gpt-test".to_string(),
        instructions: "Be concise".to_string(),
        input: vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hi".to_string(),
                }],
                phase: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "get_horoscope".to_string(),
                namespace: None,
                arguments: r#"{"sign":"Aquarius"}"#.to_string(),
                call_id: "call_1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call_1".to_string(),
                output: FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text("great day".to_string()),
                    success: None,
                },
            },
        ],
        tools: vec![
            json!({
                "type": "function",
                "name": "get_horoscope",
                "description": "Get horoscope",
                "strict": false,
                "parameters": {"type": "object"}
            }),
            json!({"type": "web_search"}),
        ],
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: false,
        stream: true,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
        chat_completions_max_tokens: Some(500),
    };

    let mapped = chat_request_from_responses(request).expect("request maps");
    assert_eq!(
        serde_json::to_value(mapped).expect("serialize"),
        json!({
            "model": "gpt-test",
            "messages": [
                {"role": "system", "content": "Be concise"},
                {"role": "user", "content": "hi"},
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_horoscope",
                            "arguments": "{\"sign\":\"Aquarius\"}"
                        }
                    }]
                },
                {"role": "tool", "content": "great day", "tool_call_id": "call_1"}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_horoscope",
                    "description": "Get horoscope",
                    "parameters": {"type": "object"},
                    "strict": false
                }
            }],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "max_tokens": 500,
            "stream": false
        })
    );
}

#[test]
fn omits_tool_options_when_no_function_tools_are_sent() {
    let request = ResponsesApiRequest {
        model: "gpt-test".to_string(),
        instructions: String::new(),
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "hi".to_string(),
            }],
            phase: None,
        }],
        tools: Vec::new(),
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
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

    let mapped = chat_request_from_responses(request).expect("request maps");
    assert_eq!(
        serde_json::to_value(mapped).expect("serialize"),
        json!({
            "model": "gpt-test",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false
        })
    );
}

#[test]
fn maps_chat_response_to_response_items() {
    let response: ChatCompletionsResponse = serde_json::from_value(json!({
        "id": "chatcmpl_1",
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": "checking",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "get_horoscope",
                        "arguments": "{\"sign\":\"Aquarius\"}"
                    }
                }]
            }
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 3,
            "total_tokens": 13
        }
    }))
    .expect("decode");

    let items = output_items_from_choice(response.choices.into_iter().next().unwrap());
    assert_eq!(
        items,
        vec![
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "checking".to_string()
                }],
                phase: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "get_horoscope".to_string(),
                namespace: None,
                arguments: r#"{"sign":"Aquarius"}"#.to_string(),
                call_id: "call_1".to_string(),
            },
        ]
    );
}

#[test]
fn accepts_legacy_function_call_finish_reason() {
    let response: ChatCompletionsResponse = serde_json::from_value(json!({
        "id": "chatcmpl_1",
        "choices": [{
            "finish_reason": "function_call",
            "message": {
                "role": "assistant",
                "content": null,
                "function_call": {
                    "name": "get_horoscope",
                    "arguments": "{}"
                }
            }
        }]
    }))
    .expect("decode");

    let items = output_items_from_choice(response.choices.into_iter().next().unwrap());
    assert_eq!(
        items,
        vec![ResponseItem::FunctionCall {
            id: None,
            name: "get_horoscope".to_string(),
            namespace: None,
            arguments: "{}".to_string(),
            call_id: "legacy_function_call".to_string(),
        }]
    );
}

#[tokio::test]
async fn chat_response_stream_uses_complete_items_without_text_delta() {
    let response: ChatCompletionsResponse = serde_json::from_value(json!({
        "id": "chatcmpl_1",
        "choices": [{
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "content": "done"
            }
        }]
    }))
    .expect("decode");

    let mut stream = stream_from_chat_response(response, Some("req_1".to_string())).await;
    assert_eq!(stream.upstream_request_id(), Some("req_1"));
    assert_matches!(
        stream.next().await.expect("created").expect("event"),
        ResponseEvent::Created
    );
    assert_matches!(
        stream
            .next()
            .await
            .expect("output item")
            .expect("event"),
        ResponseEvent::OutputItemDone(ResponseItem::Message { role, content, .. })
            if role == "assistant"
                && content == vec![ContentItem::OutputText { text: "done".to_string() }]
    );
    assert_matches!(
        stream.next().await.expect("completed").expect("event"),
        ResponseEvent::Completed {
            response_id,
            token_usage: None,
            end_turn: Some(true),
        } if response_id == "chatcmpl_1"
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn chat_response_stream_errors_on_empty_choices() {
    let response: ChatCompletionsResponse = serde_json::from_value(json!({
        "id": "chatcmpl_1",
        "choices": []
    }))
    .expect("decode");

    let mut stream = stream_from_chat_response(response, None).await;
    assert_matches!(
        stream.next().await.expect("created").expect("event"),
        ResponseEvent::Created
    );
    assert_matches!(
        stream.next().await.expect("error"),
        Err(ApiError::Stream(message)) if message.contains("choices")
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn chat_response_stream_errors_on_truncated_finish_reason() {
    let response: ChatCompletionsResponse = serde_json::from_value(json!({
        "id": "chatcmpl_1",
        "choices": [{
            "finish_reason": "length",
            "message": {
                "role": "assistant",
                "content": "partial"
            }
        }]
    }))
    .expect("decode");

    let mut stream = stream_from_chat_response(response, None).await;
    assert_matches!(
        stream.next().await.expect("created").expect("event"),
        ResponseEvent::Created
    );
    assert_matches!(
        stream.next().await.expect("error"),
        Err(ApiError::Stream(message)) if message.contains("length")
    );
    assert!(stream.next().await.is_none());
}
