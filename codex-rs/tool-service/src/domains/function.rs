use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use codex_protocol::config_types::ModeKind;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::DEFAULT_IMAGE_DETAIL;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ImageDetail;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::request_permissions::RequestPermissionsArgs;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_sandboxing_api::policy_transforms::normalize_additional_permissions;
use codex_thread_api::FunctionToolCapability;
use codex_tool_planning::REQUEST_USER_INPUT_TOOL_NAME;
use codex_tool_planning::ResponsesApiNamespace;
use codex_tool_planning::ResponsesApiNamespaceTool;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::ViewImageToolOptions;
use codex_tool_planning::create_request_permissions_tool;
use codex_tool_planning::create_request_user_input_tool;
use codex_tool_planning::create_test_sync_tool;
use codex_tool_planning::create_update_plan_tool;
use codex_tool_planning::create_view_image_tool;
use codex_tool_planning::default_namespace_description;
use codex_tool_planning::dynamic_tool_to_responses_api_tool;
use codex_tool_planning::hosted_model_tool_specs;
use codex_tool_planning::normalize_request_user_input_args;
use codex_tool_planning::request_permissions_tool_description;
use codex_tool_planning::request_user_input_tool_description;
use codex_tool_planning::request_user_input_unavailable_message;
use codex_tool_runtime::FunctionToolOutput;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_service_api::AnyToolResult;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use codex_utils_image::PromptImageMode;
use codex_utils_image::load_for_prompt_bytes;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::Value as JsonValue;
use tokio::sync::Barrier;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::context::TypedToolSpecRequest;

const UPDATE_PLAN_TOOL_NAME: &str = "update_plan";
const REQUEST_PERMISSIONS_TOOL_NAME: &str = "request_permissions";
const TEST_SYNC_TOOL_NAME: &str = "test_sync_tool";
const VIEW_IMAGE_UNSUPPORTED_MESSAGE: &str =
    "view_image is not allowed because you do not support image inputs";
const PLAN_UPDATED_MESSAGE: &str = "Plan updated";
const DEFAULT_TIMEOUT_MS: u64 = 1_000;

static BARRIERS: OnceLock<tokio::sync::Mutex<HashMap<String, BarrierState>>> = OnceLock::new();

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    let mut specs = vec![
        create_update_plan_tool(),
        create_request_permissions_tool(request_permissions_tool_description()),
        create_request_user_input_tool(request_user_input_tool_description(
            &request.config.request_user_input_available_modes,
        )),
        create_view_image_tool(ViewImageToolOptions {
            can_request_original_image_detail: false,
            include_environment_id: matches!(
                request.config.environment_mode,
                codex_tool_planning::ToolEnvironmentMode::Multiple
            ),
        }),
        create_test_sync_tool(),
    ];

    specs.extend(hosted_model_tool_specs(request.config));
    specs.extend(request.params.dynamic_tools.iter().filter_map(dynamic_tool_to_spec));
    specs
}

pub(crate) fn owns_tool_name(request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    if request
        .params
        .dynamic_tools
        .iter()
        .any(|tool| tool_name_matches_dynamic_spec(tool_name, tool))
    {
        return true;
    }

    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            UPDATE_PLAN_TOOL_NAME
                | REQUEST_PERMISSIONS_TOOL_NAME
                | REQUEST_USER_INPUT_TOOL_NAME
                | TEST_SYNC_TOOL_NAME
                | codex_protocol::models::VIEW_IMAGE_TOOL_NAME
        )
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(request: &TypedToolSpecRequest<'_>, call: &ToolCall) -> bool {
    matches!(
        call.tool_name.name.as_str(),
        REQUEST_PERMISSIONS_TOOL_NAME
            | REQUEST_USER_INPUT_TOOL_NAME
            | TEST_SYNC_TOOL_NAME
            | codex_protocol::models::VIEW_IMAGE_TOOL_NAME
    ) || request
        .params
        .dynamic_tools
        .iter()
        .any(|tool| tool_name_matches_dynamic_spec(&call.tool_name, tool))
}

