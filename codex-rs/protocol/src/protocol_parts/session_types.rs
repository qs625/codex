/// Open/close tags for special user-input blocks. Used across crates to avoid
/// duplicated hardcoded strings.
pub const USER_INSTRUCTIONS_OPEN_TAG: &str = "<user_instructions>";
pub const USER_INSTRUCTIONS_CLOSE_TAG: &str = "</user_instructions>";
pub const ENVIRONMENT_CONTEXT_OPEN_TAG: &str = "<environment_context>";
pub const ENVIRONMENT_CONTEXT_CLOSE_TAG: &str = "</environment_context>";
pub const APPS_INSTRUCTIONS_OPEN_TAG: &str = "<apps_instructions>";
pub const APPS_INSTRUCTIONS_CLOSE_TAG: &str = "</apps_instructions>";
pub const SKILLS_INSTRUCTIONS_OPEN_TAG: &str = "<skills_instructions>";
pub const SKILLS_INSTRUCTIONS_CLOSE_TAG: &str = "</skills_instructions>";
pub const PLUGINS_INSTRUCTIONS_OPEN_TAG: &str = "<plugins_instructions>";
pub const PLUGINS_INSTRUCTIONS_CLOSE_TAG: &str = "</plugins_instructions>";
pub const COLLABORATION_MODE_OPEN_TAG: &str = "<collaboration_mode>";
pub const COLLABORATION_MODE_CLOSE_TAG: &str = "</collaboration_mode>";
pub const REALTIME_CONVERSATION_OPEN_TAG: &str = "<realtime_conversation>";
pub const REALTIME_CONVERSATION_CLOSE_TAG: &str = "</realtime_conversation>";
pub const USER_MESSAGE_BEGIN: &str = "## My request for Codex:";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct TurnEnvironmentSelection {
    pub environment_id: String,
    pub cwd: AbsolutePathBuf,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, TS)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct GitSha(pub String);

impl GitSha {
    pub fn new(sha: &str) -> Self {
        Self(sha.to_string())
    }
}

/// Submission Queue Entry - requests from user
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Submission {
    /// Unique id for this Submission to correlate with Events
    pub id: String,
    /// Payload
    pub op: Op,
    /// Optional W3C trace carrier propagated across async submission handoffs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<W3cTraceContext>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct W3cTraceContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub traceparent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tracestate: Option<String>,
}

/// Config payload for refreshing MCP servers.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct McpServerRefreshConfig {
    pub mcp_servers: Value,
    pub mcp_oauth_credentials_store_mode: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct ConversationStartParams {
    /// Selects whether the realtime session should produce text or audio output.
    pub output_modality: RealtimeOutputModality,
    #[serde(
        default,
        deserialize_with = "conversation_start_prompt_serde::deserialize",
        serialize_with = "conversation_start_prompt_serde::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub prompt: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realtime_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<ConversationStartTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<RealtimeVoice>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type")]
pub enum ConversationStartTransport {
    Websocket,
    Webrtc { sdp: String },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeOutputModality {
    Text,
    Audio,
}

mod conversation_start_prompt_serde {
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serialize;
    use serde::Serializer;

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer).map(Some)
    }

    pub(crate) fn serialize<S>(
        value: &Option<Option<String>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            None => serializer.serialize_none(),
            Some(prompt) => prompt.serialize(serializer),
        }
    }
}

#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash, JsonSchema, TS, Ord, PartialOrd,
)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum RealtimeVoice {
    Alloy,
    Arbor,
    Ash,
    Ballad,
    Breeze,
    Cedar,
    Coral,
    Cove,
    Echo,
    Ember,
    Juniper,
    Maple,
    Marin,
    Sage,
    Shimmer,
    Sol,
    Spruce,
    Vale,
    Verse,
}

