//! TOML-only types used by the canonical `ConfigToml` shape.

use std::collections::HashMap;

pub use codex_config_types::AppConfig;
pub use codex_config_types::AppToolConfig;
pub use codex_config_types::AppToolsConfig;
pub use codex_config_types::AppsConfigToml;
pub use codex_config_types::AppsDefaultConfig;
pub use codex_config_types::AuthCredentialsStoreMode;
pub use codex_config_types::History;
pub use codex_config_types::MarketplaceConfig;
pub use codex_config_types::McpServerConfig;
pub use codex_config_types::MemoriesToml;
pub use codex_config_types::ModelAvailabilityNuxConfig;
pub use codex_config_types::Notice;
pub use codex_config_types::OAuthCredentialsStoreMode;
pub use codex_config_types::OtelConfigToml;
pub use codex_config_types::PluginConfig;
pub use codex_config_types::RawMcpServerConfig;
pub use codex_config_types::SandboxWorkspaceWrite;
pub use codex_config_types::SessionPickerViewMode;
pub use codex_config_types::SkillsConfig;
pub use codex_config_types::ToolSuggestConfig;
pub use codex_config_types::TuiKeymap;
pub use codex_config_types::TuiNotificationSettings;
pub use codex_config_types::TuiPetAnchor;
pub use codex_config_types::UriBasedFileOpener;
pub use codex_config_types::WindowsToml;
pub use protocol::config_types::AltScreenMode;
pub use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::EnvironmentVariablePattern;
pub use protocol::config_types::ModeKind;
pub use protocol::config_types::Personality;
pub use protocol::config_types::ServiceTier;
use protocol::config_types::ShellEnvironmentPolicy;
use protocol::config_types::ShellEnvironmentPolicyInherit;
pub use protocol::config_types::WebSearchMode;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// Analytics settings loaded from config.toml. Fields are optional so we can apply defaults.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct AnalyticsConfigToml {
    /// When `false`, disables analytics across Codex product surfaces in this profile.
    pub enabled: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct FeedbackConfigToml {
    /// When `false`, disables the feedback flow across Codex product surfaces.
    pub enabled: Option<bool>,
}

/// Collection of settings that are specific to the TUI.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct Tui {
    #[serde(default, flatten)]
    pub notification_settings: TuiNotificationSettings,

    /// Enable animations (welcome screen, shimmer effects, spinners).
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub animations: bool,

    /// Show startup tooltips in the TUI welcome screen.
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub show_tooltips: bool,

    /// Start the composer in Vim mode (`Normal`) by default.
    /// Defaults to `false`.
    #[serde(default)]
    pub vim_mode_default: bool,

    /// Start the TUI in raw scrollback mode for copy-friendly transcript output.
    /// Defaults to `false`.
    #[serde(default)]
    pub raw_output_mode: bool,

    /// Controls whether the TUI uses the terminal's alternate screen buffer.
    ///
    /// - `auto` (default): Use alternate screen.
    /// - `always`: Always use alternate screen.
    /// - `never`: Never use alternate screen (inline mode only, preserves scrollback).
    #[serde(default)]
    pub alternate_screen: AltScreenMode,

    /// Ordered list of status line item identifiers.
    ///
    /// When set, the TUI renders the selected items as the status line.
    /// When unset, the TUI defaults to: `model-with-reasoning` and `current-dir`.
    #[serde(default)]
    pub status_line: Option<Vec<String>>,

    /// Color status line items with colors derived from the active syntax theme.
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub status_line_use_colors: bool,

    /// Ordered list of terminal title item identifiers.
    ///
    /// When set, the TUI renders the selected items into the terminal window/tab title.
    /// When unset, the TUI defaults to: `activity` and `project`.
    /// The `activity` item spins while working and shows an action-required
    /// message when blocked on the user.
    #[serde(default)]
    pub terminal_title: Option<Vec<String>>,

    /// Syntax highlighting theme name (kebab-case).
    ///
    /// When set, overrides automatic light/dark theme detection.
    /// Use `/theme` in the TUI or see `$MORPHEUS_HOME/themes` for custom themes.
    #[serde(default)]
    pub theme: Option<String>,

    /// Pet id to preselect in the terminal pet picker.
    ///
    /// Custom pet ids resolve against MORPHEUS_HOME/pets/<pet-id>/pet.json.
    #[serde(default)]
    pub pet: Option<String>,

    /// Where the terminal pet should anchor vertically.
    ///
    /// Defaults to `composer`, which follows the current TUI composer viewport.
    #[serde(default)]
    pub pet_anchor: TuiPetAnchor,

    /// Preferred layout for resume/fork session picker results.
    #[serde(default)]
    pub session_picker_view: Option<SessionPickerViewMode>,

    /// Keybinding overrides for the TUI.
    ///
    /// This supports rebinding selected actions globally and by context.
    /// Context bindings take precedence over `global` bindings.
    #[serde(default)]
    pub keymap: TuiKeymap,

    /// Startup tooltip availability NUX state persisted by the TUI.
    #[serde(default)]
    pub model_availability_nux: ModelAvailabilityNuxConfig,

    /// Trim terminal resize-reflow replay to the most recent rendered terminal rows when the
    /// transcript exceeds this cap. Omit to use Codex's terminal-specific default. Set to `0` to
    /// keep all rendered rows.
    #[serde(default)]
    #[schemars(range(min = 0))]
    pub terminal_resize_reflow_max_rows: Option<usize>,
}

const fn default_true() -> bool {
    true
}

/// Policy for building the `env` when spawning a process via shell-like tools.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ShellEnvironmentPolicyToml {
    pub inherit: Option<ShellEnvironmentPolicyInherit>,

    pub ignore_default_excludes: Option<bool>,

    /// List of regular expressions.
    pub exclude: Option<Vec<String>>,

    pub r#set: Option<HashMap<String, String>>,

    /// List of regular expressions.
    pub include_only: Option<Vec<String>>,

    pub experimental_use_profile: Option<bool>,
}

impl From<ShellEnvironmentPolicyToml> for ShellEnvironmentPolicy {
    fn from(toml: ShellEnvironmentPolicyToml) -> Self {
        // Default to inheriting the full environment when not specified.
        let inherit = toml.inherit.unwrap_or(ShellEnvironmentPolicyInherit::All);
        let ignore_default_excludes = toml.ignore_default_excludes.unwrap_or(true);
        let exclude = toml
            .exclude
            .unwrap_or_default()
            .into_iter()
            .map(|s| EnvironmentVariablePattern::new_case_insensitive(&s))
            .collect();
        let r#set = toml.r#set.unwrap_or_default();
        let include_only = toml
            .include_only
            .unwrap_or_default()
            .into_iter()
            .map(|s| EnvironmentVariablePattern::new_case_insensitive(&s))
            .collect();
        let use_profile = toml.experimental_use_profile.unwrap_or(false);

        Self {
            inherit,
            ignore_default_excludes,
            exclude,
            r#set,
            include_only,
            use_profile,
        }
    }
}
