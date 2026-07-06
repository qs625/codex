//! Memory configuration TOML and effective settings types.

use codex_utils_absolute_path::AbsolutePathBuf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

pub const DEFAULT_MEMORIES_MAX_ROLLOUTS_PER_STARTUP: usize = 2;
pub const DEFAULT_MEMORIES_MAX_ROLLOUT_AGE_DAYS: i64 = 10;
pub const DEFAULT_MEMORIES_MIN_ROLLOUT_IDLE_HOURS: i64 = 6;
pub const DEFAULT_MEMORIES_MIN_RATE_LIMIT_REMAINING_PERCENT: i64 = 25;
pub const DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION: usize = 256;
pub const DEFAULT_MEMORIES_MAX_UNUSED_DAYS: i64 = 30;
pub const DEFAULT_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT: usize = 1_500;

const MIN_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION: usize = 1;
const MAX_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION: usize = 4096;
const MIN_MEMORIES_MAX_ROLLOUTS_PER_STARTUP: usize = 1;
const MAX_MEMORIES_MAX_ROLLOUTS_PER_STARTUP: usize = 128;
const MIN_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT: usize = 1;
const MAX_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT: usize = 20_000;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CompactReplacementFileRole {
    CurrentWork,
    ProjectUnderstanding,
    UserPreferences,
    Custom,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CompactReplacementFileToml {
    /// Relative paths are resolved relative to the `config.toml` that defines them.
    pub path: AbsolutePathBuf,
    pub role: CompactReplacementFileRole,
    pub label: Option<String>,
    #[schemars(range(min = 1, max = 20000))]
    pub token_limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompactReplacementFileConfig {
    pub path: AbsolutePathBuf,
    pub role: CompactReplacementFileRole,
    pub label: Option<String>,
    pub token_limit: usize,
}

/// Memories settings loaded from config.toml.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct MemoriesToml {
    /// When `true`, external context sources mark the thread `memory_mode` as `"polluted"`.
    #[serde(alias = "no_memories_if_mcp_or_web_search")]
    pub disable_on_external_context: Option<bool>,
    /// When `false`, newly created threads are stored with `memory_mode = "disabled"` in the state DB.
    pub generate_memories: Option<bool>,
    /// When `false`, skip injecting memory usage instructions into developer prompts.
    pub use_memories: Option<bool>,
    /// Maximum number of recent raw memories retained for global consolidation.
    #[schemars(range(min = 1, max = 4096))]
    pub max_raw_memories_for_consolidation: Option<usize>,
    /// Maximum number of days since a memory was last used before it becomes ineligible for phase 2 selection.
    pub max_unused_days: Option<i64>,
    /// Maximum age of the threads used for memories.
    pub max_rollout_age_days: Option<i64>,
    /// Maximum number of rollout candidates processed per pass.
    #[schemars(range(min = 1, max = 128))]
    pub max_rollouts_per_startup: Option<usize>,
    /// Minimum idle time between last thread activity and memory creation (hours). > 12h recommended.
    pub min_rollout_idle_hours: Option<i64>,
    /// Minimum remaining percentage required in Codex rate-limit windows before memory startup runs.
    #[schemars(range(min = 0, max = 100))]
    pub min_rate_limit_remaining_percent: Option<i64>,
    /// Model used for thread summarisation.
    pub extract_model: Option<String>,
    /// Model used for memory consolidation.
    pub consolidation_model: Option<String>,
    /// Default token cap when compact rereads replacement files after the model turn finishes.
    #[schemars(range(min = 1, max = 20000))]
    pub compact_replacement_file_token_limit: Option<usize>,
    /// Files reread after compact to rebuild replacement context.
    pub compact_replacement_files: Option<Vec<CompactReplacementFileToml>>,
}

/// Effective memories settings after defaults are applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoriesConfig {
    pub disable_on_external_context: bool,
    pub generate_memories: bool,
    pub use_memories: bool,
    pub max_raw_memories_for_consolidation: usize,
    pub max_unused_days: i64,
    pub max_rollout_age_days: i64,
    pub max_rollouts_per_startup: usize,
    pub min_rollout_idle_hours: i64,
    pub min_rate_limit_remaining_percent: i64,
    pub extract_model: Option<String>,
    pub consolidation_model: Option<String>,
    pub compact_replacement_file_token_limit: usize,
    pub compact_replacement_files: Vec<CompactReplacementFileConfig>,
}

impl Default for MemoriesConfig {
    fn default() -> Self {
        Self {
            disable_on_external_context: false,
            generate_memories: true,
            use_memories: true,
            max_raw_memories_for_consolidation: DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
            max_unused_days: DEFAULT_MEMORIES_MAX_UNUSED_DAYS,
            max_rollout_age_days: DEFAULT_MEMORIES_MAX_ROLLOUT_AGE_DAYS,
            max_rollouts_per_startup: DEFAULT_MEMORIES_MAX_ROLLOUTS_PER_STARTUP,
            min_rollout_idle_hours: DEFAULT_MEMORIES_MIN_ROLLOUT_IDLE_HOURS,
            min_rate_limit_remaining_percent: DEFAULT_MEMORIES_MIN_RATE_LIMIT_REMAINING_PERCENT,
            extract_model: None,
            consolidation_model: None,
            compact_replacement_file_token_limit: DEFAULT_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT,
            compact_replacement_files: Vec::new(),
        }
    }
}