impl RealtimeVoice {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Alloy => "alloy",
            Self::Arbor => "arbor",
            Self::Ash => "ash",
            Self::Ballad => "ballad",
            Self::Breeze => "breeze",
            Self::Cedar => "cedar",
            Self::Coral => "coral",
            Self::Cove => "cove",
            Self::Echo => "echo",
            Self::Ember => "ember",
            Self::Juniper => "juniper",
            Self::Maple => "maple",
            Self::Marin => "marin",
            Self::Sage => "sage",
            Self::Shimmer => "shimmer",
            Self::Sol => "sol",
            Self::Spruce => "spruce",
            Self::Vale => "vale",
            Self::Verse => "verse",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RealtimeVoicesList {
    pub v1: Vec<RealtimeVoice>,
    pub v2: Vec<RealtimeVoice>,
    pub default_v1: RealtimeVoice,
    pub default_v2: RealtimeVoice,
}

impl RealtimeVoicesList {
    pub fn builtin() -> Self {
        Self {
            v1: vec![
                RealtimeVoice::Juniper,
                RealtimeVoice::Maple,
                RealtimeVoice::Spruce,
                RealtimeVoice::Ember,
                RealtimeVoice::Vale,
                RealtimeVoice::Breeze,
                RealtimeVoice::Arbor,
                RealtimeVoice::Sol,
                RealtimeVoice::Cove,
            ],
            v2: vec![
                RealtimeVoice::Alloy,
                RealtimeVoice::Ash,
                RealtimeVoice::Ballad,
                RealtimeVoice::Coral,
                RealtimeVoice::Echo,
                RealtimeVoice::Sage,
                RealtimeVoice::Shimmer,
                RealtimeVoice::Verse,
                RealtimeVoice::Marin,
                RealtimeVoice::Cedar,
            ],
            default_v1: RealtimeVoice::Cove,
            default_v2: RealtimeVoice::Marin,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeAudioFrame {
    pub data: String,
    pub sample_rate: u32,
    pub num_channels: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samples_per_channel: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeTranscriptDelta {
    pub delta: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeTranscriptDone {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeTranscriptEntry {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeHandoffRequested {
    pub handoff_id: String,
    pub item_id: String,
    pub input_transcript: String,
    pub active_transcript: Vec<RealtimeTranscriptEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeNoopRequested {
    pub call_id: String,
    pub item_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeInputAudioSpeechStarted {
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeResponseCancelled {
    pub response_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeResponseCreated {
    pub response_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct RealtimeResponseDone {
    pub response_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub enum RealtimeEvent {
    SessionUpdated {
        realtime_session_id: String,
        instructions: Option<String>,
    },
    InputAudioSpeechStarted(RealtimeInputAudioSpeechStarted),
    InputTranscriptDelta(RealtimeTranscriptDelta),
    InputTranscriptDone(RealtimeTranscriptDone),
    OutputTranscriptDelta(RealtimeTranscriptDelta),
    OutputTranscriptDone(RealtimeTranscriptDone),
    AudioOut(RealtimeAudioFrame),
    ResponseCreated(RealtimeResponseCreated),
    ResponseCancelled(RealtimeResponseCancelled),
    ResponseDone(RealtimeResponseDone),
    ConversationItemAdded(Value),
    ConversationItemDone {
        item_id: String,
    },
    HandoffRequested(RealtimeHandoffRequested),
    NoopRequested(RealtimeNoopRequested),
    Error(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct ConversationAudioParams {
    pub frame: RealtimeAudioFrame,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct ConversationTextParams {
    pub text: String,
}

/// Submission operation
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
pub enum Op {
    /// Abort current task without terminating background terminal processes.
    /// This server sends [`EventMsg::TurnAborted`] in response.
    Interrupt,

    /// Terminate all running background terminal processes for this thread.
    /// Use this when callers intentionally want to stop long-lived background shells.
    CleanBackgroundTerminals,

    /// Start a realtime conversation stream.
    RealtimeConversationStart(ConversationStartParams),

    /// Send audio input to the running realtime conversation stream.
    RealtimeConversationAudio(ConversationAudioParams),

    /// Send text input to the running realtime conversation stream.
    RealtimeConversationText(ConversationTextParams),

    /// Close the running realtime conversation stream.
    RealtimeConversationClose,

    /// Request the list of voices supported by realtime conversation streams.
    RealtimeConversationListVoices,

    /// Legacy user input.
    ///
    /// Prefer [`Op::UserTurn`] so the caller provides full turn context
    /// (cwd/approval/sandbox/model/etc.) for each turn.
    UserInput {
        /// User input items, see `InputItem`
        items: Vec<UserInput>,
        /// Optional turn-scoped environments.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        environments: Option<Vec<TurnEnvironmentSelection>>,
        /// Optional JSON Schema used to constrain the final assistant message for this turn.
        #[serde(skip_serializing_if = "Option::is_none")]
        final_output_json_schema: Option<Value>,
        /// Optional turn-scoped Responses API `client_metadata`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    },

    /// Similar to [`Op::UserInput`], but first applies persistent turn-context
    /// overrides in the same queued operation. This preserves submission order
    /// and prevents the input from starting if the overrides are rejected.
    UserInputWithTurnContext {
        /// User input items, see `InputItem`
        items: Vec<UserInput>,
        /// Optional turn-scoped environment selections.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        environments: Option<Vec<TurnEnvironmentSelection>>,
        /// Optional JSON Schema used to constrain the final assistant message for this turn.
        #[serde(skip_serializing_if = "Option::is_none")]
        final_output_json_schema: Option<Value>,
        /// Optional turn-scoped Responses API `client_metadata`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        responsesapi_client_metadata: Option<HashMap<String, String>>,

        /// Updated `cwd` for sandbox/tool calls.
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,

        /// Updated runtime workspace roots used to materialize symbolic
        /// `:workspace_roots` filesystem permissions.
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_roots: Option<Vec<AbsolutePathBuf>>,

        /// Updated profile-defined workspace roots for status summaries and
        /// per-turn config reconstruction.
        #[serde(skip_serializing_if = "Option::is_none")]
        profile_workspace_roots: Option<Vec<AbsolutePathBuf>>,

        /// Updated command approval policy.
        #[serde(skip_serializing_if = "Option::is_none")]
        approval_policy: Option<AskForApproval>,

        /// Updated approval reviewer for future approval prompts.
        #[serde(skip_serializing_if = "Option::is_none")]
        approvals_reviewer: Option<ApprovalsReviewer>,

        /// Updated sandbox policy for tool calls.
        #[serde(skip_serializing_if = "Option::is_none")]
        sandbox_policy: Option<SandboxPolicy>,

        /// Updated permissions profile for tool calls.
        #[serde(skip_serializing_if = "Option::is_none")]
        permission_profile: Option<PermissionProfile>,

        /// Named or built-in profile that produced `permission_profile`, if
        /// the update selected a profile rather than supplying raw
        /// permissions.
        #[serde(skip_serializing_if = "Option::is_none")]
        active_permission_profile: Option<ActivePermissionProfile>,

        /// Updated Windows sandbox mode for tool execution.
        #[serde(skip_serializing_if = "Option::is_none")]
        windows_sandbox_level: Option<WindowsSandboxLevel>,

        /// Updated model slug. When set, the model info is derived
        /// automatically.
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,

        /// Updated model provider for future turns.
        #[serde(skip_serializing_if = "Option::is_none")]
        model_provider: Option<String>,

        /// Updated reasoning effort (honored only for reasoning-capable models).
        ///
        /// Use `Some(Some(_))` to set a specific effort, `Some(None)` to clear
        /// the effort, or `None` to leave the existing value unchanged.
        #[serde(skip_serializing_if = "Option::is_none")]
        effort: Option<Option<ReasoningEffortConfig>>,

        /// Updated reasoning summary preference (honored only for reasoning-capable models).
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<ReasoningSummaryConfig>,

        /// Updated service tier preference for future turns.
        ///
        /// Use `Some(Some(_))` to set a specific tier, `Some(None)` to clear the
        /// preference, or `None` to leave the existing value unchanged.
        #[serde(skip_serializing_if = "Option::is_none")]
        service_tier: Option<Option<String>>,

        /// EXPERIMENTAL - set a pre-set collaboration mode.
        /// Takes precedence over model, effort, and developer instructions if set.
        #[serde(skip_serializing_if = "Option::is_none")]
        collaboration_mode: Option<CollaborationMode>,

        /// Updated personality preference.
        #[serde(skip_serializing_if = "Option::is_none")]
        personality: Option<Personality>,
    },

    /// Similar to [`Op::UserInput`], but contains additional context required
    /// for a turn of a [`crate::codex_thread::CodexThread`].
    UserTurn {
        /// User input items, see `InputItem`
        items: Vec<UserInput>,

        /// `cwd` to use with the [`SandboxPolicy`] and potentially tool calls
        /// such as `local_shell`.
        cwd: PathBuf,

        /// Policy to use for command approval.
        approval_policy: AskForApproval,

        /// Reviewer to use for approval requests raised during this turn.
        ///
        /// When omitted, the session keeps the current setting
        approvals_reviewer: Option<ApprovalsReviewer>,

        /// Policy to use for tool calls such as `local_shell`.
        sandbox_policy: SandboxPolicy,

        /// Full permissions profile to use for tool calls such as `local_shell`.
        ///
        /// When omitted, `sandbox_policy` is used as a legacy compatibility
        /// projection.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_profile: Option<PermissionProfile>,

        /// Must be a valid model slug for the configured client session
        /// associated with this conversation.
        model: String,

        /// Will only be honored if the model is configured to use reasoning.
        #[serde(skip_serializing_if = "Option::is_none")]
        effort: Option<ReasoningEffortConfig>,

        /// Will only be honored if the model is configured to use reasoning.
        ///
        /// When omitted, the session keeps the current setting (which allows core to
        /// fall back to the selected model's default on new sessions).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<ReasoningSummaryConfig>,

        /// Optional service tier override for this turn.
        ///
        /// Use `Some(Some(_))` to set a specific tier for this turn, `Some(None)` to
        /// explicitly clear the tier for this turn, or `None` to keep the existing
        /// session preference.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        service_tier: Option<Option<String>>,

        // The JSON schema to use for the final assistant message
        final_output_json_schema: Option<Value>,

        /// EXPERIMENTAL - set a pre-set collaboration mode.
        /// Takes precedence over model, effort, and developer instructions if set.
        #[serde(skip_serializing_if = "Option::is_none")]
        collaboration_mode: Option<CollaborationMode>,

        /// Optional personality override for this turn.
        #[serde(skip_serializing_if = "Option::is_none")]
        personality: Option<Personality>,

        /// Optional turn-scoped environments.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        environments: Option<Vec<TurnEnvironmentSelection>>,
    },

    /// Inter-agent communication that should be recorded as assistant history
    /// while still using the normal thread submission lifecycle.
    InterAgentCommunication {
        communication: InterAgentCommunication,
    },

    /// Override parts of the persistent turn context for subsequent turns.
    ///
    /// All fields are optional; when omitted, the existing value is preserved.
    /// This does not enqueue any input – it only updates defaults used for
    /// turns that rely on persistent session-level context (for example,
    /// [`Op::UserInput`]).
    OverrideTurnContext {
        /// Updated `cwd` for sandbox/tool calls.
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,

        /// Updated command approval policy.
        #[serde(skip_serializing_if = "Option::is_none")]
        approval_policy: Option<AskForApproval>,

        /// Updated approval reviewer for future approval prompts.
        #[serde(skip_serializing_if = "Option::is_none")]
        approvals_reviewer: Option<ApprovalsReviewer>,

        /// Updated sandbox policy for tool calls.
        #[serde(skip_serializing_if = "Option::is_none")]
        sandbox_policy: Option<SandboxPolicy>,

        /// Updated permissions profile for tool calls.
        #[serde(skip_serializing_if = "Option::is_none")]
        permission_profile: Option<PermissionProfile>,

        /// Updated Windows sandbox mode for tool execution.
        #[serde(skip_serializing_if = "Option::is_none")]
        windows_sandbox_level: Option<WindowsSandboxLevel>,

        /// Updated model slug. When set, the model info is derived
        /// automatically.
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,

        /// Updated reasoning effort (honored only for reasoning-capable models).
        ///
        /// Use `Some(Some(_))` to set a specific effort, `Some(None)` to clear
        /// the effort, or `None` to leave the existing value unchanged.
        #[serde(skip_serializing_if = "Option::is_none")]
        effort: Option<Option<ReasoningEffortConfig>>,

        /// Updated reasoning summary preference (honored only for reasoning-capable models).
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<ReasoningSummaryConfig>,

        /// Updated service tier preference for future turns.
        ///
        /// Use `Some(Some(_))` to set a specific tier, `Some(None)` to clear the
        /// preference, or `None` to leave the existing value unchanged.
        #[serde(skip_serializing_if = "Option::is_none")]
        service_tier: Option<Option<String>>,

        /// EXPERIMENTAL - set a pre-set collaboration mode.
        /// Takes precedence over model, effort, and developer instructions if set.
        #[serde(skip_serializing_if = "Option::is_none")]
        collaboration_mode: Option<CollaborationMode>,

        /// Updated personality preference.
        #[serde(skip_serializing_if = "Option::is_none")]
        personality: Option<Personality>,
    },

    /// Approve a command execution
    ExecApproval {
        /// The id of the submission we are approving
        id: String,
        /// Turn id associated with the approval event, when available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        /// The user's decision in response to the request.
        decision: ReviewDecision,
    },

    /// Approve a code patch
    PatchApproval {
        /// The id of the submission we are approving
        id: String,
        /// The user's decision in response to the request.
        decision: ReviewDecision,
    },

    /// Resolve an MCP elicitation request.
    ResolveElicitation {
        /// Name of the MCP server that issued the request.
        server_name: String,
        /// Request identifier from the MCP server.
        request_id: RequestId,
        /// User's decision for the request.
        decision: ElicitationAction,
        /// Structured user input supplied for accepted elicitations.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<Value>,
        /// Optional client metadata associated with the elicitation response.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
    },

    /// Resolve a request_user_input tool call.
    #[serde(rename = "user_input_answer", alias = "request_user_input_response")]
    UserInputAnswer {
        /// Turn id for the in-flight request.
        id: String,
        /// User-provided answers.
        response: RequestUserInputResponse,
    },

    /// Resolve a request_permissions tool call.
    RequestPermissionsResponse {
        /// Call id for the in-flight request.
        id: String,
        /// User-granted permissions.
        response: RequestPermissionsResponse,
    },

    /// Resolve a dynamic tool call request.
    DynamicToolResponse {
        /// Call id for the in-flight request.
        id: String,
        /// Tool output payload.
        response: DynamicToolResponse,
    },

    /// Request MCP servers to reinitialize and refresh cached tool lists.
    RefreshMcpServers { config: McpServerRefreshConfig },

    /// Reload user config layer overrides for the active session.
    ///
    /// This updates runtime config-derived behavior (for example app
    /// enable/disable state) without restarting the thread.
    ReloadUserConfig,

    /// Request the agent to summarize the current conversation context.
    /// The agent will use its existing context (either conversation history or previous response id)
    /// to generate a summary which will be returned as an AgentMessage event.
    Compact,

    /// Set whether the thread remains eligible for memory generation.
    ///
    /// This persists thread-level memory mode metadata without involving the
    /// model.
    SetThreadMemoryMode { mode: ThreadMemoryMode },

    /// Request Codex to drop the last N user turns from in-memory context.
    ///
    /// This does not attempt to revert local filesystem changes. Clients are
    /// responsible for undoing any edits on disk.
    ThreadRollback { num_turns: u32 },

    /// Request a code review from the agent.
    Review { review_request: ReviewRequest },

    /// Record that the user approved one retry of a concrete Guardian-denied action.
    ApproveGuardianDeniedAction { event: GuardianAssessmentEvent },

    /// Request to shut down codex instance.
    Shutdown,

    /// Execute a user-initiated one-off shell command (triggered by "!cmd").
    ///
    /// The command string is executed using the user's default shell and may
    /// include shell syntax (pipes, redirects, etc.). Output is streamed via
    /// `ExecCommand*` events and the UI regains control upon `TurnComplete`.
    RunUserShellCommand {
        /// The raw command string after '!'
        command: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThreadMemoryMode {
    Enabled,
    Disabled,
}

impl From<Vec<UserInput>> for Op {
    fn from(value: Vec<UserInput>) -> Self {
        Op::UserInput {
            environments: None,
            items: value,
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum InterAgentOperation {
    #[default]
    Unknown,
    SpawnAgent,
    SendMessage,
    FollowupTask,
    ChildCompletion,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InterAgentContentPart {
    Text {
        text: String,
    },
    ImageRef {
        attachment_id: String,
        #[serde(default)]
        #[ts(optional = false, type = "string | null")]
        image_url: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, TS)]
pub struct InterAgentCommunication {
    pub author: AgentPath,
    pub recipient: AgentPath,
    #[serde(default)]
    pub other_recipients: Vec<AgentPath>,
    pub content: String,
    #[serde(default)]
    pub content_parts: Vec<InterAgentContentPart>,
    #[serde(default)]
    pub operation: InterAgentOperation,
    #[serde(default = "default_true")]
    pub trigger_turn: bool,
    #[serde(default)]
    #[ts(optional = false, type = "string | null")]
    pub sender_thread_id: Option<ThreadId>,
    #[serde(default)]
    #[ts(optional = false, type = "string | null")]
    pub recipient_thread_id: Option<ThreadId>,
    #[serde(default)]
    #[ts(optional = false, type = "unknown | null")]
    pub status: Option<AgentStatus>,
    #[serde(default)]
    #[ts(optional = false, type = "unknown | null")]
    pub lifecycle_status: Option<ThreadLifecycleStatus>,
    #[serde(default)]
    #[ts(optional = false, type = "string | null")]
    pub agent_nickname: Option<String>,
    #[serde(default)]
    #[ts(optional = false, type = "string | null")]
    pub agent_role: Option<String>,
}

impl InterAgentCommunication {
    pub fn new(
        author: AgentPath,
        recipient: AgentPath,
        other_recipients: Vec<AgentPath>,
        content: String,
        operation: InterAgentOperation,
    ) -> Self {
        Self {
            author,
            recipient,
            other_recipients,
            content,
            content_parts: Vec::new(),
            operation,
            trigger_turn: true,
            sender_thread_id: None,
            recipient_thread_id: None,
            status: None,
            lifecycle_status: None,
            agent_nickname: None,
            agent_role: None,
        }
    }

    pub fn with_content_parts(mut self, content_parts: Vec<InterAgentContentPart>) -> Self {
        self.content_parts = content_parts;
        self
    }

    pub fn with_trigger_turn(mut self, trigger_turn: bool) -> Self {
        self.trigger_turn = trigger_turn;
        self
    }

    pub fn with_thread_ids(
        mut self,
        sender_thread_id: ThreadId,
        recipient_thread_id: ThreadId,
    ) -> Self {
        self.sender_thread_id = Some(sender_thread_id);
        self.recipient_thread_id = Some(recipient_thread_id);
        self
    }

    pub fn with_status(mut self, status: AgentStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_lifecycle_status(mut self, lifecycle_status: ThreadLifecycleStatus) -> Self {
        self.lifecycle_status = Some(lifecycle_status);
        self
    }

    pub fn with_agent_metadata(
        mut self,
        agent_nickname: Option<String>,
        agent_role: Option<String>,
    ) -> Self {
        self.agent_nickname = agent_nickname;
        self.agent_role = agent_role;
        self
    }

    pub fn to_response_input_item(&self) -> ResponseInputItem {
        if !self.content_parts.is_empty() {
            let mut content = Vec::new();
            for part in &self.content_parts {
                match part {
                    InterAgentContentPart::Text { text } => {
                        content.push(ContentItem::InputText { text: text.clone() });
                    }
                    InterAgentContentPart::ImageRef {
                        attachment_id,
                        image_url: Some(image_url),
                    } => {
                        content.push(ContentItem::InputText {
                            text: format!("Image attachment_id={attachment_id} begins"),
                        });
                        content.push(ContentItem::InputText {
                            text: crate::models::image_open_tag_text_with_attachment_id(
                                attachment_id,
                            ),
                        });
                        content.push(ContentItem::InputImage {
                            image_url: image_url.clone(),
                            detail: Some(crate::models::DEFAULT_IMAGE_DETAIL),
                        });
                        content.push(ContentItem::InputText {
                            text: crate::models::image_close_tag_text(),
                        });
                        content.push(ContentItem::InputText {
                            text: format!("Image attachment_id={attachment_id} ends"),
                        });
                    }
                    InterAgentContentPart::ImageRef { .. } => {}
                }
            }
            return ResponseInputItem::Message {
                role: "user".to_string(),
                content,
                phase: None,
            };
        }
        ResponseInputItem::Message {
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: serde_json::to_string(self).unwrap_or_default(),
            }],
            phase: Some(MessagePhase::Commentary),
        }
    }
}

impl Op {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Interrupt => "interrupt",
            Self::CleanBackgroundTerminals => "clean_background_terminals",
            Self::RealtimeConversationStart(_) => "realtime_conversation_start",
            Self::RealtimeConversationAudio(_) => "realtime_conversation_audio",
            Self::RealtimeConversationText(_) => "realtime_conversation_text",
            Self::RealtimeConversationClose => "realtime_conversation_close",
            Self::RealtimeConversationListVoices => "realtime_conversation_list_voices",
            Self::UserInput { .. } => "user_input",
            Self::UserInputWithTurnContext { .. } => "user_input_with_turn_context",
            Self::UserTurn { .. } => "user_turn",
            Self::InterAgentCommunication { .. } => "inter_agent_communication",
            Self::OverrideTurnContext { .. } => "override_turn_context",
            Self::ExecApproval { .. } => "exec_approval",
            Self::PatchApproval { .. } => "patch_approval",
            Self::ResolveElicitation { .. } => "resolve_elicitation",
            Self::UserInputAnswer { .. } => "user_input_answer",
            Self::RequestPermissionsResponse { .. } => "request_permissions_response",
            Self::DynamicToolResponse { .. } => "dynamic_tool_response",
            Self::RefreshMcpServers { .. } => "refresh_mcp_servers",
            Self::ReloadUserConfig => "reload_user_config",
            Self::Compact => "compact",
            Self::SetThreadMemoryMode { .. } => "set_thread_memory_mode",
            Self::ThreadRollback { .. } => "thread_rollback",
            Self::Review { .. } => "review",
            Self::ApproveGuardianDeniedAction { .. } => "approve_guardian_denied_action",
            Self::Shutdown => "shutdown",
            Self::RunUserShellCommand { .. } => "run_user_shell_command",
        }
    }
}

/// Determines the conditions under which the user is consulted to approve
/// running the command proposed by Codex.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    JsonSchema,
    TS,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum AskForApproval {
    /// Under this policy, only "known safe" commands—as determined by
    /// `is_safe_command()`—that **only read files** are auto‑approved.
    /// Everything else will ask the user to approve.
    #[serde(rename = "untrusted")]
    #[strum(serialize = "untrusted")]
    UnlessTrusted,

    /// DEPRECATED: *All* commands are auto‑approved, but they are expected to
    /// run inside a sandbox where network access is disabled and writes are
    /// confined to a specific set of paths. If the command fails, it will be
    /// escalated to the user to approve execution without a sandbox.
    /// Prefer `OnRequest` for interactive runs or `Never` for non-interactive
    /// runs.
    OnFailure,

    /// The model decides when to ask the user for approval.
    #[default]
    OnRequest,

    /// Fine-grained controls for individual approval flows.
    ///
    /// When a field is `true`, commands in that category are allowed. When it
    /// is `false`, those requests are automatically rejected instead of shown
    /// to the user.
    #[strum(serialize = "granular")]
    Granular(GranularApprovalConfig),

    /// Never ask the user to approve commands. Failures are immediately returned
    /// to the model, and never escalated to the user for approval.
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub struct GranularApprovalConfig {
    /// Whether to allow shell command approval requests, including inline
    /// `with_additional_permissions` and `require_escalated` requests.
    pub sandbox_approval: bool,
    /// Whether to allow prompts triggered by execpolicy `prompt` rules.
    pub rules: bool,
    /// Whether to allow approval prompts triggered by skill script execution.
    #[serde(default)]
    pub skill_approval: bool,
    /// Whether to allow prompts triggered by the `request_permissions` tool.
    #[serde(default)]
    pub request_permissions: bool,
    /// Whether to allow MCP elicitation prompts.
    pub mcp_elicitations: bool,
}

impl GranularApprovalConfig {
    pub const fn allows_sandbox_approval(self) -> bool {
        self.sandbox_approval
    }

    pub const fn allows_rules_approval(self) -> bool {
        self.rules
    }

    pub const fn allows_skill_approval(self) -> bool {
        self.skill_approval
    }

    pub const fn allows_request_permissions(self) -> bool {
        self.request_permissions
    }

    pub const fn allows_mcp_elicitations(self) -> bool {
        self.mcp_elicitations
    }
}

/// Represents whether outbound network access is available to the agent.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, Default, JsonSchema, TS,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum NetworkAccess {
    #[default]
    Restricted,
    Enabled,
}

impl NetworkAccess {
    pub fn is_enabled(self) -> bool {
        matches!(self, NetworkAccess::Enabled)
    }
}

/// Determines execution restrictions for model shell commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Display, JsonSchema, TS)]
#[strum(serialize_all = "kebab-case")]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SandboxPolicy {
    /// No restrictions whatsoever. Use with caution.
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,

    /// Read-only access configuration.
    #[serde(rename = "read-only")]
    ReadOnly {
        /// When set to `true`, outbound network access is allowed. `false` by
        /// default.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        network_access: bool,
    },

    /// Indicates the process is already in an external sandbox. Allows full
    /// disk access while honoring the provided network setting.
    #[serde(rename = "external-sandbox")]
    ExternalSandbox {
        /// Whether the external sandbox permits outbound network traffic.
        #[serde(default)]
        network_access: NetworkAccess,
    },

    /// Same as `ReadOnly` but additionally grants write access to the current
    /// working directory ("workspace").
    #[serde(rename = "workspace-write")]
    WorkspaceWrite {
        /// Additional folders (beyond cwd and possibly TMPDIR) that should be
        /// writable from within the sandbox.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        writable_roots: Vec<AbsolutePathBuf>,

        /// When set to `true`, outbound network access is allowed. `false` by
        /// default.
        #[serde(default)]
        network_access: bool,

        /// When set to `true`, will NOT include the per-user `TMPDIR`
        /// environment variable among the default writable roots. Defaults to
        /// `false`.
        #[serde(default)]
        exclude_tmpdir_env_var: bool,

        /// When set to `true`, will NOT include the `/tmp` among the default
        /// writable roots on UNIX. Defaults to `false`.
        #[serde(default)]
        exclude_slash_tmp: bool,
    },
}

/// A writable root path accompanied by a list of subpaths that should remain
/// read‑only even when the root is writable. This is primarily used to ensure
/// that folders containing files that could be modified to escalate the
/// privileges of the agent (e.g. `.codex`, `.git`, notably `.git/hooks`) under
/// a writable root are not modified by the agent.
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
pub struct WritableRoot {
    pub root: AbsolutePathBuf,

    /// By construction, these subpaths are all under `root`.
    pub read_only_subpaths: Vec<AbsolutePathBuf>,

    /// Workspace metadata path names that must not be created or replaced under
    /// `root` unless the policy grants an explicit write rule for that metadata
    /// path.
    pub protected_metadata_names: Vec<String>,
}

impl WritableRoot {
    pub fn is_path_writable(&self, path: &Path) -> bool {
        // Check if the path is under the root.
        if !path.starts_with(&self.root) {
            return false;
        }

        // Check if the path is under any of the read-only subpaths.
        for subpath in &self.read_only_subpaths {
            if path.starts_with(subpath) {
                return false;
            }
        }

        if self.path_contains_protected_metadata_name(path) {
            return false;
        }

        true
    }

    fn path_contains_protected_metadata_name(&self, path: &Path) -> bool {
        let Ok(relative_path) = path.strip_prefix(&self.root) else {
            return false;
        };

        let Some(first_component) = relative_path.components().next() else {
            return false;
        };

        self.protected_metadata_names
            .iter()
            .any(|name| first_component.as_os_str() == std::ffi::OsStr::new(name))
    }
}

impl FromStr for SandboxPolicy {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

impl FromStr for FileSystemSandboxPolicy {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

impl FromStr for NetworkSandboxPolicy {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

impl SandboxPolicy {
    /// Returns a policy with read-only disk access and no network.
    pub fn new_read_only_policy() -> Self {
        SandboxPolicy::ReadOnly {
            network_access: false,
        }
    }

    /// Returns a policy that can read the entire disk, but can only write to
    /// the current working directory and the per-user tmp dir on macOS. It does
    /// not allow network access.
    pub fn new_workspace_write_policy() -> Self {
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        }
    }

    pub fn has_full_disk_read_access(&self) -> bool {
        true
    }

    pub fn has_full_disk_write_access(&self) -> bool {
        match self {
            SandboxPolicy::DangerFullAccess => true,
            SandboxPolicy::ExternalSandbox { .. } => true,
            SandboxPolicy::ReadOnly { .. } => false,
            SandboxPolicy::WorkspaceWrite { .. } => false,
        }
    }

    pub fn has_full_network_access(&self) -> bool {
        match self {
            SandboxPolicy::DangerFullAccess => true,
            SandboxPolicy::ExternalSandbox { network_access } => network_access.is_enabled(),
            SandboxPolicy::ReadOnly { network_access, .. } => *network_access,
            SandboxPolicy::WorkspaceWrite { network_access, .. } => *network_access,
        }
    }

    /// Returns the list of writable roots (tailored to the current working
    /// directory) together with subpaths that should remain read‑only under
    /// each writable root.
    pub fn get_writable_roots_with_cwd(&self, cwd: &Path) -> Vec<WritableRoot> {
        match self {
            SandboxPolicy::DangerFullAccess => Vec::new(),
            SandboxPolicy::ExternalSandbox { .. } => Vec::new(),
            SandboxPolicy::ReadOnly { .. } => Vec::new(),
            SandboxPolicy::WorkspaceWrite {
                writable_roots,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
                network_access: _,
            } => {
                // Start from explicitly configured writable roots.
                let mut roots: Vec<AbsolutePathBuf> = writable_roots.clone();

                // Always include defaults: cwd, /tmp (if present on Unix), and
                // on macOS, the per-user TMPDIR unless explicitly excluded.
                // TODO(mbolin): cwd param should be AbsolutePathBuf.
                let cwd_absolute = AbsolutePathBuf::from_absolute_path(cwd);
                match cwd_absolute {
                    Ok(cwd) => {
                        roots.push(cwd);
                    }
                    Err(e) => {
                        error!(
                            "Ignoring invalid cwd {:?} for sandbox writable root: {}",
                            cwd, e
                        );
                    }
                }

                // Include /tmp on Unix unless explicitly excluded.
                if cfg!(unix) && !exclude_slash_tmp {
                    match AbsolutePathBuf::from_absolute_path("/tmp") {
                        Ok(slash_tmp) => {
                            if slash_tmp.as_path().is_dir() {
                                roots.push(slash_tmp);
                            }
                        }
                        Err(e) => {
                            error!("Ignoring invalid /tmp for sandbox writable root: {e}");
                        }
                    }
                }

                // Include $TMPDIR unless explicitly excluded. On macOS, TMPDIR
                // is per-user, so writes to TMPDIR should not be readable by
                // other users on the system.
                //
                // By comparison, TMPDIR is not guaranteed to be defined on
                // Linux or Windows, but supporting it here gives users a way to
                // provide the model with their own temporary directory without
                // having to hardcode it in the config.
                if !exclude_tmpdir_env_var
                    && let Some(tmpdir) = std::env::var_os("TMPDIR")
                    && !tmpdir.is_empty()
                {
                    match AbsolutePathBuf::from_absolute_path(PathBuf::from(&tmpdir)) {
                        Ok(tmpdir_path) => {
                            roots.push(tmpdir_path);
                        }
                        Err(e) => {
                            error!(
                                "Ignoring invalid TMPDIR value {tmpdir:?} for sandbox writable root: {e}",
                            );
                        }
                    }
                }

                // For each root, compute subpaths that should remain read-only.
                let cwd_root = AbsolutePathBuf::from_absolute_path(cwd).ok();
                roots
                    .into_iter()
                    .map(|writable_root| {
                        let protect_missing_dot_codex = cwd_root
                            .as_ref()
                            .is_some_and(|cwd_root| cwd_root == &writable_root);
                        WritableRoot {
                            read_only_subpaths: default_read_only_subpaths_for_writable_root(
                                &writable_root,
                                protect_missing_dot_codex,
                            ),
                            protected_metadata_names: Vec::new(),
                            root: writable_root,
                        }
                    })
                    .collect()
            }
        }
    }
}
