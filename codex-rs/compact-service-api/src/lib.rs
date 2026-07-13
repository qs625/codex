use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::models::ResponseItem;

pub const DEFAULT_SOFT_COMPACT_LOWER_BOUND: f64 = 0.80;
pub const DEFAULT_HARD_COMPACT_BOUND: f64 = 0.90;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftCompactThresholds {
    pub soft_lower_bound: f64,
    pub hard_bound: f64,
}

impl Default for SoftCompactThresholds {
    fn default() -> Self {
        Self {
            soft_lower_bound: DEFAULT_SOFT_COMPACT_LOWER_BOUND,
            hard_bound: DEFAULT_HARD_COMPACT_BOUND,
        }
    }
}

impl SoftCompactThresholds {
    pub fn resolve(soft_lower_bound: Option<f64>, hard_bound: Option<f64>) -> Result<Self, String> {
        let thresholds = Self {
            soft_lower_bound: soft_lower_bound.unwrap_or(DEFAULT_SOFT_COMPACT_LOWER_BOUND),
            hard_bound: hard_bound.unwrap_or(DEFAULT_HARD_COMPACT_BOUND),
        };
        thresholds.validate()?;
        Ok(thresholds)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_ratio(
            "model_auto_compact_soft_ratio",
            self.soft_lower_bound,
            /*allow_one*/ false,
        )?;
        validate_ratio(
            "model_auto_compact_hard_ratio",
            self.hard_bound,
            /*allow_one*/ true,
        )?;
        if self.hard_bound <= self.soft_lower_bound {
            return Err(
                "model_auto_compact_hard_ratio must be greater than model_auto_compact_soft_ratio"
                    .to_string(),
            );
        }
        Ok(())
    }
}

fn validate_ratio(field_name: &str, value: f64, allow_one: bool) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 || value > 1.0 || (!allow_one && value >= 1.0) {
        let range = if allow_one { "(0.0, 1.0]" } else { "(0.0, 1.0)" };
        return Err(format!("{field_name} must be in range {range}"));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactMemoryBundle {
    pub snapshots: Vec<CompactMemorySnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactWindowSummary {
    pub recent_real_user_messages: Vec<String>,
    pub turns_since_last_compact: usize,
    pub recent_file_read_search_count: usize,
    pub recent_tool_output_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftCompactInputs {
    pub usage_ratio: f64,
    pub thresholds: SoftCompactThresholds,
    pub turns_since_last_compact: usize,
    pub recent_file_read_search_count: usize,
    pub recent_tool_output_bytes: usize,
    pub current_work_completeness: f64,
    pub cooldown_turns_satisfied: bool,
    pub cooldown_bytes_satisfied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftCompactDecision {
    pub should_compact: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactReplacementFile {
    pub path: AbsolutePathBuf,
    pub role: CompactMemoryRole,
    pub label: Option<String>,
    pub token_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactMemoryRole {
    CurrentWork,
    ProjectUnderstanding,
    UserPreferences,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactMemorySnapshot {
    pub role: CompactMemoryRole,
    pub label: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementHistoryInput {
    pub initial_context: Vec<ResponseItem>,
    pub memory_bundle: CompactMemoryBundle,
    pub recent_real_user_messages: Vec<String>,
    pub final_output: Option<String>,
}

impl CompactMemoryBundle {
    pub fn current_work_content(&self) -> Option<&str> {
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.role == CompactMemoryRole::CurrentWork)
            .map(|snapshot| snapshot.content.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_compact_thresholds_resolve_defaults_and_overrides() {
        assert_eq!(SoftCompactThresholds::resolve(None, None).unwrap(), {
            SoftCompactThresholds {
                soft_lower_bound: 0.80,
                hard_bound: 0.90,
            }
        });
        assert_eq!(
            SoftCompactThresholds::resolve(Some(0.62), Some(0.88)).unwrap(),
            SoftCompactThresholds {
                soft_lower_bound: 0.62,
                hard_bound: 0.88,
            }
        );
    }

    #[test]
    fn soft_compact_thresholds_reject_invalid_ranges() {
        let cases = [
            (Some(0.0), Some(0.90), "model_auto_compact_soft_ratio"),
            (Some(0.80), Some(1.1), "model_auto_compact_hard_ratio"),
            (
                Some(0.90),
                Some(0.80),
                "model_auto_compact_hard_ratio must be greater",
            ),
        ];

        for (soft_ratio, hard_ratio, expected_message) in cases {
            let err = SoftCompactThresholds::resolve(soft_ratio, hard_ratio)
                .expect_err("invalid threshold should fail");
            assert!(
                err.contains(expected_message),
                "expected {expected_message:?}, got {err}"
            );
        }
    }
}
