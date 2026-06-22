use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

pub use codex_config_edit::ConfigEdit;
pub use codex_config_edit::apply;
pub use codex_config_edit::apply_blocking;
pub use codex_config_edit::keymap_binding_clear_edit;
pub use codex_config_edit::keymap_binding_edit;
pub use codex_config_edit::keymap_bindings_edit;
pub use codex_config_edit::model_availability_nux_count_edits;
pub use codex_config_edit::session_picker_view_edit;
pub use codex_config_edit::status_line_items_edit;
pub use codex_config_edit::status_line_use_colors_edit;
pub use codex_config_edit::syntax_theme_edit;
pub use codex_config_edit::terminal_title_items_edit;
pub use codex_config_edit::tui_pet_edit;
use codex_config_types::McpServerConfig;
use codex_config_types::SessionPickerViewMode;
use codex_features::FEATURES;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::TrustLevel;
use codex_protocol::openai_models::ReasoningEffort;

use super::CONFIG_TOML_FILE;
use super::Config;

/// Core compatibility wrapper around the config-edit owner crate.
///
/// The config editing engine lives in `codex-config-edit`; this wrapper only
/// keeps the historical `for_config(&Config)` constructor at the core boundary.
pub struct ConfigEditsBuilder {
    inner: codex_config_edit::ConfigEditsBuilder,
}

impl ConfigEditsBuilder {
    pub fn new(codex_home: &Path) -> Self {
        Self {
            inner: with_core_feature_defaults(codex_config_edit::ConfigEditsBuilder::new(
                codex_home,
            )),
        }
    }

    pub fn for_config(config: &Config) -> Self {
        Self {
            inner: with_core_feature_defaults(
                codex_config_edit::ConfigEditsBuilder::for_config_path(
                    config_path_for_config(config).as_path(),
                ),
            ),
        }
    }

    pub fn for_config_path(config_path: &Path) -> Self {
        Self {
            inner: with_core_feature_defaults(
                codex_config_edit::ConfigEditsBuilder::for_config_path(config_path),
            ),
        }
    }

    pub fn with_profile(mut self, profile: Option<&str>) -> Self {
        self.inner = self.inner.with_profile(profile);
        self
    }

    pub fn set_model(mut self, model: Option<&str>, effort: Option<ReasoningEffort>) -> Self {
        self.inner = self
            .inner
            .set_model(model, effort.map(|effort| effort.to_string()));
        self
    }

    pub fn set_service_tier(mut self, service_tier: Option<String>) -> Self {
        self.inner = self.inner.set_service_tier(service_tier);
        self
    }

    pub fn set_personality(mut self, personality: Option<Personality>) -> Self {
        self.inner = self
            .inner
            .set_personality(personality.map(|personality| personality.to_string()));
        self
    }

    pub fn set_hide_full_access_warning(mut self, acknowledged: bool) -> Self {
        self.inner = self.inner.set_hide_full_access_warning(acknowledged);
        self
    }

    pub fn set_hide_world_writable_warning(mut self, acknowledged: bool) -> Self {
        self.inner = self.inner.set_hide_world_writable_warning(acknowledged);
        self
    }

    pub fn set_fast_default_opt_out(mut self, opted_out: bool) -> Self {
        self.inner = self.inner.set_fast_default_opt_out(opted_out);
        self
    }

    pub fn set_hide_rate_limit_model_nudge(mut self, acknowledged: bool) -> Self {
        self.inner = self.inner.set_hide_rate_limit_model_nudge(acknowledged);
        self
    }

    pub fn set_hide_model_migration_prompt(mut self, model: &str, acknowledged: bool) -> Self {
        self.inner = self
            .inner
            .set_hide_model_migration_prompt(model, acknowledged);
        self
    }

    pub fn set_hide_external_config_migration_prompt_home(mut self, acknowledged: bool) -> Self {
        self.inner = self
            .inner
            .set_hide_external_config_migration_prompt_home(acknowledged);
        self
    }

    pub fn set_hide_external_config_migration_prompt_project(
        mut self,
        project: &str,
        acknowledged: bool,
    ) -> Self {
        self.inner = self
            .inner
            .set_hide_external_config_migration_prompt_project(project, acknowledged);
        self
    }

    pub fn record_model_migration_seen(mut self, from: &str, to: &str) -> Self {
        self.inner = self.inner.record_model_migration_seen(from, to);
        self
    }

    pub fn set_model_availability_nux_count(mut self, shown_count: &HashMap<String, u32>) -> Self {
        self.inner = self.inner.set_model_availability_nux_count(shown_count);
        self
    }

    pub fn replace_mcp_servers(mut self, servers: &BTreeMap<String, McpServerConfig>) -> Self {
        self.inner = self.inner.replace_mcp_servers(servers);
        self
    }

    pub fn set_project_trust_level<P: Into<PathBuf>>(
        mut self,
        project_path: P,
        trust_level: TrustLevel,
    ) -> Self {
        self.inner = self
            .inner
            .set_project_trust_level(project_path, trust_level.to_string());
        self
    }

    pub fn set_feature_enabled(mut self, key: &str, enabled: bool) -> Self {
        self.inner = self.inner.set_feature_enabled(key, enabled);
        self
    }

    pub fn set_windows_sandbox_mode(mut self, mode: &str) -> Self {
        self.inner = self.inner.set_windows_sandbox_mode(mode);
        self
    }

    pub fn set_realtime_microphone(mut self, microphone: Option<&str>) -> Self {
        self.inner = self.inner.set_realtime_microphone(microphone);
        self
    }

    pub fn set_realtime_speaker(mut self, speaker: Option<&str>) -> Self {
        self.inner = self.inner.set_realtime_speaker(speaker);
        self
    }

    pub fn set_realtime_voice(mut self, voice: Option<&str>) -> Self {
        self.inner = self.inner.set_realtime_voice(voice);
        self
    }

    pub fn clear_legacy_windows_sandbox_keys(mut self) -> Self {
        self.inner = self.inner.clear_legacy_windows_sandbox_keys();
        self
    }

    pub fn set_session_picker_view(mut self, mode: SessionPickerViewMode) -> Self {
        self.inner = self.inner.set_session_picker_view(mode);
        self
    }

    pub fn with_edits<I>(mut self, edits: I) -> Self
    where
        I: IntoIterator<Item = ConfigEdit>,
    {
        self.inner = self.inner.with_edits(edits);
        self
    }

    pub fn apply_blocking(self) -> anyhow::Result<()> {
        self.inner.apply_blocking()
    }

    pub async fn apply(self) -> anyhow::Result<()> {
        self.inner.apply().await
    }
}

pub(crate) fn config_path_for_config(config: &Config) -> PathBuf {
    config
        .config_layer_stack
        .get_user_config_file()
        .map(codex_utils_absolute_path::AbsolutePathBuf::to_path_buf)
        .unwrap_or_else(|| config.codex_home.join(CONFIG_TOML_FILE).to_path_buf())
}

fn with_core_feature_defaults(
    builder: codex_config_edit::ConfigEditsBuilder,
) -> codex_config_edit::ConfigEditsBuilder {
    builder.with_default_false_feature_keys(
        FEATURES
            .iter()
            .filter_map(|spec| (!spec.default_enabled).then_some(spec.key)),
    )
}
