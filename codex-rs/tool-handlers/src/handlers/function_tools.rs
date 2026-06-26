use codex_protocol::config_types::ModeKind;
use codex_protocol::dynamic_tools::DynamicToolResponse;
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
use codex_tool_planning::REQUEST_USER_INPUT_TOOL_NAME;
use codex_tool_planning::ResponsesApiNamespace;
use codex_tool_planning::ResponsesApiNamespaceTool;
use codex_tool_planning::TOOL_SEARCH_TOOL_NAME;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSearchInfo;
use codex_tool_planning::ToolSearchSourceInfo;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::ViewImageToolOptions;
use codex_tool_planning::create_request_permissions_tool;
use codex_tool_planning::create_request_user_input_tool;
use codex_tool_planning::create_update_plan_tool;
use codex_tool_planning::create_view_image_tool;
use codex_tool_planning::default_namespace_description;
use codex_tool_planning::dynamic_tool_to_responses_api_tool;
use codex_tool_planning::normalize_request_user_input_args;
use codex_tool_planning::request_permissions_tool_description;
use codex_tool_planning::request_user_input_tool_description;
use codex_tool_planning::request_user_input_unavailable_message;
use codex_thread_api::ThreadRuntimeCapability;
use codex_tool_runtime_api::FunctionToolSession;
use codex_tool_runtime_api::FunctionToolTurn;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolExposure;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_utils_image::PromptImageMode;
use codex_utils_image::load_for_prompt_bytes;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::Value as JsonValue;

use crate::FunctionToolOutput;
use codex_tool_runtime::ToolInvocation;

pub struct PlanHandler;

impl PlanHandler {
    pub fn new() -> Self {
        Self
    }
}

pub struct RequestPermissionsHandler;

impl RequestPermissionsHandler {
    pub fn new() -> Self {
        Self
    }
}

pub struct RequestUserInputHandler {
    available_modes: Vec<ModeKind>,
}

impl RequestUserInputHandler {
    pub fn new(available_modes: Vec<ModeKind>) -> Self {
        Self { available_modes }
    }
}

pub struct DynamicToolHandler {
    tool_name: ToolName,
    spec: Option<ToolSpec>,
    exposure: ToolExposure,
    search_text: String,
}

pub struct ViewImageHandler<Turn> {
    _turn: std::marker::PhantomData<fn() -> Turn>,
    options: ViewImageToolOptions,
}

impl<Turn> Default for ViewImageHandler<Turn> {
    fn default() -> Self {
        Self {
            _turn: std::marker::PhantomData,
            options: ViewImageToolOptions {
                can_request_original_image_detail: false,
                include_environment_id: false,
            },
        }
    }
}

impl<Turn> ViewImageHandler<Turn> {
    pub fn new(options: ViewImageToolOptions) -> Self {
        Self {
            _turn: std::marker::PhantomData,
            options,
        }
    }
}

impl DynamicToolHandler {
    pub fn new(tool: &DynamicToolSpec) -> Option<Self> {
        let tool_name = ToolName::new(tool.namespace.clone(), tool.name.clone());
        let output_tool = dynamic_tool_to_responses_api_tool(tool).ok()?;
        let spec = match tool.namespace.as_ref() {
            Some(namespace) => ToolSpec::Namespace(ResponsesApiNamespace {
                name: namespace.clone(),
                description: default_namespace_description(namespace),
                tools: vec![ResponsesApiNamespaceTool::Function(output_tool)],
            }),
            None => ToolSpec::Function(output_tool),
        };
        Some(Self {
            tool_name,
            spec: Some(spec),
            exposure: if tool.defer_loading {
                ToolExposure::Deferred
            } else {
                ToolExposure::Direct
            },
            search_text: build_dynamic_search_text(tool),
        })
    }
}

pub struct PlanToolOutput;

pub struct ViewImageOutput {
    image_url: String,
    image_detail: Option<ImageDetail>,
}

const VIEW_IMAGE_UNSUPPORTED_MESSAGE: &str =
    "view_image is not allowed because you do not support image inputs";

#[derive(Deserialize)]
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

