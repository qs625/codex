use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use codex_config_state::ConfigLayerStack;
use codex_config_toml::config_toml::ConfigToml;
use codex_config_toml::config_toml::RealtimeConfig;
use codex_config_types::AuthCredentialsStoreMode;
use codex_config_types::ConfigLockfileToml;
use codex_config_types::Constrained;
use codex_config_types::History;
use codex_config_types::McpServerConfig;
use codex_config_types::MemoriesConfig;
use codex_config_types::ModelAvailabilityNuxConfig;
use codex_config_types::Notice;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_config_types::OtelConfig;
use codex_config_types::RealtimeAudioConfig;
use codex_config_types::SessionPickerViewMode;
use codex_config_types::ToolSuggestConfig;
use codex_config_types::TuiKeymap;
use codex_config_types::TuiNotificationSettings;
use codex_config_types::TuiPetAnchor;
use codex_config_types::UriBasedFileOpener;
use codex_utils_absolute_path::AbsolutePathBuf;
use model_service_api::ModelOptionToml;
use model_service_api::ModelProviderInfo;
use protocol::config_types::AltScreenMode;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::ForcedLoginMethod;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary;
use protocol::config_types::Verbosity;
use protocol::config_types::WebSearchConfig;
use protocol::config_types::WebSearchMode;
use protocol::openai_models::ModelsResponse;
use protocol::openai_models::ReasoningEffort;

use super::AgentRoleConfig;
use super::DEFAULT_MULTI_AGENT_V2_DEFAULT_WAIT_TIMEOUT_MS;
use super::DEFAULT_MULTI_AGENT_V2_MAX_CONCURRENT_THREADS_PER_SESSION;
use super::DEFAULT_MULTI_AGENT_V2_MAX_WAIT_TIMEOUT_MS;
use super::DEFAULT_MULTI_AGENT_V2_MIN_WAIT_TIMEOUT_MS;
use super::GhostSnapshotConfig;
use super::ManagedFeatures;
use super::Permissions;
use super::ProjectConfig;

/// Configured thread persistence backend.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ThreadStoreConfig {
    /// Persist threads locally using rollout JSONL files and sqlite metadata.
    #[default]
    Local,
    /// In-memory thread store for test and debug configurations.
    InMemory { id: String },
}