pub(crate) async fn dispatch(
    turn: Arc<codex_thread_runtime::ThreadTurnContext>,
    request_user_input_available_modes: Vec<ModeKind>,
    dynamic_tools: Vec<DynamicToolSpec>,
    cancellation_token: CancellationToken,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result: Box<dyn ToolOutput> = match call.tool_name.name.as_str() {
        UPDATE_PLAN_TOOL_NAME => Box::new(dispatch_update_plan(turn.as_ref(), &call).await?),
        REQUEST_PERMISSIONS_TOOL_NAME => Box::new(
            dispatch_request_permissions(turn.as_ref(), cancellation_token, &call).await?,
        ),
        REQUEST_USER_INPUT_TOOL_NAME => Box::new(
            dispatch_request_user_input(
                turn.as_ref(),
                &request_user_input_available_modes,
                &call,
            )
            .await?,
        ),
        codex_protocol::models::VIEW_IMAGE_TOOL_NAME => {
            Box::new(dispatch_view_image(turn.as_ref(), &call).await?)
        }
        TEST_SYNC_TOOL_NAME => Box::new(dispatch_test_sync(&call).await?),
        _ if tool_name_matches_dynamic_specs(&call.tool_name, &dynamic_tools) => {
            Box::new(dispatch_dynamic_tool(turn.as_ref(), &call).await?)
        }
        _ => {
            return Err(FunctionCallError::Fatal(format!(
                "unsupported function tool {}",
                call.tool_name
            )));
        }
    };

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result,
        post_tool_use_payload: None,
    })
}

async fn dispatch_update_plan(
    turn: &impl FunctionToolCapability,
    call: &ToolCall,
) -> Result<PlanToolOutput, FunctionCallError> {
    if turn.function_tool_collaboration_mode() == ModeKind::Plan {
        return Err(FunctionCallError::RespondToModel(
            "update_plan is a TODO/checklist tool and is not allowed in Plan mode".to_string(),
        ));
    }

    turn.function_tool_emit_plan_update(parse_arguments::<UpdatePlanArgs>(call)?)
        .await;
    Ok(PlanToolOutput)
}

async fn dispatch_request_permissions(
    turn: &impl FunctionToolCapability,
    cancellation_token: CancellationToken,
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    #[allow(deprecated)]
    let mut args: RequestPermissionsArgs = parse_arguments_with_base_path(
        call.function_arguments()?,
        &turn.function_tool_cwd(),
    )?;
    args.permissions = normalize_additional_permissions(args.permissions.into())
        .map(codex_protocol::request_permissions::RequestPermissionProfile::from)
        .map_err(FunctionCallError::RespondToModel)?;
    if args.permissions.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "request_permissions requires at least one permission".to_string(),
        ));
    }

    let response = turn
        .function_tool_request_permissions(call.call_id.clone(), args, cancellation_token)
        .await
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "request_permissions was cancelled before receiving a response".to_string(),
            )
        })?;

    serde_json::to_string(&response)
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize request_permissions response: {err}"
            ))
        })
}

async fn dispatch_request_user_input(
    turn: &impl FunctionToolCapability,
    available_modes: &[ModeKind],
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    if turn.function_tool_is_non_root_agent() {
        return Err(FunctionCallError::RespondToModel(
            "request_user_input can only be used by the root thread".to_string(),
        ));
    }

    let mode = turn.function_tool_session_collaboration_mode().await;
    if let Some(message) = request_user_input_unavailable_message(mode, available_modes) {
        return Err(FunctionCallError::RespondToModel(message));
    }

    let args = normalize_request_user_input_args(parse_arguments::<RequestUserInputArgs>(call)?)
        .map_err(FunctionCallError::RespondToModel)?;
    let response = turn
        .function_tool_request_user_input(call.call_id.clone(), args)
        .await
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(format!(
                "{REQUEST_USER_INPUT_TOOL_NAME} was cancelled before receiving a response"
            ))
        })?;

    serde_json::to_string(&response)
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize {REQUEST_USER_INPUT_TOOL_NAME} response: {err}"
            ))
        })
}

async fn dispatch_dynamic_tool(
    turn: &impl FunctionToolCapability,
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let response = turn
        .function_tool_request_dynamic_tool(
            call.call_id.clone(),
            call.tool_name.clone(),
            parse_arguments::<Value>(call)?,
        )
        .await
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "dynamic tool call was cancelled before receiving a response".to_string(),
            )
        })?;

    Ok(FunctionToolOutput::from_content(
        response
            .content_items
            .into_iter()
            .map(FunctionCallOutputContentItem::from)
            .collect(),
        Some(response.success),
    ))
}

