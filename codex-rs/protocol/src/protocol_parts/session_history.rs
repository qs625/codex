#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct AgentMessageEvent {
    pub message: String,
    #[serde(default)]
    pub phase: Option<MessagePhase>,
    #[serde(default)]
    pub memory_citation: Option<MemoryCitation>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct UserMessageSkill {
    pub name: String,
    pub path: std::path::PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct UserMessageEvent {
    pub message: String,
    /// Image URLs sourced from `UserInput::Image`. These are safe
    /// to replay in legacy UI history events and correspond to images sent to
    /// the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    /// Local file paths sourced from `UserInput::LocalImage`. These are kept so
    /// the UI can reattach images when editing history, and should not be sent
    /// to the model or treated as API-ready URLs.
    #[serde(default)]
    pub local_images: Vec<std::path::PathBuf>,
    /// Skill selections sourced from `UserInput::Skill`. These preserve explicit
    /// loaded-skill attachments in legacy UI history events.
    #[serde(default)]
    pub skills: Vec<UserMessageSkill>,
    /// UI-defined spans within `message` used to render or persist special elements.
    #[serde(default)]
    pub text_elements: Vec<crate::user_input::TextElement>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct AgentReasoningEvent {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct AgentReasoningRawContentEvent {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct AgentReasoningSectionBreakEvent {
    // load with default value so it's backward compatible with the old format.
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub summary_index: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS, PartialEq)]
pub struct McpInvocation {
    /// Name of the MCP server as defined in the config.
    pub server: String,
    /// Name of the tool as given by the MCP server.
    pub tool: String,
    /// Arguments to the tool call.
    pub arguments: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS, PartialEq)]
pub struct McpToolCallBeginEvent {
    /// Identifier so this can be paired with the McpToolCallEnd event.
    pub call_id: String,
    pub invocation: McpInvocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mcp_app_resource_uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS, PartialEq)]
pub struct McpToolCallEndEvent {
    /// Identifier for the corresponding McpToolCallBegin that finished.
    pub call_id: String,
    pub invocation: McpInvocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mcp_app_resource_uri: Option<String>,
    #[ts(type = "string")]
    pub duration: Duration,
    /// Result of the tool call. Note this could be an error.
    pub result: Result<CallToolResult, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS, PartialEq)]
pub struct DynamicToolCallResponseEvent {
    /// Identifier for the corresponding DynamicToolCallRequest.
    pub call_id: String,
    /// Turn ID that this dynamic tool call belongs to.
    pub turn_id: String,
    #[serde(default)]
    pub completed_at_ms: i64,
    /// Dynamic tool namespace, when one was provided.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Dynamic tool name.
    pub tool: String,
    /// Dynamic tool call arguments.
    pub arguments: serde_json::Value,
    /// Dynamic tool response content items.
    pub content_items: Vec<DynamicToolCallOutputContentItem>,
    /// Whether the tool call succeeded.
    pub success: bool,
    /// Optional error text when the tool call failed before producing a response.
    pub error: Option<String>,
    /// The duration of the dynamic tool call.
    #[ts(type = "string")]
    pub duration: Duration,
}