/// Application configuration loaded from disk and merged with overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Provenance for how this [`Config`] was derived (merged layers + enforced
    /// requirements).
    pub config_layer_stack: ConfigLayerStack,

    /// Warnings collected during config load that should be shown on startup.
    pub startup_warnings: Vec<String>,

    /// Optional override of model selection.
    pub model: Option<String>,

    /// Effective service tier request id preference for new turns.
    pub service_tier: Option<String>,

    /// Model used specifically for review sessions.
    pub review_model: Option<String>,

    /// Size of the context window for the model, in tokens.
    pub model_context_window: Option<i64>,

    /// Token usage threshold triggering auto-compaction of conversation history.
    pub model_auto_compact_token_limit: Option<i64>,

    /// Usage ratio at which auto-compaction may start considering soft signals.
    pub model_auto_compact_soft_ratio: Option<f64>,

    /// Usage ratio at which auto-compaction triggers regardless of soft signals.
    pub model_auto_compact_hard_ratio: Option<f64>,

    /// Key into the model_providers map that specifies which provider to use.
    pub model_provider_id: String,

    /// Info needed to make an API request to the model.
    pub model_provider: ModelProviderInfo,

    /// User-visible model picker entries backed by configured providers.
    pub model_options: Vec<ModelOptionToml>,

    /// Optionally specify the personality of the model
    pub personality: Option<Personality>,

    /// Effective permission configuration for shell tool execution.
    pub permissions: Permissions,

    /// Configures who approval requests are routed to for review once they have
    /// been escalated. This does not disable separate safety checks such as
    /// ARC.
    pub approvals_reviewer: ApprovalsReviewer,

    /// enforce_residency means web traffic cannot be routed outside of a
    /// particular geography. HTTP clients should direct their requests
    /// using backend-specific headers or URLs to enforce this.
    pub enforce_residency: Constrained<Option<codex_config_types::ResidencyRequirement>>,

    /// When `true`, `AgentReasoning` events emitted by the backend will be
    /// suppressed from the frontend output. This can reduce visual noise when
    /// users are only interested in the final agent responses.
    pub hide_agent_reasoning: bool,

    /// When set to `true`, `AgentReasoningRawContentEvent` events will be shown in the UI/output.
    /// Defaults to `false`.
    pub show_raw_agent_reasoning: bool,

    /// User-provided instructions injected ahead of project doc fallback.
    pub user_instructions: Option<String>,

    /// Base instructions override.
    pub base_instructions: Option<String>,

    /// Developer instructions override injected as a separate message.
    pub developer_instructions: Option<String>,

    /// Guardian-specific policy config override from requirements.toml or config.toml.
    /// This is inserted into the fixed guardian prompt template under the
    /// `# Policy Configuration` section rather than replacing the whole
    /// guardian developer prompt.
    pub guardian_policy_config: Option<String>,

    /// Whether to inject the `<permissions instructions>` developer block.
    pub include_permissions_instructions: bool,

    /// Whether to inject the `<apps_instructions>` developer block.
    pub include_apps_instructions: bool,

    /// Whether to inject the `<collaboration_mode>` developer block.
    pub include_collaboration_mode_instructions: bool,

    /// Whether to inject the `<skills_instructions>` developer block.
    pub include_skill_instructions: bool,

    /// Whether to inject the `<environment_context>` user block.
    pub include_environment_context: bool,

    /// Compact prompt override.
    pub compact_prompt: Option<String>,

    /// Optional external notifier command. When set, Codex will spawn this
    /// program after each completed *turn* (i.e. when the agent finishes
    /// processing a user submission). The value must be the full command
    /// broken into argv tokens **without** the trailing JSON argument - Codex
    /// appends one extra argument containing a JSON payload describing the
    /// event.
    ///
    /// Example `~/.codex/config.toml` snippet:
    ///
    /// ```toml
    /// notify = ["notify-send", "Codex"]
    /// ```
    ///
    /// which will be invoked as:
    ///
    /// ```shell
    /// notify-send Codex '{"type":"agent-turn-complete","turn-id":"12345"}'
    /// ```
    ///
    /// If unset the feature is disabled.
    pub notify: Option<Vec<String>>,

    /// TUI notification settings, including enabled events, delivery method, and focus condition.
    pub tui_notifications: TuiNotificationSettings,

    /// Enable ASCII animations and shimmer effects in the TUI.
    pub animations: bool,

    /// Show startup tooltips in the TUI welcome screen.
    pub show_tooltips: bool,

    /// Persisted startup availability NUX state for model tooltips.
    pub model_availability_nux: ModelAvailabilityNuxConfig,

    /// Start the composer in Vim mode (`Normal`) by default.
    pub tui_vim_mode_default: bool,

    /// Start the TUI in raw scrollback mode for copy-friendly transcript output.
    pub tui_raw_output_mode: bool,

    /// Controls whether the TUI uses the terminal's alternate screen buffer.
    ///
    /// This is the same `tui.alternate_screen` value from `config.toml`.
    /// - `auto` (default): Use alternate screen.
    /// - `always`: Always use alternate screen.
    /// - `never`: Never use alternate screen (inline mode, preserves scrollback).
    pub tui_alternate_screen: AltScreenMode,
    /// Ordered list of status line item identifiers for the TUI.
    ///
    /// When unset, the TUI defaults to: `model-with-reasoning` and `current-dir`.
    pub tui_status_line: Option<Vec<String>>,

    /// Whether to color status line items with colors from the active syntax theme.
    pub tui_status_line_use_colors: bool,

    /// Ordered list of terminal title item identifiers for the TUI.
    ///
    /// When unset, the TUI defaults to: `activity` and `project`.
    /// The `activity` item spins while working and shows an action-required
    /// message when blocked on the user.
    pub tui_terminal_title: Option<Vec<String>>,

    /// Syntax highlighting theme override (kebab-case name).
    pub tui_theme: Option<String>,

    /// Pet id preselected by the terminal pet picker.
    pub tui_pet: Option<String>,

    /// Vertical anchor used by terminal pet rendering.
    pub tui_pet_anchor: TuiPetAnchor,

    /// Preferred layout for resume/fork session picker results.
    pub tui_session_picker_view: SessionPickerViewMode,

    /// Terminal resize-reflow tuning knobs.
    pub terminal_resize_reflow: TerminalResizeReflowConfig,

    /// Keybinding overrides for the TUI.
    ///
    /// Precedence is:
    ///
    /// 1. context table (`tui.keymap.chat`, `tui.keymap.composer`, etc.)
    /// 2. `tui.keymap.global`
    /// 3. built-in defaults
    pub tui_keymap: TuiKeymap,

    /// The absolute directory that should be treated as the current working
    /// directory for the session. All relative paths inside the business-logic
    /// layer are resolved against this path.
    pub cwd: AbsolutePathBuf,

    /// Absolute runtime workspace roots for the session. Symbolic
    /// `:workspace_roots` permission entries are materialized against these
    /// roots while profile-defined workspace roots remain encoded directly in
    /// the permission profile.
    pub workspace_roots: Vec<AbsolutePathBuf>,
    /// Whether runtime workspace roots were supplied explicitly by the caller
    /// or legacy config, rather than defaulting to `cwd`.
    pub workspace_roots_explicit: bool,

    /// Preferred store for CLI auth credentials.
    /// file (default): Use a file in the Codex home directory.
    /// keyring: Use an OS-specific keyring service.
    /// auto: Use the OS-specific keyring service if available, otherwise use a file.
    pub cli_auth_credentials_store_mode: AuthCredentialsStoreMode,

    /// Definition for MCP servers that Codex can reach out to for tool calls.
    pub mcp_servers: Constrained<HashMap<String, McpServerConfig>>,

    /// Preferred store for MCP OAuth credentials.
    /// keyring: Use the OS-specific keyring service.
    ///          Credentials stored in the keyring will only be readable by Codex unless the user explicitly grants access via OS-level keyring access.
    ///          https://github.com/openai/codex/blob/main/codex-rs/rmcp-client/src/oauth.rs#L2
    /// file: CODEX_HOME/.credentials.json
    ///       This file will be readable to Codex and other applications running as the same user.
    /// auto (default): keyring if available, otherwise file.
    pub mcp_oauth_credentials_store_mode: OAuthCredentialsStoreMode,

    /// Optional fixed port to use for the local HTTP callback server used during MCP OAuth login.
    ///
    /// When unset, Codex will bind to an ephemeral port chosen by the OS.
    pub mcp_oauth_callback_port: Option<u16>,

    /// Optional redirect URI to use during MCP OAuth login.
    ///
    /// When set, this URI is used in the OAuth authorization request instead
    /// of the local listener address. The local callback listener still binds
    /// to 127.0.0.1 (using `mcp_oauth_callback_port` when provided).
    pub mcp_oauth_callback_url: Option<String>,

    /// Combined provider map (defaults plus user-defined providers).
    pub model_providers: HashMap<String, ModelProviderInfo>,

    /// Maximum number of bytes to include from an AGENTS.md project doc file.
    pub project_doc_max_bytes: usize,

    /// Additional filenames to try when looking for project-level docs.
    pub project_doc_fallback_filenames: Vec<String>,

    /// Explicit instruction files to load into model-visible user instructions.
    pub instruction_files: Vec<AbsolutePathBuf>,

    /// Token budget applied when storing tool/function outputs in the context manager.
    pub tool_output_token_limit: Option<usize>,

    /// Maximum number of agent threads that can be open concurrently.
    pub agent_max_threads: Option<usize>,
    /// Maximum runtime in seconds for agent job workers before they are failed.
    pub agent_job_max_runtime_seconds: Option<u64>,

    /// Whether to record a model-visible message when an agent turn is interrupted.
    pub agent_interrupt_message_enabled: bool,

    /// Maximum nesting depth allowed for spawned agent threads.
    pub agent_max_depth: i32,

    /// User-defined role declarations keyed by role name.
    pub agent_roles: BTreeMap<String, AgentRoleConfig>,
    /// Tool allowlist patterns inherited from the active Markdown agent role.
    pub agent_tool_patterns: Option<Vec<String>>,
    /// Skill allowlist patterns inherited from the active Markdown agent role.
    pub agent_skill_patterns: Option<Vec<String>>,

    /// Memories subsystem settings.
    pub memories: MemoriesConfig,

    /// Directory containing all Codex state (defaults to `~/.codex` but can be
    /// overridden by the `CODEX_HOME` environment variable).
    pub codex_home: AbsolutePathBuf,

    /// Directory where Codex stores the SQLite state DB.
    pub sqlite_home: PathBuf,

    /// Directory where Codex writes log files (defaults to `$CODEX_HOME/log`).
    pub log_dir: PathBuf,

    /// Directory where Codex writes effective session config lock files.
    pub config_lock_export_dir: Option<AbsolutePathBuf>,

    /// Whether config lock replay ignores Codex version drift between the
    /// lock metadata and the regenerated lock.
    pub config_lock_allow_codex_version_mismatch: bool,

    /// Whether config lock creation saves values resolved from the model
    /// catalog/session configuration.
    pub config_lock_save_fields_resolved_from_model_catalog: bool,

    /// Effective config lock used for strict replay validation.
    pub config_lock_toml: Option<Arc<ConfigLockfileToml<ConfigToml>>>,

    /// Settings that govern if and what will be written to `~/.codex/history.jsonl`.
    pub history: History,

    /// When true, session is not persisted on disk. Default to `false`
    pub ephemeral: bool,

    /// Whether enabled hooks should run without requiring persisted hook trust for this session.
    ///
    /// This is a runtime-only knob populated from invocation overrides, not from config files.
    pub bypass_hook_trust: bool,

    /// Optional URI-based file opener. If set, citations to files in the model
    /// output will be hyperlinked using the specified URI scheme.
    pub file_opener: UriBasedFileOpener,

    /// Path to the current Codex executable. This cannot be set in the config
    /// file: it must be set in code via [`super::ConfigOverrides`].
    pub codex_self_exe: Option<PathBuf>,

    /// Path to the `codex-linux-sandbox` executable. This must be set if
    /// [`codex_sandboxing_api::SandboxType::LinuxSeccomp`] is used. Note that this
    /// cannot be set in the config file: it must be set in code via
    /// [`super::ConfigOverrides`].
    ///
    /// When this program is invoked, arg0 will be set to `codex-linux-sandbox`.
    pub codex_linux_sandbox_exe: Option<PathBuf>,

    /// Path to the `codex-execve-wrapper` executable used for shell
    /// escalation. This cannot be set in the config file: it must be set in
    /// code via [`super::ConfigOverrides`].
    pub main_execve_wrapper_exe: Option<PathBuf>,

    /// Optional absolute path to patched zsh used by zsh-exec-bridge-backed shell execution.
    pub zsh_path: Option<PathBuf>,

    /// Value to use for `reasoning.effort` when making a request using the
    /// Responses API.
    pub model_reasoning_effort: Option<ReasoningEffort>,
    /// Optional Plan-mode-specific reasoning effort override used by the TUI.
    ///
    /// When unset, Plan mode uses the built-in Plan preset default (currently
    /// `medium`). When explicitly set (including `none`), this overrides the
    /// Plan preset. The `none` value means "no reasoning" (not "inherit the
    /// global default").
    pub plan_mode_reasoning_effort: Option<ReasoningEffort>,

    /// Optional value to use for `reasoning.summary` when making a request
    /// using the Responses API. When unset, the model catalog default is used.
    pub model_reasoning_summary: Option<ReasoningSummary>,

    /// Optional override to force-enable reasoning summaries for the configured model.
    pub model_supports_reasoning_summaries: Option<bool>,

    /// Optional full model catalog loaded from `model_catalog_json`.
    /// When set, this replaces the bundled catalog for the current process.
    pub model_catalog: Option<ModelsResponse>,

    /// Optional verbosity control for GPT-5 models (Responses API `text.verbosity`).
    pub model_verbosity: Option<Verbosity>,

    /// Base URL for requests to ChatGPT (as opposed to the OpenAI API).
    pub chatgpt_base_url: String,

    /// Optional path override for the host-owned apps MCP server.
    pub apps_mcp_path_override: Option<String>,

    /// Machine-local realtime audio device preferences used by realtime voice.
    pub realtime_audio: RealtimeAudioConfig,

    /// Experimental / do not use. Overrides only the realtime conversation
    /// websocket transport base URL (the `Op::RealtimeConversation`
    /// `/v1/realtime`
    /// connection) without changing normal provider HTTP requests.
    pub experimental_realtime_ws_base_url: Option<String>,
    /// Experimental / do not use. Selects the realtime websocket model/snapshot
    /// used for the `Op::RealtimeConversation` connection.
    pub experimental_realtime_ws_model: Option<String>,
    /// Experimental / do not use. Realtime websocket session selection.
    /// `version` controls v1/v2 and `type` controls conversational/transcription.
    pub realtime: RealtimeConfig,
    /// Experimental / do not use. Overrides only the realtime conversation
    /// websocket transport instructions (the `Op::RealtimeConversation`
    /// `/ws` session.update instructions) without changing normal prompts.
    pub experimental_realtime_ws_backend_prompt: Option<String>,
    /// Experimental / do not use. Replaces the synthesized realtime startup
    /// context appended to websocket session instructions. An empty string
    /// disables startup context injection entirely.
    pub experimental_realtime_ws_startup_context: Option<String>,
    /// Experimental / do not use. Replaces the built-in realtime start
    /// instructions inserted into developer messages when realtime becomes
    /// active.
    pub experimental_realtime_start_instructions: Option<String>,
    /// Experimental / do not use. When set, app-server fetches thread-scoped
    /// config from a remote service at this endpoint.
    pub experimental_thread_config_endpoint: Option<String>,

    /// Experimental / do not use. Selects the thread persistence backend.
    pub experimental_thread_store: ThreadStoreConfig,
    /// When set, restricts ChatGPT login to one or more workspace identifiers.
    pub forced_chatgpt_workspace_id: Option<Vec<String>>,

    /// When set, restricts the login mechanism users may use.
    pub forced_login_method: Option<ForcedLoginMethod>,

    /// Explicit or feature-derived web search mode.
    pub web_search_mode: Constrained<WebSearchMode>,

    /// Additional parameters for the web search tool when it is enabled.
    pub web_search_config: Option<WebSearchConfig>,

    /// If set to `true`, used only the experimental unified exec tool.
    pub use_experimental_unified_exec_tool: bool,

    /// Command session wait hard cap. This is a wait protection limit and does
    /// not terminate the underlying command.
    pub background_terminal_max_timeout: u64,

    /// Compatibility-only settings retained for legacy `ghost_snapshot`
    /// config loading.
    pub ghost_snapshot: GhostSnapshotConfig,

    /// Settings specific to the task-path-based multi-agent tool surface.
    pub multi_agent_v2: MultiAgentV2Config,

    /// Centralized feature flags; source of truth for feature gating.
    pub features: ManagedFeatures,

    /// When `true`, suppress warnings about unstable (under development) features.
    pub suppress_unstable_features_warning: bool,

    /// The active profile name used to derive this `Config` (if any).
    pub active_profile: Option<String>,

    /// The currently active project config, resolved by checking if cwd:
    /// is (1) part of a git repo, (2) a git worktree, or (3) just using the cwd
    pub active_project: ProjectConfig,

    /// Collection of various notices we show the user
    pub notices: Notice,

    /// When `true`, checks for Codex updates on startup and surfaces update prompts.
    /// Set to `false` only if your Codex updates are centrally managed.
    /// Defaults to `true`.
    pub check_for_update_on_startup: bool,

    /// When true, disables burst-paste detection for typed input entirely.
    /// All characters are inserted as they are received, and no buffering
    /// or placeholder replacement will occur for fast keypress bursts.
    pub disable_paste_burst: bool,

    /// When `false`, disables analytics across Codex product surfaces in this machine.
    /// Voluntarily left as Optional because the default value might depend on the client.
    pub analytics_enabled: Option<bool>,

    /// When `false`, disables feedback collection across Codex product surfaces.
    /// Defaults to `true`.
    pub feedback_enabled: bool,

    /// Configured discoverable tools for tool suggestions.
    pub tool_suggest: ToolSuggestConfig,

    /// OTEL configuration (exporter type, endpoint, headers, etc.).
    pub otel: OtelConfig,
}