async fn dispatch_view_image(
    turn: &impl FunctionToolCapability,
    call: &ToolCall,
) -> Result<ViewImageOutput, FunctionCallError> {
    if !turn.function_tool_supports_image_input() {
        return Err(FunctionCallError::RespondToModel(
            VIEW_IMAGE_UNSUPPORTED_MESSAGE.to_string(),
        ));
    }

    let ViewImageArgs {
        path,
        environment_id,
        detail,
    } = parse_arguments(call)?;
    let detail = match detail.as_deref() {
        None => None,
        Some("original") => Some(ViewImageDetail::Original),
        Some(detail) => {
            return Err(FunctionCallError::RespondToModel(format!(
                "view_image.detail only supports `original`; omit `detail` for default resized behavior, got `{detail}`"
            )));
        }
    };

    let Some(turn_environment) = turn.resolve_environment(environment_id.as_deref())? else {
        return Err(FunctionCallError::RespondToModel(
            "view_image is unavailable in this session".to_string(),
        ));
    };
    let cwd = turn_environment.cwd.clone();
    let abs_path = cwd.join(path);
    let sandbox = turn.file_system_sandbox_context(/*additional_permissions*/ None, &cwd);
    let fs = turn_environment.environment.filesystem();

    let metadata = fs
        .get_metadata(&abs_path, Some(&sandbox))
        .await
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "unable to locate image at `{}`: {error}",
                abs_path.display()
            ))
        })?;
    if !metadata.is_file {
        return Err(FunctionCallError::RespondToModel(format!(
            "image path `{}` is not a file",
            abs_path.display()
        )));
    }

    let file_bytes = fs.read_file(&abs_path, Some(&sandbox)).await.map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "unable to read image at `{}`: {error}",
            abs_path.display()
        ))
    })?;

    let use_original_detail =
        turn.can_request_original_image_detail() && matches!(detail, Some(ViewImageDetail::Original));
    let image_detail = Some(if use_original_detail {
        ImageDetail::Original
    } else {
        DEFAULT_IMAGE_DETAIL
    });
    let image_mode = if use_original_detail {
        PromptImageMode::Original
    } else {
        PromptImageMode::ResizeToFit
    };
    let image = load_for_prompt_bytes(abs_path.as_path(), file_bytes, image_mode).map_err(
        |error| {
            FunctionCallError::RespondToModel(format!(
                "unable to process image at `{}`: {error}",
                abs_path.display()
            ))
        },
    )?;

    turn.function_tool_emit_image_view(call.call_id.clone(), abs_path)
        .await;

    Ok(ViewImageOutput {
        image_url: image.into_data_url(),
        image_detail,
    })
}

async fn dispatch_test_sync(call: &ToolCall) -> Result<FunctionToolOutput, FunctionCallError> {
    let args: TestSyncArgs = parse_arguments(call)?;

    if let Some(delay) = args.sleep_before_ms
        && delay > 0
    {
        sleep(Duration::from_millis(delay)).await;
    }
    if let Some(barrier) = args.barrier {
        wait_on_barrier(barrier).await?;
    }
    if let Some(delay) = args.sleep_after_ms
        && delay > 0
    {
        sleep(Duration::from_millis(delay)).await;
    }

    Ok(FunctionToolOutput::from_text("ok".to_string(), Some(true)))
}

fn tool_name_matches_dynamic_spec(tool_name: &ToolName, spec: &DynamicToolSpec) -> bool {
    tool_name.name == spec.name && tool_name.namespace.as_deref() == spec.namespace.as_deref()
}

fn tool_name_matches_dynamic_specs(tool_name: &ToolName, specs: &[DynamicToolSpec]) -> bool {
    specs.iter()
        .any(|tool| tool_name_matches_dynamic_spec(tool_name, tool))
}

fn dynamic_tool_to_spec(tool: &DynamicToolSpec) -> Option<ToolSpec> {
    let output_tool = dynamic_tool_to_responses_api_tool(tool).ok()?;
    Some(match tool.namespace.as_ref() {
        Some(namespace) => ToolSpec::Namespace(ResponsesApiNamespace {
            name: namespace.clone(),
            description: default_namespace_description(namespace),
            tools: vec![ResponsesApiNamespaceTool::Function(output_tool)],
        }),
        None => ToolSpec::Function(output_tool),
    })
}