impl McpToolCallEndEvent {
    pub fn is_success(&self) -> bool {
        match &self.result {
            Ok(result) => !result.is_error.unwrap_or(false),
            Err(_) => false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct WebSearchBeginEvent {
    pub call_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct WebSearchEndEvent {
    pub call_id: String,
    pub query: String,
    pub action: WebSearchAction,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ImageGenerationBeginEvent {
    pub call_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ImageGenerationEndEvent {
    pub call_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revised_prompt: Option<String>,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub saved_path: Option<AbsolutePathBuf>,
}

// Conversation kept for backward compatibility.
/// Response payload for `Op::GetHistory` containing the current session's
/// in-memory transcript.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ConversationPathResponseEvent {
    pub conversation_id: ThreadId,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ResumedHistory {
    pub conversation_id: ThreadId,
    pub history: Vec<RolloutItem>,
    pub rollout_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub enum InitialHistory {
    New,
    Cleared,
    Resumed(ResumedHistory),
    Forked(Vec<RolloutItem>),
}

impl InitialHistory {
    pub fn scan_rollout_items(&self, mut predicate: impl FnMut(&RolloutItem) -> bool) -> bool {
        match self {
            InitialHistory::New | InitialHistory::Cleared => false,
            InitialHistory::Resumed(resumed) => resumed.history.iter().any(&mut predicate),
            InitialHistory::Forked(items) => items.iter().any(predicate),
        }
    }

    pub fn forked_from_id(&self) -> Option<ThreadId> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => {
                resumed.history.iter().find_map(|item| match item {
                    RolloutItem::SessionMeta(meta_line) => meta_line.meta.forked_from_id,
                    _ => None,
                })
            }
            InitialHistory::Forked(items) => items.iter().find_map(|item| match item {
                RolloutItem::SessionMeta(meta_line) => Some(meta_line.meta.id),
                _ => None,
            }),
        }
    }

    pub fn session_cwd(&self) -> Option<PathBuf> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => session_cwd_from_items(&resumed.history),
            InitialHistory::Forked(items) => session_cwd_from_items(items),
        }
    }

    pub fn get_rollout_items(&self) -> Vec<RolloutItem> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => Vec::new(),
            InitialHistory::Resumed(resumed) => resumed.history.clone(),
            InitialHistory::Forked(items) => items.clone(),
        }
    }

    pub fn get_event_msgs(&self) -> Option<Vec<EventMsg>> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => Some(
                resumed
                    .history
                    .iter()
                    .filter_map(|ri| match ri {
                        RolloutItem::EventMsg(ev) => Some(ev.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
            InitialHistory::Forked(items) => Some(
                items
                    .iter()
                    .filter_map(|ri| match ri {
                        RolloutItem::EventMsg(ev) => Some(ev.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
        }
    }

    pub fn get_base_instructions(&self) -> Option<BaseInstructions> {
        // TODO: SessionMeta should (in theory) always be first in the history, so we can probably only check the first item?
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => {
                resumed.history.iter().find_map(|item| match item {
                    RolloutItem::SessionMeta(meta_line) => meta_line.meta.base_instructions.clone(),
                    _ => None,
                })
            }
            InitialHistory::Forked(items) => items.iter().find_map(|item| match item {
                RolloutItem::SessionMeta(meta_line) => meta_line.meta.base_instructions.clone(),
                _ => None,
            }),
        }
    }

    pub fn get_dynamic_tools(&self) -> Option<Vec<DynamicToolSpec>> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => {
                resumed.history.iter().find_map(|item| match item {
                    RolloutItem::SessionMeta(meta_line) => meta_line.meta.dynamic_tools.clone(),
                    _ => None,
                })
            }
            InitialHistory::Forked(items) => items.iter().find_map(|item| match item {
                RolloutItem::SessionMeta(meta_line) => meta_line.meta.dynamic_tools.clone(),
                _ => None,
            }),
        }
    }

    pub fn get_subscriptions(&self) -> Option<Vec<PersistedSubscription>> {
        match self {
            InitialHistory::New | InitialHistory::Cleared => None,
            InitialHistory::Resumed(resumed) => {
                resumed.history.iter().rev().find_map(|item| match item {
                    RolloutItem::SessionMeta(meta_line) => meta_line.meta.subscriptions.clone(),
                    _ => None,
                })
            }
            InitialHistory::Forked(items) => items.iter().rev().find_map(|item| match item {
                RolloutItem::SessionMeta(meta_line) => meta_line.meta.subscriptions.clone(),
                _ => None,
            }),
        }
    }

    pub fn get_resumed_thread_source(&self) -> Option<ThreadSource> {
        match self {
            InitialHistory::New | InitialHistory::Cleared | InitialHistory::Forked(_) => None,
            InitialHistory::Resumed(resumed) => {
                resumed.history.iter().find_map(|item| match item {
                    RolloutItem::SessionMeta(meta_line) => meta_line.meta.thread_source,
                    _ => None,
                })
            }
        }
    }
}

fn session_cwd_from_items(items: &[RolloutItem]) -> Option<PathBuf> {
    items.iter().find_map(|item| match item {
        RolloutItem::SessionMeta(meta_line) => Some(meta_line.meta.cwd.clone()),
        _ => None,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, TS, Default)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase")]
pub enum SessionSource {
    Cli,
    #[default]
    VSCode,
    Exec,
    Mcp,
    Custom(String),
    Internal(InternalSessionSource),
    SubAgent(SubAgentSource),
    #[serde(other)]
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ThreadSource {
    User,
    Subagent,
    MemoryConsolidation,
}

impl ThreadSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadSource::User => "user",
            ThreadSource::Subagent => "subagent",
            ThreadSource::MemoryConsolidation => "memory_consolidation",
        }
    }
}

impl fmt::Display for ThreadSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ThreadSource {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user" => Ok(ThreadSource::User),
            "subagent" => Ok(ThreadSource::Subagent),
            "memory_consolidation" => Ok(ThreadSource::MemoryConsolidation),
            other => Err(format!("unknown thread source: {other}")),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum InternalSessionSource {
    MemoryConsolidation,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum SubAgentSource {
    Review,
    Compact,
    ThreadSpawn {
        parent_thread_id: ThreadId,
        depth: i32,
        #[serde(default)]
        agent_path: Option<AgentPath>,
        #[serde(default)]
        agent_nickname: Option<String>,
        #[serde(default, alias = "agent_type")]
        agent_role: Option<String>,
    },
    MemoryConsolidation,
    Other(String),
}

impl fmt::Display for SessionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionSource::Cli => f.write_str("cli"),
            SessionSource::VSCode => f.write_str("vscode"),
            SessionSource::Exec => f.write_str("exec"),
            SessionSource::Mcp => f.write_str("mcp"),
            SessionSource::Custom(source) => f.write_str(source),
            SessionSource::Internal(source) => write!(f, "internal_{source}"),
            SessionSource::SubAgent(sub_source) => write!(f, "subagent_{sub_source}"),
            SessionSource::Unknown => f.write_str("unknown"),
        }
    }
}

impl SessionSource {
    pub fn from_startup_arg(value: &str) -> Result<Self, &'static str> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err("session source must not be empty");
        }

        let normalized = trimmed.to_ascii_lowercase();
        Ok(match normalized.as_str() {
            "cli" => SessionSource::Cli,
            "vscode" => SessionSource::VSCode,
            "exec" => SessionSource::Exec,
            "mcp" | "appserver" | "app-server" | "app_server" => SessionSource::Mcp,
            "unknown" => SessionSource::Unknown,
            _ => SessionSource::Custom(normalized),
        })
    }

    pub fn is_internal(&self) -> bool {
        matches!(self, SessionSource::Internal(_))
    }

    pub fn is_non_root_agent(&self) -> bool {
        matches!(
            self,
            SessionSource::Internal(_) | SessionSource::SubAgent(_)
        )
    }

    pub fn get_nickname(&self) -> Option<String> {
        match self {
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn { agent_nickname, .. }) => {
                agent_nickname.clone()
            }
            _ => None,
        }
    }

