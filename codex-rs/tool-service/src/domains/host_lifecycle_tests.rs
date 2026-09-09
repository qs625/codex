use std::sync::Mutex;

use protocol::AgentPath;
use serde_json::json;
use thread_service::test_support;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolCallOutcome;
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
            self.requests
                .lock()
                .expect("requests mutex")
                .push(request.clone());
            HostRelaunchResult {
                request_id: request.request_id,
                status: HostRelaunchStatus::Accepted,
                accepted: true,
                restarting: false,
                message: "accepted".to_string(),
                reason: request.reason,
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

fn agent_path(path: &str) -> AgentPath {
    AgentPath::try_from(path).expect("valid test agent path")
}

#[tokio::test]
async fn accepted_restart_is_terminal_and_has_no_function_call_output() {
    let (session, turn) = test_support::make_session_and_context().await;
    let runtime = Arc::new(FakeHostLifecycleRuntime::default());

    let result = dispatch(
        Some(agent_path(AUTHORIZED_AGENT_PATH)),
        session.clone(),
        turn,
        Some(runtime.clone()),
        tool_call(json!({
            "reason": " runtime update ",
        })),
    )
    .await
    .expect("dispatch should succeed");

    assert!(matches!(result, ToolCallOutcome::FinishTurn));
    let requests = runtime.requests.lock().expect("requests mutex");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].request_id, "restart-call");
    assert_eq!(requests[0].reason.as_deref(), Some("runtime update"));
    assert_eq!(
        requests[0].requested_by_thread_id,
        Some(session.conversation_id().to_string())
    );
}

#[tokio::test]
async fn unsupported_restart_remains_a_model_visible_error_result() {
    let (session, turn) = test_support::make_session_and_context().await;

    let result = dispatch(
        Some(agent_path(AUTHORIZED_AGENT_PATH)),
        session,
        turn,
        None,
        tool_call(json!({})),
    )
    .await
    .expect("unsupported is model-visible");
    let ToolCallOutcome::ReturnToModel(result) = result else {
        panic!("unsupported restart must return an error result to the model");
    };
    let response_json = tool_output_json(&result);

    assert_eq!(response_json["status"], "unsupported");
    assert_eq!(response_json["requestId"], "restart-call");
    assert_eq!(response_json["accepted"], false);
    assert_eq!(response_json["restarting"], false);
    assert!(response_json.get("relaunching").is_none());
    assert!(response_json.get("requestedMode").is_none());
    assert!(response_json.get("executedMode").is_none());
    assert_eq!(response_json["resumeStrategy"], RESUME_STRATEGY);
}

#[tokio::test]
async fn legacy_mode_is_rejected_before_host_dispatch() {
    let (session, turn) = test_support::make_session_and_context().await;
    let runtime = Arc::new(FakeHostLifecycleRuntime::default());

    let error = match dispatch(
        Some(agent_path(AUTHORIZED_AGENT_PATH)),
        session,
        turn,
        Some(runtime.clone()),
        tool_call(json!({ "mode": "hot" })),
    )
    .await
    {
        Ok(_) => panic!("legacy mode should fail"),
        Err(error) => error,
    };

    match error {
        FunctionCallError::RespondToModel(message) => {
            assert!(message.contains("unknown field `mode`"));
        }
        other => panic!("expected model-visible parse error, got {other:?}"),
    }
    assert!(runtime.requests.lock().expect("requests mutex").is_empty());
}

#[tokio::test]
async fn unauthorized_paths_reject_forged_restart_before_host_dispatch() {
    for path in [
        None,
        Some("/self/child"),
        Some("/project"),
        Some("/project/child"),
        Some("/root"),
    ] {
        let (session, turn) = test_support::make_session_and_context().await;
        let runtime = Arc::new(FakeHostLifecycleRuntime::default());
        let current_agent_path = path.map(agent_path);

        let error = match dispatch(
            current_agent_path,
            session,
            turn,
            Some(runtime.clone()),
            tool_call(json!({})),
        )
        .await
        {
            Ok(_) => panic!("unauthorized restart should fail closed"),
            Err(error) => error,
        };

        match error {
            FunctionCallError::RespondToModel(message) => {
                assert_eq!(
                    message,
                    "request_runtime_restart is only authorized for the exact canonical agent path /self"
                );
            }
            other => panic!("expected typed authorization error, got {other:?}"),
        }
        assert!(
            runtime.requests.lock().expect("requests mutex").is_empty(),
            "unauthorized path {path:?} must not reach the host runtime"
        );
    }
}

#[test]
fn restart_tool_visibility_requires_exact_self_path() {
    let authorized_specs = specs_for_agent_path(Some(&agent_path(AUTHORIZED_AGENT_PATH)));
    assert_eq!(authorized_specs.len(), 1);
    assert!(matches!(
        &authorized_specs[0],
        ToolSpec::Function(tool) if tool.name == REQUEST_RUNTIME_RESTART_TOOL_NAME
    ));
    let authorized_nested_names =
        crate::planning::collect_code_mode_tool_definitions(&authorized_specs)
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>();
    assert!(
        authorized_nested_names
            .iter()
            .any(|name| name == REQUEST_RUNTIME_RESTART_TOOL_NAME)
    );

    for path in [
        None,
        Some("/self/child"),
        Some("/project"),
        Some("/project/child"),
        Some("/root"),
    ] {
        let current_agent_path = path.map(agent_path);
        let specs = specs_for_agent_path(current_agent_path.as_ref());
        assert!(
            specs.is_empty(),
            "unauthorized path {path:?} must not receive the restart spec"
        );
        let nested_names = crate::planning::collect_code_mode_tool_definitions(&specs)
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>();
        assert!(
            !nested_names
                .iter()
                .any(|name| name == REQUEST_RUNTIME_RESTART_TOOL_NAME),
            "unauthorized path {path:?} must not receive the restart tool in code mode"
        );
    }
}

#[test]
fn request_runtime_restart_tool_schema_is_narrow() {
    let tool = create_request_runtime_restart_tool();
    let ToolSpec::Function(tool) = tool else {
        panic!("restart tool should be a function tool");
    };
    assert_eq!(tool.name, REQUEST_RUNTIME_RESTART_TOOL_NAME);
    assert!(tool.description.contains("Use after completing a feature"));
    assert!(tool.description.contains("frontend and backend builds"));
    assert!(tool.description.contains("complete Runtime Capsule restart"));
    assert!(!tool.description.contains("hot"));
    assert!(!tool.description.contains("full"));
    assert!(!tool.description.contains("run shell commands"));
    assert!(!tool.description.contains("kill processes"));
    assert_eq!(tool.parameters.required, Some(vec![]));
    assert_eq!(tool.parameters.additional_properties, Some(false.into()));
    let properties = tool.parameters.properties.expect("properties");
    assert_eq!(properties.len(), 1);
    assert!(!properties.contains_key("mode"));
    assert!(properties.contains_key("reason"));
}
