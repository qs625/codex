use super::JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL;
use super::JOB_KIND_MEMORY_STAGE1;
use super::MEMORY_CONSOLIDATION_JOB_KEY;
use super::PHASE2_SUCCESS_COOLDOWN_SECONDS;
use super::StateRuntime;
use super::test_support::test_thread_metadata;
use super::test_support::unique_temp_dir;
use crate::model::Phase2JobClaimOutcome;
use crate::model::Stage1JobClaimOutcome;
use crate::model::Stage1StartupClaimParams;
use chrono::Duration;
use chrono::Utc;
use pretty_assertions::assert_eq;
use protocol::ThreadId;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

fn stable_thread_id(value: &str) -> ThreadId {
    ThreadId::from_string(value).expect("thread id")
}

async fn age_phase2_success_beyond_cooldown(runtime: &StateRuntime) {
    sqlx::query("UPDATE jobs SET finished_at = ? WHERE kind = ? AND job_key = ?")
        .bind(Utc::now().timestamp() - PHASE2_SUCCESS_COOLDOWN_SECONDS - 1)
        .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
        .bind(MEMORY_CONSOLIDATION_JOB_KEY)
        .execute(runtime.pool.as_ref())
        .await
        .expect("age phase2 success beyond cooldown");
}


mod phase2_selection;
mod retention_and_lock;
mod stage1;