    pub fn get_agent_role(&self) -> Option<String> {
        match self {
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn { agent_role, .. }) => {
                agent_role.clone()
            }
            _ => None,
        }
    }

    pub fn get_agent_path(&self) -> Option<AgentPath> {
        match self {
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn { agent_path, .. }) => {
                agent_path.clone()
            }
            _ => None,
        }
    }

    pub fn restriction_product(&self) -> Option<Product> {
        match self {
            SessionSource::Custom(source) => Product::from_session_source_name(source),
            SessionSource::Cli
            | SessionSource::VSCode
            | SessionSource::Exec
            | SessionSource::Mcp
            | SessionSource::Unknown => Some(Product::Codex),
            SessionSource::Internal(_) | SessionSource::SubAgent(_) => None,
        }
    }

    pub fn matches_product_restriction(&self, products: &[Product]) -> bool {
        products.is_empty()
            || self
                .restriction_product()
                .is_some_and(|product| product.matches_product_restriction(products))
    }
}

impl fmt::Display for SubAgentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubAgentSource::Review => f.write_str("review"),
            SubAgentSource::Compact => f.write_str("compact"),
            SubAgentSource::MemoryConsolidation => f.write_str("memory_consolidation"),
            SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth,
                ..
            } => {
                write!(f, "thread_spawn_{parent_thread_id}_d{depth}")
            }
            SubAgentSource::Other(other) => f.write_str(other),
        }
    }
}

impl fmt::Display for InternalSessionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InternalSessionSource::MemoryConsolidation => f.write_str("memory_consolidation"),
        }
    }
}

/// SessionMeta contains session-level data that doesn't correspond to a specific turn.
///
/// NOTE: There used to be an `instructions` field here, which stored user_instructions, but we
/// now save that on TurnContext. base_instructions stores the base instructions for the session,
/// and should be used when there is no config override.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, TS)]
pub struct SessionMeta {
    pub id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from_id: Option<ThreadId>,
    pub timestamp: String,
    pub cwd: PathBuf,
    pub originator: String,
    pub cli_version: String,
    #[serde(default)]
    pub source: SessionSource,
    /// Optional analytics source classification for this thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_source: Option<ThreadSource>,
    /// Optional random unique nickname assigned to an AgentControl-spawned sub-agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_nickname: Option<String>,
    /// Optional role (agent_role) assigned to an AgentControl-spawned sub-agent.
    #[serde(default, alias = "agent_type", skip_serializing_if = "Option::is_none")]
    pub agent_role: Option<String>,
    /// Optional canonical agent path assigned to an AgentControl-spawned sub-agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_path: Option<String>,
    pub model_provider: Option<String>,
    /// base_instructions for the session. This *should* always be present when creating a new session,
    /// but may be missing for older sessions. If not present, fall back to rendering the base_instructions
    /// from ModelsManager.
    pub base_instructions: Option<BaseInstructions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_tools: Option<Vec<DynamicToolSpec>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscriptions: Option<Vec<PersistedSubscription>>,
}

impl Default for SessionMeta {
    fn default() -> Self {
        SessionMeta {
            id: ThreadId::default(),
            forked_from_id: None,
            timestamp: String::new(),
            cwd: PathBuf::new(),
            originator: String::new(),
            cli_version: String::new(),
            source: SessionSource::default(),
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            model_provider: None,
            base_instructions: None,
            dynamic_tools: None,
            memory_mode: None,
            subscriptions: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, TS)]
pub struct SessionMetaLine {
    #[serde(flatten)]
    pub meta: SessionMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<GitInfo>,
}