impl From<MemoriesToml> for MemoriesConfig {
    fn from(toml: MemoriesToml) -> Self {
        Self::from_toml_with_replacement_defaults(toml, Vec::new())
    }
}

impl MemoriesConfig {
    pub fn from_toml_with_replacement_defaults(
        toml: MemoriesToml,
        default_compact_replacement_files: Vec<CompactReplacementFileConfig>,
    ) -> Self {
        let defaults = Self::default();
        let compact_replacement_file_token_limit = toml
            .compact_replacement_file_token_limit
            .unwrap_or(defaults.compact_replacement_file_token_limit)
            .clamp(
                MIN_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT,
                MAX_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT,
            );
        let compact_replacement_files = toml
            .compact_replacement_files
            .map(|files| {
                files.into_iter()
                    .map(|file| CompactReplacementFileConfig {
                        path: file.path,
                        role: file.role,
                        label: file.label,
                        token_limit: file
                            .token_limit
                            .unwrap_or(compact_replacement_file_token_limit)
                            .clamp(
                                MIN_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT,
                                MAX_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT,
                            ),
                    })
                    .collect()
            })
            .unwrap_or(default_compact_replacement_files);
        Self {
            disable_on_external_context: toml
                .disable_on_external_context
                .unwrap_or(defaults.disable_on_external_context),
            generate_memories: toml.generate_memories.unwrap_or(defaults.generate_memories),
            use_memories: toml.use_memories.unwrap_or(defaults.use_memories),
            max_raw_memories_for_consolidation: toml
                .max_raw_memories_for_consolidation
                .unwrap_or(defaults.max_raw_memories_for_consolidation)
                .clamp(
                    MIN_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
                    MAX_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
                ),
            max_unused_days: toml
                .max_unused_days
                .unwrap_or(defaults.max_unused_days)
                .clamp(0, 365),
            max_rollout_age_days: toml
                .max_rollout_age_days
                .unwrap_or(defaults.max_rollout_age_days)
                .clamp(0, 90),
            max_rollouts_per_startup: toml
                .max_rollouts_per_startup
                .unwrap_or(defaults.max_rollouts_per_startup)
                .clamp(
                    MIN_MEMORIES_MAX_ROLLOUTS_PER_STARTUP,
                    MAX_MEMORIES_MAX_ROLLOUTS_PER_STARTUP,
                ),
            min_rollout_idle_hours: toml
                .min_rollout_idle_hours
                .unwrap_or(defaults.min_rollout_idle_hours)
                .clamp(1, 48),
            min_rate_limit_remaining_percent: toml
                .min_rate_limit_remaining_percent
                .unwrap_or(defaults.min_rate_limit_remaining_percent)
                .clamp(0, 100),
            extract_model: toml.extract_model,
            consolidation_model: toml.consolidation_model,
            compact_replacement_file_token_limit,
            compact_replacement_files,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn memories_config_clamps_count_limits_to_nonzero_values() {
        let config = MemoriesConfig::from(MemoriesToml {
            max_raw_memories_for_consolidation: Some(0),
            max_rollouts_per_startup: Some(0),
            ..Default::default()
        });

        assert_eq!(
            config,
            MemoriesConfig {
                max_raw_memories_for_consolidation: 1,
                max_rollouts_per_startup: 1,
                ..MemoriesConfig::default()
            }
        );
    }

    #[test]
    fn memories_config_clamps_rate_limit_remaining_threshold() {
        let config = MemoriesConfig::from(MemoriesToml {
            min_rate_limit_remaining_percent: Some(101),
            ..Default::default()
        });
        assert_eq!(
            config,
            MemoriesConfig {
                min_rate_limit_remaining_percent: 100,
                ..MemoriesConfig::default()
            }
        );

        let config = MemoriesConfig::from(MemoriesToml {
            min_rate_limit_remaining_percent: Some(-1),
            ..Default::default()
        });
        assert_eq!(
            config,
            MemoriesConfig {
                min_rate_limit_remaining_percent: 0,
                ..MemoriesConfig::default()
            }
        );
    }

    #[test]
    fn memories_config_resolves_compact_replacement_files_with_global_default_limit() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let file_path = AbsolutePathBuf::from_absolute_path(tempdir.path().join("current-work.md"))
            .expect("absolute path");
        let config = MemoriesConfig::from(MemoriesToml {
            compact_replacement_file_token_limit: Some(321),
            compact_replacement_files: Some(vec![CompactReplacementFileToml {
                path: file_path.clone(),
                role: CompactReplacementFileRole::CurrentWork,
                label: None,
                token_limit: None,
            }]),
            ..Default::default()
        });

        assert_eq!(
            config.compact_replacement_files,
            vec![CompactReplacementFileConfig {
                path: file_path,
                role: CompactReplacementFileRole::CurrentWork,
                label: None,
                token_limit: 321,
            }]
        );
    }
}
