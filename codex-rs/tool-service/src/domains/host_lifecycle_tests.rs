use std::sync::Mutex;

use serde_json::json;
use thread_service::test_support;
use thread_service_api::ThreadSessionCapability;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolName;
use tool_service_api::ToolOutput;
use tool_service_api::ToolPayload;

use super::*;

#[derive(Default)]
struct FakeHostLifecycleRuntime {
    requests: Mutex<Vec<HostRelaunchRequest>>,
}

impl HostLifecycleToolRuntime for FakeHostLifecycleRuntime {
    fn request_client_relaunch<'a>(
        &'a self,
        request: HostRelaunchRequest,
    ) -> tool_service_api::ToolServiceFuture<'a, HostRelaunchResult> {
        Box::pin(async move {
            self.requests.lock().expect("requests mutex").push(request);
            HostRelaunchResult {
                status: HostRelaunchStatus::Accepted,
                accepted: true,
                relaunching: false,
                requested_mode: request.mode.clone(),
                executed_mode: Some(request.mode.clone()),
                message: "accepted".to_string(),
                reason: Some("runtime update".to_string()),
                resume_strategy: RESUME_STRATEGY.to_string(),
            }
        })
    }
}

fn tool_call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        call_id: "restart-call".to_string(),
        tool_name: ToolName::plain(REQUEST_RUNTIME_RESTART_TOOL_NAME),
        payload: ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    }
}

fn tool_output_json(result: &AnyToolResult) -> serde_json::Value {
    let response = result
        .result
        .to_response_item("restart-call", &result.payload);
    let protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } = response else {
        panic!("expected function call output");
    };
    let text = output.to_text().expect("function output text");
    serde_json::from_str(&text).expect("json response")
}

#[tokio::test]
async fn request_runtime_restart_dispatches_host_request_and_returns_result() {
    let (session, turn) = test_support::make_session_and_context().await;
    let runtime = Arc::new(FakeHostLifecycleRuntime::default());

    let result = dispatch(
        session.clone(),
        turn,
        Some(runtime.clone()),
        tool_call(json!({
            "reason": " runtime update ",
            "mode": "full",
        })),
    )
    .await
    .expect("dispatch should succeed");

    assert_eq!(result.call_id, "restart-call");
    let response_json = tool_output_json(&result);
    assert_eq!(response_json["status"], "accepted");
    assert_eq!(response_json["accepted"], true);
    assert_eq!(response_json["relaunching"], false);
    assert_eq!(response_json["requestedMode"], "full");
    assert_eq!(response_json["executedMode"], "full");
    assert_eq!(response_json["resumeStrategy"], RESUME_STRATEGY);

    let requests = runtime.requests.lock().expect("requests mutex");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].mode, HostRelaunchMode::Full);
    assert_eq!(requests[0].reason.as_deref(), Some("runtime update"));
    let expected_thread_id = session.conversation_id().to_string();
    assert_eq!(requests[0].requested_by_thread_id, Some(expected_thread_id));
    drop(requests);
}

#[tokio::test]
async fn request_runtime_restart_reports_unsupported_without_host_runtime() {
    let (session, turn) = test_support::make_session_and_context().await;

    let result = dispatch(session, turn, None, tool_call(json!({ "mode": "hot" })))
        .await
        .expect("unsupported is a model-visible result");
    let response_json = tool_output_json(&result);

    assert_eq!(response_json["status"], "unsupported");
    assert_eq!(response_json["accepted"], false);
    assert_eq!(response_json["relaunching"], false);
    assert_eq!(response_json["requestedMode"], "hot");
    assert_eq!(response_json["executedMode"], serde_json::Value::Null);
    assert_eq!(response_json["resumeStrategy"], RESUME_STRATEGY);
}

#[tokio::test]
async fn request_runtime_restart_rejects_missing_mode_before_host_request() {
    let (session, turn) = test_support::make_session_and_context().await;
    let runtime = Arc::new(FakeHostLifecycleRuntime::default());

    let error = dispatch(session, turn, Some(runtime.clone()), tool_call(json!({})))
        .await
        .expect_err("missing mode should fail");

    match error {
        FunctionCallError::RespondToModel(message) => {
            assert!(message.contains("missing field `mode`"));
        }
        other => panic!("expected model-visible parse error, got {other:?}"),
    }
    assert!(runtime.requests.lock().expect("requests mutex").is_empty());
}

#[tokio::test]
async fn request_runtime_restart_rejects_invalid_mode_before_host_request() {
    let (session, turn) = test_support::make_session_and_context().await;
    let runtime = Arc::new(FakeHostLifecycleRuntime::default());

    let error = dispatch(
        session,
        turn,
        Some(runtime.clone()),
        tool_call(json!({ "mode": "auto" })),
    )
    .await
    .expect_err("invalid mode should fail");

    match error {
        FunctionCallError::RespondToModel(message) => {
            assert!(message.contains("unknown variant `auto`"));
        }
        other => panic!("expected model-visible parse error, got {other:?}"),
    }
    assert!(runtime.requests.lock().expect("requests mutex").is_empty());
}

#[test]
fn request_runtime_restart_tool_schema_is_narrow() {
    let tool = create_request_runtime_restart_tool();
    let ToolSpec::Function(tool) = tool else {
        panic!("restart tool should be a function tool");
    };
    assert_eq!(tool.name, REQUEST_RUNTIME_RESTART_TOOL_NAME);
    assert!(tool.description.contains("Use after completing a feature"));
    assert!(tool.description.contains("fixing a bug"));
    assert!(tool.description.contains("frontend and backend builds"));
    assert!(
        !tool
            .description
            .contains("full Morpheus client/app-server relaunch")
    );
    assert!(!tool.description.contains("run shell commands"));
    assert!(!tool.description.contains("kill processes"));
    assert_eq!(tool.parameters.required, Some(vec!["mode".to_string()]));
    assert_eq!(tool.parameters.additional_properties, Some(false.into()));
    let properties = tool.parameters.properties.expect("properties");
    assert_eq!(properties.len(), 2);
    let mode = properties.get("mode").expect("mode property");
    assert_eq!(mode.enum_values, Some(vec![json!("hot"), json!("full")]));
    assert!(properties.contains_key("reason"));
    let output_schema = tool.output_schema.expect("output schema");
    assert_eq!(
        output_schema["required"],
        json!([
            "status",
            "accepted",
            "relaunching",
            "requestedMode",
            "executedMode",
            "message",
            "reason",
            "resumeStrategy"
        ])
    );
    assert_eq!(
        output_schema["properties"]["status"]["enum"],
        json!(["accepted", "unsupported", "failed"])
    );
    assert_eq!(
        output_schema["properties"]["status"]["description"],
        "Whether the host accepted, does not support, or failed the refresh request."
    );
    assert_eq!(
        output_schema["properties"]["reason"]["description"],
        "The normalized refresh reason."
    );
    assert!(
        output_schema["properties"]["relaunching"]["description"]
            .as_str()
            .expect("relaunching description")
            .contains("host has already confirmed")
    );
}