fn parse_arguments<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(call.function_arguments()?).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to parse {} arguments: {err}",
            call.tool_name
        ))
    })
}

fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: &codex_utils_absolute_path::AbsolutePathBuf,
) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    let _guard = AbsolutePathBufGuard::new(base_path);
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn barrier_map() -> &'static tokio::sync::Mutex<HashMap<String, BarrierState>> {
    BARRIERS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

async fn wait_on_barrier(args: BarrierArgs) -> Result<(), FunctionCallError> {
    if args.participants == 0 {
        return Err(FunctionCallError::RespondToModel(
            "barrier participants must be greater than zero".to_string(),
        ));
    }
    if args.timeout_ms == 0 {
        return Err(FunctionCallError::RespondToModel(
            "barrier timeout must be greater than zero".to_string(),
        ));
    }

    let barrier_id = args.id.clone();
    let barrier = {
        let mut map = barrier_map().lock().await;
        match map.entry(barrier_id.clone()) {
            Entry::Occupied(entry) => {
                let state = entry.get();
                if state.participants != args.participants {
                    let existing = state.participants;
                    return Err(FunctionCallError::RespondToModel(format!(
                        "barrier {barrier_id} already registered with {existing} participants"
                    )));
                }
                state.barrier.clone()
            }
            Entry::Vacant(entry) => {
                let barrier = Arc::new(Barrier::new(args.participants));
                entry.insert(BarrierState {
                    barrier: barrier.clone(),
                    participants: args.participants,
                });
                barrier
            }
        }
    };

    let timeout = Duration::from_millis(args.timeout_ms);
    let wait_result = tokio::time::timeout(timeout, barrier.wait())
        .await
        .map_err(|_| {
            FunctionCallError::RespondToModel("test_sync_tool barrier wait timed out".to_string())
        })?;

    if wait_result.is_leader() {
        let mut map = barrier_map().lock().await;
        if let Some(state) = map.get(&barrier_id)
            && Arc::ptr_eq(&state.barrier, &barrier)
        {
            map.remove(&barrier_id);
        }
    }

    Ok(())
}

struct PlanToolOutput;

impl ToolOutput for PlanToolOutput {
    fn log_preview(&self) -> String {
        PLAN_UPDATED_MESSAGE.to_string()
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, _payload: &ToolPayload) -> ResponseInputItem {
        let mut output = FunctionCallOutputPayload::from_text(PLAN_UPDATED_MESSAGE.to_string());
        output.success = Some(true);
        ResponseInputItem::FunctionCallOutput {
            call_id: call_id.to_string(),
            output,
        }
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        JsonValue::Object(serde_json::Map::new())
    }
}

struct ViewImageOutput {
    image_url: String,
    image_detail: Option<ImageDetail>,
}

impl ToolOutput for ViewImageOutput {
    fn log_preview(&self) -> String {
        self.image_url.clone()
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, _payload: &ToolPayload) -> ResponseInputItem {
        let body =
            FunctionCallOutputBody::ContentItems(vec![FunctionCallOutputContentItem::InputImage {
                image_url: self.image_url.clone(),
                detail: self.image_detail,
            }]);
        let output = FunctionCallOutputPayload {
            body,
            success: Some(true),
        };
        ResponseInputItem::FunctionCallOutput {
            call_id: call_id.to_string(),
            output,
        }
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        serde_json::json!({
            "image_url": self.image_url,
            "detail": self.image_detail
        })
    }
}

struct BarrierState {
    barrier: Arc<Barrier>,
    participants: usize,
}

#[derive(Debug, Deserialize)]
struct BarrierArgs {
    id: String,
    participants: usize,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
struct TestSyncArgs {
    #[serde(default)]
    sleep_before_ms: Option<u64>,
    #[serde(default)]
    sleep_after_ms: Option<u64>,
    #[serde(default)]
    barrier: Option<BarrierArgs>,
}

#[derive(Debug, Deserialize)]
struct ViewImageArgs {
    path: String,
    #[serde(default)]
    environment_id: Option<String>,
    detail: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ViewImageDetail {
    Original,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}
