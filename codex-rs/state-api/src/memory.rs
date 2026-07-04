use crate::ThreadMetadata;
use chrono::DateTime;
use chrono::Utc;
use protocol::ThreadId;
use std::path::PathBuf;

/// Stored stage-1 memory extraction output for a single thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage1Output {
    pub thread_id: ThreadId,
    pub rollout_path: PathBuf,
    pub source_updated_at: DateTime<Utc>,
    pub raw_memory: String,
    pub rollout_summary: String,
    pub rollout_slug: Option<String>,
    pub cwd: PathBuf,
    pub git_branch: Option<String>,
    pub generated_at: DateTime<Utc>,
}

/// Result of trying to claim a stage-1 memory extraction job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage1JobClaimOutcome {
    Claimed { ownership_token: String },
    SkippedUpToDate,
    SkippedRunning,
    SkippedRetryBackoff,
    SkippedRetryExhausted,
}

/// Claimed stage-1 job with thread metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage1JobClaim {
    pub thread: ThreadMetadata,
    pub ownership_token: String,
}

#[derive(Debug, Clone)]
pub struct Stage1StartupClaimParams {
    pub scan_limit: usize,
    pub max_claimed: usize,
    pub max_age_days: i64,
    pub min_rollout_idle_hours: i64,
    pub allowed_sources: Vec<String>,
    pub lease_seconds: i64,
}

/// Result of trying to claim a phase-2 consolidation job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase2JobClaimOutcome {
    Claimed {
        ownership_token: String,
        input_watermark: i64,
    },
    SkippedRetryUnavailable,
    SkippedCooldown,
    SkippedRunning,
}