const PLAN_UPDATED_MESSAGE: &str = "Plan updated";

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

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>> for PlanHandler
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = PlanToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("update_plan")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_update_plan_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let arguments = function_arguments(metadata.payload, "update_plan")?;

            if turn.function_tool_collaboration_mode() == ModeKind::Plan {
                return Err(FunctionCallError::RespondToModel(
                    "update_plan is a TODO/checklist tool and is not allowed in Plan mode"
                        .to_string(),
                ));
            }

            let args: UpdatePlanArgs = parse_arguments(&arguments)?;
            session.function_tool_emit_plan_update(&turn, args).await;

            Ok(PlanToolOutput)
        })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for PlanHandler
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for RequestPermissionsHandler
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("request_permissions")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_request_permissions_tool(
            request_permissions_tool_description(),
        ))
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                cancellation_token,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let arguments = function_arguments(metadata.payload, "request_permissions")?;

            let cwd = turn.function_tool_cwd();
            #[allow(deprecated)]
            let mut args: RequestPermissionsArgs =
                parse_arguments_with_base_path(&arguments, &cwd)?;
            args.permissions = normalize_additional_permissions(args.permissions.into())
                .map(codex_protocol::request_permissions::RequestPermissionProfile::from)
                .map_err(FunctionCallError::RespondToModel)?;
            if args.permissions.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "request_permissions requires at least one permission".to_string(),
                ));
            }

            let response = session
                .function_tool_request_permissions(&turn, call_id, args, cancellation_token)
                .await
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "request_permissions was cancelled before receiving a response".to_string(),
                    )
                })?;

            let content = serde_json::to_string(&response).map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize request_permissions response: {err}"
                ))
            })?;

            Ok(FunctionToolOutput::from_text(content, Some(true)))
        })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for RequestPermissionsHandler
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for RequestUserInputHandler
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(REQUEST_USER_INPUT_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_request_user_input_tool(
            request_user_input_tool_description(&self.available_modes),
        ))
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let arguments = function_arguments(metadata.payload, REQUEST_USER_INPUT_TOOL_NAME)?;

            if turn.function_tool_is_non_root_agent() {
                return Err(FunctionCallError::RespondToModel(
                    "request_user_input can only be used by the root thread".to_string(),
                ));
            }

            let mode = session.function_tool_session_collaboration_mode().await;
            if let Some(message) =
                request_user_input_unavailable_message(mode, &self.available_modes)
            {
                return Err(FunctionCallError::RespondToModel(message));
            }

            let args: RequestUserInputArgs = parse_arguments(&arguments)?;
            let args = normalize_request_user_input_args(args)
                .map_err(FunctionCallError::RespondToModel)?;
            let response = session
                .function_tool_request_user_input(&turn, call_id, args)
                .await
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(format!(
                        "{REQUEST_USER_INPUT_TOOL_NAME} was cancelled before receiving a response"
                    ))
                })?;

            let content = serde_json::to_string(&response).map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize {REQUEST_USER_INPUT_TOOL_NAME} response: {err}"
                ))
            })?;

            Ok(FunctionToolOutput::from_text(content, Some(true)))
        })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for RequestUserInputHandler
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for DynamicToolHandler
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        self.tool_name.clone()
    }

    fn spec(&self) -> Option<ToolSpec> {
        self.spec.clone()
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let arguments = function_arguments(metadata.payload, "dynamic tool")?;

            let args: Value = parse_arguments(&arguments)?;
            let response = session
                .function_tool_request_dynamic_tool(&turn, call_id, self.tool_name.clone(), args)
                .await
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "dynamic tool call was cancelled before receiving a response".to_string(),
                    )
                })?;

            let DynamicToolResponse {
                content_items,
                success,
            } = response;
            let body = content_items
                .into_iter()
                .map(FunctionCallOutputContentItem::from)
                .collect::<Vec<_>>();
            Ok(FunctionToolOutput::from_content(body, Some(success)))
        })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for DynamicToolHandler
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
    fn search_info(&self) -> Option<ToolSearchInfo> {
        ToolSearchInfo::from_spec(
            self.search_text.clone(),
            self.spec.clone()?,
            Some(ToolSearchSourceInfo {
                name: "Dynamic tools".to_string(),
                description: Some("Tools provided by the current Codex thread.".to_string()),
            }),
        )
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for ViewImageHandler<Turn>
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn + ThreadRuntimeCapability,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = ViewImageOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("view_image")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_view_image_tool(self.options))
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            if !turn.function_tool_supports_image_input() {
                return Err(FunctionCallError::RespondToModel(
                    VIEW_IMAGE_UNSUPPORTED_MESSAGE.to_string(),
                ));
            }

            let call_id = metadata.call_id;
            let arguments = function_arguments(metadata.payload, "view_image")?;
            let ViewImageArgs {
                path,
                environment_id,
                detail,
            } = parse_arguments(&arguments)?;
            let detail = match detail.as_deref() {
                None => None,
                Some("original") => Some(ViewImageDetail::Original),
                Some(detail) => {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "view_image.detail only supports `original`; omit `detail` for default resized behavior, got `{detail}`"
                    )));
                }
            };

            let Some(turn_environment) = turn.resolve_environment(environment_id.as_deref())?
            else {
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
            let file_bytes = fs
                .read_file(&abs_path, Some(&sandbox))
                .await
                .map_err(|error| {
                    FunctionCallError::RespondToModel(format!(
                        "unable to read image at `{}`: {error}",
                        abs_path.display()
                    ))
                })?;
            let event_path = abs_path.clone();

            let can_request_original_detail =
                turn.function_tool_can_request_original_image_detail();
            let use_original_detail =
                can_request_original_detail && matches!(detail, Some(ViewImageDetail::Original));
            let image_mode = if use_original_detail {
                PromptImageMode::Original
            } else {
                PromptImageMode::ResizeToFit
            };
            let image_detail = Some(if use_original_detail {
                ImageDetail::Original
            } else {
                DEFAULT_IMAGE_DETAIL
            });

            let image = load_for_prompt_bytes(abs_path.as_path(), file_bytes, image_mode).map_err(
                |error| {
                    FunctionCallError::RespondToModel(format!(
                        "unable to process image at `{}`: {error}",
                        abs_path.display()
                    ))
                },
            )?;
            let image_url = image.into_data_url();

            session
                .function_tool_emit_image_view(&turn, call_id, event_path)
                .await;

            Ok(ViewImageOutput {
                image_url,
                image_detail,
            })
        })
    }
}