impl rollout_api::RolloutConfigView for Config {
    fn codex_home(&self) -> &std::path::Path {
        self.codex_home.as_path()
    }

    fn sqlite_home(&self) -> &std::path::Path {
        self.sqlite_home.as_path()
    }

    fn cwd(&self) -> &std::path::Path {
        self.cwd.as_path()
    }

    fn model_provider_id(&self) -> &str {
        self.model_provider_id.as_str()
    }

    fn generate_memories(&self) -> bool {
        self.memories.generate_memories
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MultiAgentV2Config {
    pub max_concurrent_threads_per_session: usize,
    pub min_wait_timeout_ms: i64,
    pub max_wait_timeout_ms: i64,
    pub default_wait_timeout_ms: i64,
    pub usage_hint_enabled: bool,
    pub usage_hint_text: Option<String>,
    pub root_agent_usage_hint_text: Option<String>,
    pub subagent_usage_hint_text: Option<String>,
    pub hide_spawn_agent_metadata: bool,
    pub non_code_mode_only: bool,
}

impl Default for MultiAgentV2Config {
    fn default() -> Self {
        Self {
            max_concurrent_threads_per_session:
                DEFAULT_MULTI_AGENT_V2_MAX_CONCURRENT_THREADS_PER_SESSION,
            min_wait_timeout_ms: DEFAULT_MULTI_AGENT_V2_MIN_WAIT_TIMEOUT_MS,
            max_wait_timeout_ms: DEFAULT_MULTI_AGENT_V2_MAX_WAIT_TIMEOUT_MS,
            default_wait_timeout_ms: DEFAULT_MULTI_AGENT_V2_DEFAULT_WAIT_TIMEOUT_MS,
            usage_hint_enabled: true,
            usage_hint_text: None,
            root_agent_usage_hint_text: None,
            subagent_usage_hint_text: None,
            hide_spawn_agent_metadata: false,
            non_code_mode_only: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalResizeReflowMaxRows {
    /// Use the runtime terminal detector to choose a scrollback-sized cap.
    #[default]
    Auto,
    /// Keep all rendered transcript rows during resize reflow.
    Disabled,
    /// Keep at most this many rendered transcript rows during resize reflow.
    Limit(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerminalResizeReflowConfig {
    pub max_rows: TerminalResizeReflowMaxRows,
}