impl<Session, Turn, Tracker, DiffContext> ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext>
    for ViewImageHandler<Turn>
where
    Session: FunctionToolSession<Turn>,
    Turn: FunctionToolTurn + ThreadRuntimeCapability,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
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

    fn code_mode_result(&self, _payload: &ToolPayload) -> serde_json::Value {
        serde_json::json!({
            "image_url": self.image_url,
            "detail": self.image_detail
        })
    }
}

fn function_arguments(payload: ToolPayload, tool_name: &str) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(format!(
            "{tool_name} handler received unsupported payload"
        ))),
    }
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: &codex_utils_absolute_path::AbsolutePathBuf,
) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    let _guard = codex_utils_absolute_path::AbsolutePathBufGuard::new(base_path);
    parse_arguments(arguments)
}

fn build_dynamic_search_text(tool: &DynamicToolSpec) -> String {
    let namespace = tool.namespace.as_deref().unwrap_or("");
    let schema = serde_json::to_string(&tool.input_schema).unwrap_or_default();
    format!(
        "{namespace} {} {} {} {TOOL_SEARCH_TOOL_NAME}",
        tool.name, tool.description, schema
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem;
    use codex_protocol::dynamic_tools::DynamicToolResponse;
    use codex_protocol::request_permissions::RequestPermissionsResponse;
    use codex_protocol::request_user_input::RequestUserInputResponse;
    use codex_tool_types::ToolCallSource;
    use codex_tool_types::ToolInvocationMetadata;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone, Copy)]
    struct StubSession;

    #[derive(Clone, Copy)]
    struct StubTurn {
        non_root_agent: bool,
    }

    impl FunctionToolTurn for StubTurn {
        fn function_tool_collaboration_mode(&self) -> ModeKind {
            ModeKind::Default
        }

        fn function_tool_cwd(&self) -> codex_utils_absolute_path::AbsolutePathBuf {
            codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path("/tmp")
                .expect("absolute path")
        }

        fn function_tool_is_non_root_agent(&self) -> bool {
            self.non_root_agent
        }

        fn function_tool_supports_image_input(&self) -> bool {
            false
        }

        fn function_tool_can_request_original_image_detail(&self) -> bool {
            false
        }
    }

    impl FunctionToolSession<StubTurn> for StubSession {
        async fn function_tool_session_collaboration_mode(&self) -> ModeKind {
            ModeKind::Default
        }

        async fn function_tool_emit_plan_update(&self, _turn: &StubTurn, _args: UpdatePlanArgs) {}

        async fn function_tool_emit_image_view(
            &self,
            _turn: &StubTurn,
            _call_id: String,
            _path: codex_utils_absolute_path::AbsolutePathBuf,
        ) {
        }

        async fn function_tool_request_permissions(
            &self,
            _turn: &StubTurn,
            _call_id: String,
            _args: RequestPermissionsArgs,
            _cancellation_token: CancellationToken,
        ) -> Option<RequestPermissionsResponse> {
            None
        }

        async fn function_tool_request_user_input(
            &self,
            _turn: &StubTurn,
            _call_id: String,
            _args: RequestUserInputArgs,
        ) -> Option<RequestUserInputResponse> {
            None
        }

        async fn function_tool_request_dynamic_tool(
            &self,
            _turn: &StubTurn,
            _call_id: String,
            _tool_name: ToolName,
            _arguments: serde_json::Value,
        ) -> Option<DynamicToolResponse> {
            Some(DynamicToolResponse {
                content_items: vec![DynamicToolCallOutputContentItem::InputText {
                    text: "ok".to_string(),
                }],
                success: true,
            })
        }
    }

    #[test]
    fn dynamic_search_info_uses_tool_metadata_and_parameter_names() {
        let handler = DynamicToolHandler::new(&DynamicToolSpec {
            namespace: Some("codex_app".to_string()),
            name: "automation_update".to_string(),
            description: "Create or update automations.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "timezone": { "type": "string" },
                    "mode": { "type": "string" }
                }
            }),
            defer_loading: true,
        })
        .expect("dynamic handler should be created");

        let search_info = <DynamicToolHandler as ToolHandler<
            ToolInvocation<StubSession, StubTurn, ()>,
            (),
        >>::search_info(&handler)
        .expect("dynamic search info");

        assert!(
            search_info.entry.search_text.contains("codex_app"),
            "{}",
            search_info.entry.search_text
        );
        assert!(
            search_info.entry.search_text.contains("automation_update"),
            "{}",
            search_info.entry.search_text
        );
        assert!(
            search_info
                .entry
                .search_text
                .contains("Create or update automations."),
            "{}",
            search_info.entry.search_text
        );
        assert!(
            search_info.entry.search_text.contains("timezone"),
            "{}",
            search_info.entry.search_text
        );
        assert!(
            search_info.entry.search_text.contains("mode"),
            "{}",
            search_info.entry.search_text
        );
        assert!(
            search_info
                .entry
                .search_text
                .contains(TOOL_SEARCH_TOOL_NAME),
            "{}",
            search_info.entry.search_text
        );
        assert_eq!(
            search_info.source_info,
            Some(ToolSearchSourceInfo {
                name: "Dynamic tools".to_string(),
                description: Some("Tools provided by the current Codex thread.".to_string()),
            })
        );
    }

    #[tokio::test]
    async fn request_user_input_rejects_non_root_agent_threads() {
        let handler = RequestUserInputHandler::new(Vec::new());

        let result = handler
            .handle(ToolInvocation {
                session: StubSession,
                turn: StubTurn {
                    non_root_agent: true,
                },
                cancellation_token: CancellationToken::new(),
                tracker: (),
                metadata: ToolInvocationMetadata {
                    call_id: "call-1".to_string(),
                    tool_name: ToolName::plain(REQUEST_USER_INPUT_TOOL_NAME),
                    source: ToolCallSource::Direct,
                    payload: ToolPayload::Function {
                        arguments: json!({
                            "questions": [{
                                "header": "Hdr",
                                "question": "Pick one",
                                "id": "pick_one",
                                "options": [
                                    { "label": "A", "description": "A" },
                                    { "label": "B", "description": "B" }
                                ]
                            }]
                        })
                        .to_string(),
                    },
                },
            })
            .await;

        let Err(err) = result else {
            panic!("non-root agent thread should be rejected");
        };

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "request_user_input can only be used by the root thread".to_string(),
            )
        );
    }
}
