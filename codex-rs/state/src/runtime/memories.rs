use super::threads::ThreadFilterOptions;
use super::threads::push_thread_filters;
use super::threads::push_thread_order_and_limit;
use super::*;
use crate::SortDirection;
use crate::model::Phase2JobClaimOutcome;
use crate::model::Stage1JobClaim;
use crate::model::Stage1JobClaimOutcome;
use crate::model::Stage1Output;
use crate::model::Stage1OutputRow;
use crate::model::Stage1StartupClaimParams;
use crate::model::ThreadRow;
use chrono::Duration;
use sqlx::Executor;
use sqlx::QueryBuilder;
use sqlx::Sqlite;
use uuid::Uuid;

const JOB_KIND_MEMORY_STAGE1: &str = "memory_stage1";
const JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL: &str = "memory_consolidate_global";
const MEMORY_CONSOLIDATION_JOB_KEY: &str = "global";
const PHASE2_SUCCESS_COOLDOWN_SECONDS: i64 = 6 * 60 * 60;

const DEFAULT_RETRY_REMAINING: i64 = 3;

impl StateRuntime {
    /// Deletes all persisted memory state in one transaction.
    ///
    /// This removes every `stage1_outputs` row and all `jobs` rows for the
    /// stage-1 (`memory_stage1`) and phase-2 (`memory_consolidate_global`)
    /// memory pipelines.
    pub async fn clear_memory_data(&self) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            r#"
DELETE FROM stage1_outputs
            "#,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
DELETE FROM jobs
WHERE kind = ? OR kind = ?
            "#,
        )
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Record usage for cited stage-1 outputs.
    ///
    /// Each thread id increments `usage_count` by one and sets `last_usage` to
    /// the current Unix timestamp. Missing rows are ignored.
    pub async fn record_stage1_output_usage(
        &self,
        thread_ids: &[ThreadId],
    ) -> anyhow::Result<usize> {
        if thread_ids.is_empty() {
            return Ok(0);
        }

        let now = Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        let mut updated_rows = 0;

        for thread_id in thread_ids {
            updated_rows += sqlx::query(
                r#"
UPDATE stage1_outputs
SET
    usage_count = COALESCE(usage_count, 0) + 1,
    last_usage = ?
WHERE thread_id = ?
                "#,
            )
            .bind(now)
            .bind(thread_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected() as usize;
        }

        tx.commit().await?;
        Ok(updated_rows)
    }

    /// Selects and claims stage-1 startup jobs for stale threads.
    ///
    /// Query behavior:
    /// - starts from `threads` filtered to active threads and allowed sources
    ///   (`push_thread_filters`)
    /// - excludes threads with `memory_mode != 'enabled'`
    /// - excludes the current thread id
    /// - keeps only threads whose millisecond `updated_at` is in the age window
    /// - keeps only threads whose memory is stale compared to millisecond `updated_at`
    /// - orders by `updated_at_ms DESC` and applies `scan_limit`
    ///
    /// For each selected thread, this function calls [`Self::try_claim_stage1_job`]
    /// with `source_updated_at = thread.updated_at.timestamp()` and returns up to
    /// `max_claimed` successful claims.
    pub async fn claim_stage1_jobs_for_startup(
        &self,
        current_thread_id: ThreadId,
        params: Stage1StartupClaimParams<'_>,
    ) -> anyhow::Result<Vec<Stage1JobClaim>> {
        let Stage1StartupClaimParams {
            scan_limit,
            max_claimed,
            max_age_days,
            min_rollout_idle_hours,
            allowed_sources,
            lease_seconds,
        } = params;
        if scan_limit == 0 || max_claimed == 0 {
            return Ok(Vec::new());
        }

        let worker_id = current_thread_id;
        let current_thread_id = worker_id.to_string();
        let max_age_cutoff = (Utc::now() - Duration::days(max_age_days.max(0))).timestamp_millis();
        let idle_cutoff =
            (Utc::now() - Duration::hours(min_rollout_idle_hours.max(0))).timestamp_millis();

        let mut builder = QueryBuilder::<Sqlite>::new(
            r#"
SELECT
    threads.id,
    threads.rollout_path,
    threads.created_at_ms AS created_at,
    threads.updated_at_ms AS updated_at,
    threads.source,
    threads.thread_source,
    threads.agent_path,
    threads.agent_nickname,
    threads.agent_role,
    threads.model_provider,
    threads.model,
    threads.reasoning_effort,
    threads.cwd,
    threads.cli_version,
    threads.title,
    threads.preview,
    threads.sandbox_policy,
    threads.approval_mode,
    threads.tokens_used,
    threads.first_user_message,
    threads.archived_at,
    threads.git_sha,
    threads.git_branch,
    threads.git_origin_url
FROM threads
LEFT JOIN stage1_outputs
    ON stage1_outputs.thread_id = threads.id
LEFT JOIN jobs
    ON jobs.kind = 
            "#,
        );
        builder.push_bind(JOB_KIND_MEMORY_STAGE1);
        builder.push(
            r#"
   AND jobs.job_key = threads.id
            "#,
        );
        push_thread_filters(
            &mut builder,
            ThreadFilterOptions {
                archived_only: false,
                allowed_sources,
                model_providers: None,
                cwd_filters: None,
                anchor: None,
                sort_key: SortKey::UpdatedAt,
                sort_direction: SortDirection::Desc,
                search_term: None,
            },
        );
        builder.push(" AND threads.memory_mode = 'enabled'");
        builder
            .push(" AND threads.id != ")
            .push_bind(current_thread_id.as_str());
        builder
            .push(" AND ")
            .push("threads.updated_at_ms")
            .push(" >= ")
            .push_bind(max_age_cutoff);
        builder
            .push(" AND ")
            .push("threads.updated_at_ms")
            .push(" <= ")
            .push_bind(idle_cutoff);
        let updated_at = "threads.updated_at_ms";
        builder.push(" AND ((COALESCE(stage1_outputs.source_updated_at, -1) + 1) * 1000) <= ");
        builder.push(updated_at);
        builder.push(" AND ((COALESCE(jobs.last_success_watermark, -1) + 1) * 1000) <= ");
        builder.push(updated_at);
        push_thread_order_and_limit(
            &mut builder,
            SortKey::UpdatedAt,
            SortDirection::Desc,
            scan_limit,
        );

        let items = builder
            .build()
            .fetch_all(self.pool.as_ref())
            .await?
            .into_iter()
            .map(|row| ThreadRow::try_from_row(&row).and_then(ThreadMetadata::try_from))
            .collect::<Result<Vec<_>, _>>()?;

        let mut claimed = Vec::new();

        for item in items {
            if claimed.len() >= max_claimed {
                break;
            }

            if let Stage1JobClaimOutcome::Claimed { ownership_token } = self
                .try_claim_stage1_job(
                    item.id,
                    worker_id,
                    item.updated_at.timestamp(),
                    lease_seconds,
                    max_claimed,
                )
                .await?
            {
                claimed.push(Stage1JobClaim {
                    thread: item,
                    ownership_token,
                });
            }
        }

        Ok(claimed)
    }

    /// Lists the most recent non-empty stage-1 outputs for global consolidation.
    ///
    /// Query behavior:
    /// - filters out rows where both `raw_memory` and `rollout_summary` are blank
    /// - joins `threads` to include thread `cwd`, `rollout_path`, and `git_branch`
    /// - orders by `source_updated_at DESC, thread_id DESC`
    /// - applies `LIMIT n`
    pub async fn list_stage1_outputs_for_global(
        &self,
        n: usize,
    ) -> anyhow::Result<Vec<Stage1Output>> {
        if n == 0 {
            return Ok(Vec::new());
        }

        let rows = sqlx::query(
            r#"
SELECT
    so.thread_id,
    COALESCE(t.rollout_path, '') AS rollout_path,
    so.source_updated_at,
    so.raw_memory,
    so.rollout_summary,
    so.rollout_slug,
    so.generated_at,
    COALESCE(t.cwd, '') AS cwd,
    t.git_branch AS git_branch
FROM stage1_outputs AS so
LEFT JOIN threads AS t
    ON t.id = so.thread_id
WHERE t.memory_mode = 'enabled'
  AND (length(trim(so.raw_memory)) > 0 OR length(trim(so.rollout_summary)) > 0)
ORDER BY so.source_updated_at DESC, so.thread_id DESC
LIMIT ?
            "#,
        )
        .bind(n as i64)
        .fetch_all(self.pool.as_ref())
        .await?;

        rows.into_iter()
            .map(|row| Stage1OutputRow::try_from_row(&row).and_then(Stage1Output::try_from))
            .collect::<Result<Vec<_>, _>>()
    }

    /// Prunes stale stage-1 outputs while preserving the latest phase-2
    /// baseline and stage-1 job watermarks.
    ///
    /// Query behavior:
    /// - considers only rows with `selected_for_phase2 = 0`
    /// - keeps recency as `COALESCE(last_usage, source_updated_at)`
    /// - removes rows older than `max_unused_days`
    /// - prunes at most `limit` rows ordered from stalest to newest
    pub async fn prune_stage1_outputs_for_retention(
        &self,
        max_unused_days: i64,
        limit: usize,
    ) -> anyhow::Result<usize> {
        if limit == 0 {
            return Ok(0);
        }

        let cutoff = (Utc::now() - Duration::days(max_unused_days.max(0))).timestamp();
        let rows_affected = sqlx::query(
            r#"
DELETE FROM stage1_outputs
WHERE thread_id IN (
    SELECT thread_id
    FROM stage1_outputs
    WHERE selected_for_phase2 = 0
      AND COALESCE(last_usage, source_updated_at) < ?
    ORDER BY
      COALESCE(last_usage, source_updated_at) ASC,
      source_updated_at ASC,
      thread_id ASC
    LIMIT ?
)
            "#,
        )
        .bind(cutoff)
        .bind(limit as i64)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected();

        Ok(rows_affected as usize)
    }

    /// Returns the current phase-2 input set.
    ///
    /// Query behavior:
    /// - current selection keeps only non-empty stage-1 outputs whose
    ///   `last_usage` is within `max_unused_days`, or whose
    ///   `source_updated_at` is within that window when the memory has never
    ///   been used
    /// - eligible rows are ranked by `usage_count DESC`,
    ///   `COALESCE(last_usage, source_updated_at) DESC`, `source_updated_at DESC`,
    ///   `thread_id DESC`
    /// - the selected top-N rows are returned in stable `thread_id ASC` order
    ///
    /// The returned rows are the complete Phase 2 filesystem input. Phase 2
    /// syncs these rows directly; deletions are represented by the workspace
    /// diff against the previous successful memory baseline.
    pub async fn get_phase2_input_selection(
        &self,
        n: usize,
        max_unused_days: i64,
    ) -> anyhow::Result<Vec<Stage1Output>> {
        if n == 0 {
            return Ok(Vec::new());
        }
        let cutoff = (Utc::now() - Duration::days(max_unused_days.max(0))).timestamp();

        let current_rows = sqlx::query(
            r#"
SELECT
    selected.thread_id,
    selected.rollout_path,
    selected.source_updated_at,
    selected.raw_memory,
    selected.rollout_summary,
    selected.rollout_slug,
    selected.generated_at,
    selected.cwd,
    selected.git_branch
FROM (
    SELECT
        so.thread_id,
        COALESCE(t.rollout_path, '') AS rollout_path,
        so.source_updated_at,
        so.raw_memory,
        so.rollout_summary,
        so.rollout_slug,
        so.generated_at,
        COALESCE(t.cwd, '') AS cwd,
        t.git_branch AS git_branch
    FROM stage1_outputs AS so
    LEFT JOIN threads AS t
        ON t.id = so.thread_id
    WHERE t.memory_mode = 'enabled'
      AND (length(trim(so.raw_memory)) > 0 OR length(trim(so.rollout_summary)) > 0)
      AND (
            (so.last_usage IS NOT NULL AND so.last_usage >= ?)
            OR (so.last_usage IS NULL AND so.source_updated_at >= ?)
      )
    ORDER BY
        COALESCE(so.usage_count, 0) DESC,
        COALESCE(so.last_usage, so.source_updated_at) DESC,
        so.source_updated_at DESC,
        so.thread_id DESC
    LIMIT ?
) AS selected
ORDER BY selected.thread_id ASC
            "#,
        )
        .bind(cutoff)
        .bind(cutoff)
        .bind(n as i64)
        .fetch_all(self.pool.as_ref())
        .await?;

        let mut selected = Vec::with_capacity(current_rows.len());
        for row in current_rows {
            selected.push(Stage1Output::try_from(Stage1OutputRow::try_from_row(
                &row,
            )?)?);
        }

        Ok(selected)
    }

    /// Marks a thread as polluted and enqueues phase-2 forgetting when the
    /// thread participated in the last successful phase-2 baseline.
    pub async fn mark_thread_memory_mode_polluted(
        &self,
        thread_id: ThreadId,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let thread_id = thread_id.to_string();
        let mut tx = self.pool.begin().await?;
        let rows_affected = sqlx::query(
            r#"
UPDATE threads
SET memory_mode = 'polluted'
WHERE id = ? AND memory_mode != 'polluted'
            "#,
        )
        .bind(thread_id.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if rows_affected == 0 {
            tx.commit().await?;
            return Ok(false);
        }

        let selected_for_phase2 = sqlx::query_scalar::<_, i64>(
            r#"
SELECT selected_for_phase2
FROM stage1_outputs
WHERE thread_id = ?
            "#,
        )
        .bind(thread_id.as_str())
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);
        if selected_for_phase2 != 0 {
            enqueue_global_consolidation_with_executor(&mut *tx, now).await?;
        }

        tx.commit().await?;
        Ok(true)
    }

    /// Attempts to claim a stage-1 job for a thread at `source_updated_at`.
    ///
    /// Claim semantics:
    /// - skips as up-to-date when either:
    ///   - `stage1_outputs.source_updated_at >= source_updated_at`, or
    ///   - `jobs.last_success_watermark >= source_updated_at`
    /// - inserts or updates a `jobs` row to `running` only when:
    ///   - global running job count for `memory_stage1` is below `max_running_jobs`
    ///   - existing row is not actively running with a valid lease
    ///   - retry backoff (if present) has elapsed, or `source_updated_at` advanced
    ///   - retries remain, or `source_updated_at` advanced (which resets retries)
    ///
    /// The update path refreshes ownership token, lease, and `input_watermark`.
    /// If claiming fails, a follow-up read maps current row state to a precise
    /// skip outcome (`SkippedRunning`, `SkippedRetryBackoff`, or
    /// `SkippedRetryExhausted`).
    pub async fn try_claim_stage1_job(
        &self,
        thread_id: ThreadId,
        worker_id: ThreadId,
        source_updated_at: i64,
        lease_seconds: i64,
        max_running_jobs: usize,
    ) -> anyhow::Result<Stage1JobClaimOutcome> {
        let now = Utc::now().timestamp();
        let lease_until = now.saturating_add(lease_seconds.max(0));
        let max_running_jobs = max_running_jobs as i64;
        let ownership_token = Uuid::new_v4().to_string();
        let thread_id = thread_id.to_string();
        let worker_id = worker_id.to_string();

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let existing_output = sqlx::query(
            r#"
SELECT source_updated_at
FROM stage1_outputs
WHERE thread_id = ?
            "#,
        )
        .bind(thread_id.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing_output) = existing_output {
            let existing_source_updated_at: i64 = existing_output.try_get("source_updated_at")?;
            if existing_source_updated_at >= source_updated_at {
                tx.commit().await?;
                return Ok(Stage1JobClaimOutcome::SkippedUpToDate);
            }
        }
        let existing_job = sqlx::query(
            r#"
SELECT last_success_watermark
FROM jobs
WHERE kind = ? AND job_key = ?
            "#,
        )
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(thread_id.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing_job) = existing_job {
            let last_success_watermark =
                existing_job.try_get::<Option<i64>, _>("last_success_watermark")?;
            if last_success_watermark.is_some_and(|watermark| watermark >= source_updated_at) {
                tx.commit().await?;
                return Ok(Stage1JobClaimOutcome::SkippedUpToDate);
            }
        }

        let rows_affected = sqlx::query(
            r#"
INSERT INTO jobs (
    kind,
    job_key,
    status,
    worker_id,
    ownership_token,
    started_at,
    finished_at,
    lease_until,
    retry_at,
    retry_remaining,
    last_error,
    input_watermark,
    last_success_watermark
)
SELECT ?, ?, 'running', ?, ?, ?, NULL, ?, NULL, ?, NULL, ?, NULL
WHERE (
    SELECT COUNT(*)
    FROM jobs
    WHERE kind = ?
      AND status = 'running'
      AND lease_until IS NOT NULL
      AND lease_until > ?
) < ?
ON CONFLICT(kind, job_key) DO UPDATE SET
    status = 'running',
    worker_id = excluded.worker_id,
    ownership_token = excluded.ownership_token,
    started_at = excluded.started_at,
    finished_at = NULL,
    lease_until = excluded.lease_until,
    retry_at = NULL,
    retry_remaining = CASE
        WHEN excluded.input_watermark > COALESCE(jobs.input_watermark, -1) THEN ?
        ELSE jobs.retry_remaining
    END,
    last_error = NULL,
    input_watermark = excluded.input_watermark
WHERE
    (jobs.status != 'running' OR jobs.lease_until IS NULL OR jobs.lease_until <= excluded.started_at)
    AND (
        jobs.retry_at IS NULL
        OR jobs.retry_at <= excluded.started_at
        OR excluded.input_watermark > COALESCE(jobs.input_watermark, -1)
    )
    AND (
        jobs.retry_remaining > 0
        OR excluded.input_watermark > COALESCE(jobs.input_watermark, -1)
    )
    AND (
        SELECT COUNT(*)
        FROM jobs AS running_jobs
        WHERE running_jobs.kind = excluded.kind
          AND running_jobs.status = 'running'
          AND running_jobs.lease_until IS NOT NULL
          AND running_jobs.lease_until > excluded.started_at
          AND running_jobs.job_key != excluded.job_key
    ) < ?
            "#,
        )
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(thread_id.as_str())
        .bind(worker_id.as_str())
        .bind(ownership_token.as_str())
        .bind(now)
        .bind(lease_until)
        .bind(DEFAULT_RETRY_REMAINING)
        .bind(source_updated_at)
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(now)
        .bind(max_running_jobs)
        .bind(DEFAULT_RETRY_REMAINING)
        .bind(max_running_jobs)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if rows_affected > 0 {
            tx.commit().await?;
            return Ok(Stage1JobClaimOutcome::Claimed { ownership_token });
        }

        let existing_job = sqlx::query(
            r#"
SELECT status, lease_until, retry_at, retry_remaining
FROM jobs
WHERE kind = ? AND job_key = ?
            "#,
        )
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(thread_id.as_str())
        .fetch_optional(&mut *tx)
        .await?;

        tx.commit().await?;

        if let Some(existing_job) = existing_job {
            let status: String = existing_job.try_get("status")?;
            let existing_lease_until: Option<i64> = existing_job.try_get("lease_until")?;
            let retry_at: Option<i64> = existing_job.try_get("retry_at")?;
            let retry_remaining: i64 = existing_job.try_get("retry_remaining")?;

            if retry_remaining <= 0 {
                return Ok(Stage1JobClaimOutcome::SkippedRetryExhausted);
            }
            if retry_at.is_some_and(|retry_at| retry_at > now) {
                return Ok(Stage1JobClaimOutcome::SkippedRetryBackoff);
            }
            if status == "running"
                && existing_lease_until.is_some_and(|lease_until| lease_until > now)
            {
                return Ok(Stage1JobClaimOutcome::SkippedRunning);
            }
        }

        Ok(Stage1JobClaimOutcome::SkippedRunning)
    }

    /// Marks a claimed stage-1 job successful and upserts generated output.
    ///
    /// Transaction behavior:
    /// - updates `jobs` only for the currently owned running row
    /// - sets `status='done'` and `last_success_watermark = input_watermark`
    /// - upserts `stage1_outputs` for the thread, replacing existing output only
    ///   when `source_updated_at` is newer or equal
    /// - preserves any existing `selected_for_phase2` baseline until the next
    ///   successful phase-2 run rewrites the baseline selection, including the
    ///   snapshot timestamp chosen during that run
    /// - persists optional `rollout_slug` for rollout summary artifact naming
    /// - enqueues/advances the global phase-2 job watermark using
    ///   `source_updated_at`
    pub async fn mark_stage1_job_succeeded(
        &self,
        thread_id: ThreadId,
        ownership_token: &str,
        source_updated_at: i64,
        raw_memory: &str,
        rollout_summary: &str,
        rollout_slug: Option<&str>,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let thread_id = thread_id.to_string();

        let mut tx = self.pool.begin().await?;
        let rows_affected = sqlx::query(
            r#"
UPDATE jobs
SET
    status = 'done',
    finished_at = ?,
    lease_until = NULL,
    last_error = NULL,
    last_success_watermark = input_watermark
WHERE kind = ? AND job_key = ?
  AND status = 'running' AND ownership_token = ?
            "#,
        )
        .bind(now)
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(thread_id.as_str())
        .bind(ownership_token)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if rows_affected == 0 {
            tx.commit().await?;
            return Ok(false);
        }

        sqlx::query(
            r#"
INSERT INTO stage1_outputs (
    thread_id,
    source_updated_at,
    raw_memory,
    rollout_summary,
    rollout_slug,
    generated_at
) VALUES (?, ?, ?, ?, ?, ?)
ON CONFLICT(thread_id) DO UPDATE SET
    source_updated_at = excluded.source_updated_at,
    raw_memory = excluded.raw_memory,
    rollout_summary = excluded.rollout_summary,
    rollout_slug = excluded.rollout_slug,
    generated_at = excluded.generated_at
WHERE excluded.source_updated_at >= stage1_outputs.source_updated_at
            "#,
        )
        .bind(thread_id.as_str())
        .bind(source_updated_at)
        .bind(raw_memory)
        .bind(rollout_summary)
        .bind(rollout_slug)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        enqueue_global_consolidation_with_executor(&mut *tx, source_updated_at).await?;

        tx.commit().await?;
        Ok(true)
    }

    /// Marks a claimed stage-1 job successful when extraction produced no output.
    ///
    /// Transaction behavior:
    /// - updates `jobs` only for the currently owned running row
    /// - sets `status='done'` and `last_success_watermark = input_watermark`
    /// - deletes any existing `stage1_outputs` row for the thread
    /// - enqueues/advances the global phase-2 job watermark using the claimed
    ///   `input_watermark` only when deleting an existing `stage1_outputs` row
    pub async fn mark_stage1_job_succeeded_no_output(
        &self,
        thread_id: ThreadId,
        ownership_token: &str,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let thread_id = thread_id.to_string();

        let mut tx = self.pool.begin().await?;
        let rows_affected = sqlx::query(
            r#"
UPDATE jobs
SET
    status = 'done',
    finished_at = ?,
    lease_until = NULL,
    last_error = NULL,
    last_success_watermark = input_watermark
WHERE kind = ? AND job_key = ?
  AND status = 'running' AND ownership_token = ?
            "#,
        )
        .bind(now)
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(thread_id.as_str())
        .bind(ownership_token)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if rows_affected == 0 {
            tx.commit().await?;
            return Ok(false);
        }

        let source_updated_at = sqlx::query(
            r#"
SELECT input_watermark
FROM jobs
WHERE kind = ? AND job_key = ? AND ownership_token = ?
            "#,
        )
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(thread_id.as_str())
        .bind(ownership_token)
        .fetch_one(&mut *tx)
        .await?
        .try_get::<i64, _>("input_watermark")?;

        let deleted_rows = sqlx::query(
            r#"
DELETE FROM stage1_outputs
WHERE thread_id = ?
            "#,
        )
        .bind(thread_id.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if deleted_rows > 0 {
            enqueue_global_consolidation_with_executor(&mut *tx, source_updated_at).await?;
        }

        tx.commit().await?;
        Ok(true)
    }

    /// Marks a claimed stage-1 job as failed and schedules retry backoff.
    ///
    /// Query behavior:
    /// - updates only the owned running row for `(kind='memory_stage1', job_key)`
    /// - sets `status='error'`, clears lease, writes `last_error`
    /// - decrements `retry_remaining`
    /// - sets `retry_at = now + retry_delay_seconds`
    pub async fn mark_stage1_job_failed(
        &self,
        thread_id: ThreadId,
        ownership_token: &str,
        failure_reason: &str,
        retry_delay_seconds: i64,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let retry_at = now.saturating_add(retry_delay_seconds.max(0));
        let thread_id = thread_id.to_string();

        let rows_affected = sqlx::query(
            r#"
UPDATE jobs
SET
    status = 'error',
    finished_at = ?,
    lease_until = NULL,
    retry_at = ?,
    retry_remaining = retry_remaining - 1,
    last_error = ?
WHERE kind = ? AND job_key = ?
  AND status = 'running' AND ownership_token = ?
            "#,
        )
        .bind(now)
        .bind(retry_at)
        .bind(failure_reason)
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(thread_id.as_str())
        .bind(ownership_token)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected();

        Ok(rows_affected > 0)
    }

    /// Enqueues or advances the global phase-2 consolidation job watermark.
    ///
    /// The underlying upsert keeps the job `running` when already running, resets
    /// `pending/error` jobs to `pending`, and advances `input_watermark` as
    /// bookkeeping even when `source_updated_at` is older than prior maxima.
    /// Phase 2 does not use this watermark as a dirty check; git workspace diffing
    /// decides whether consolidation work exists after the lock is claimed.
    pub async fn enqueue_global_consolidation(&self, input_watermark: i64) -> anyhow::Result<()> {
        enqueue_global_consolidation_with_executor(self.pool.as_ref(), input_watermark).await
    }

    /// Attempts to claim the global phase-2 consolidation lock.
    ///
    /// Claim semantics:
    /// - reads the singleton global job row (`kind='memory_consolidate_global'`)
    /// - creates and claims the singleton row when it does not exist yet
    /// - does not use DB watermarks to decide whether Phase 2 has work; git workspace
    ///   dirtiness is the source of truth after the caller materializes inputs
    /// - returns `SkippedRetryUnavailable` when retry backoff is active
    /// - returns `SkippedRunning` when an active running lease exists
    /// - returns `SkippedCooldown` when the latest successful run finished
    ///   within the phase-2 success cooldown
    /// - otherwise updates the row to `running`, sets ownership + lease, and
    ///   returns `Claimed`
    pub async fn try_claim_global_phase2_job(
        &self,
        worker_id: ThreadId,
        lease_seconds: i64,
    ) -> anyhow::Result<Phase2JobClaimOutcome> {
        let now = Utc::now().timestamp();
        let lease_until = now.saturating_add(lease_seconds.max(0));
        let cooldown_cutoff = now.saturating_sub(PHASE2_SUCCESS_COOLDOWN_SECONDS);
        let ownership_token = Uuid::new_v4().to_string();
        let worker_id = worker_id.to_string();

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let existing_job = sqlx::query(
            r#"
SELECT status, lease_until, retry_at, input_watermark, finished_at, last_error
FROM jobs
WHERE kind = ? AND job_key = ?
            "#,
        )
        .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
        .bind(MEMORY_CONSOLIDATION_JOB_KEY)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(existing_job) = existing_job else {
            let rows_affected = sqlx::query(
                r#"
INSERT INTO jobs (
    kind,
    job_key,
    status,
    worker_id,
    ownership_token,
    started_at,
    finished_at,
    lease_until,
    retry_at,
    retry_remaining,
    last_error,
    input_watermark,
    last_success_watermark
) VALUES (?, ?, 'running', ?, ?, ?, NULL, ?, NULL, ?, NULL, 0, 0)
                "#,
            )
            .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
            .bind(MEMORY_CONSOLIDATION_JOB_KEY)
            .bind(worker_id.as_str())
            .bind(ownership_token.as_str())
            .bind(now)
            .bind(lease_until)
            .bind(DEFAULT_RETRY_REMAINING)
            .execute(&mut *tx)
            .await?
            .rows_affected();

            tx.commit().await?;
            return if rows_affected == 0 {
                Ok(Phase2JobClaimOutcome::SkippedRunning)
            } else {
                Ok(Phase2JobClaimOutcome::Claimed {
                    ownership_token,
                    input_watermark: 0,
                })
            };
        };

        let input_watermark: Option<i64> = existing_job.try_get("input_watermark")?;
        let input_watermark_value = input_watermark.unwrap_or(0);
        let status: String = existing_job.try_get("status")?;
        let existing_lease_until: Option<i64> = existing_job.try_get("lease_until")?;
        let retry_at: Option<i64> = existing_job.try_get("retry_at")?;
        let finished_at: Option<i64> = existing_job.try_get("finished_at")?;
        let last_error: Option<String> = existing_job.try_get("last_error")?;
        if retry_at.is_some_and(|retry_at| retry_at > now) {
            tx.commit().await?;
            return Ok(Phase2JobClaimOutcome::SkippedRetryUnavailable);
        }
        if status == "running" && existing_lease_until.is_some_and(|lease_until| lease_until > now)
        {
            tx.commit().await?;
            return Ok(Phase2JobClaimOutcome::SkippedRunning);
        }
        if last_error.is_none()
            && finished_at.is_some_and(|finished_at| finished_at > cooldown_cutoff)
        {
            tx.commit().await?;
            return Ok(Phase2JobClaimOutcome::SkippedCooldown);
        }

        let rows_affected = sqlx::query(
            r#"
UPDATE jobs
SET
    status = 'running',
    worker_id = ?,
    ownership_token = ?,
    started_at = ?,
    finished_at = NULL,
    lease_until = ?,
    retry_at = NULL,
    last_error = NULL
WHERE kind = ? AND job_key = ?
  AND (status != 'running' OR lease_until IS NULL OR lease_until <= ?)
  AND (retry_at IS NULL OR retry_at <= ?)
  AND (last_error IS NOT NULL OR finished_at IS NULL OR finished_at <= ?)
            "#,
        )
        .bind(worker_id.as_str())
        .bind(ownership_token.as_str())
        .bind(now)
        .bind(lease_until)
        .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
        .bind(MEMORY_CONSOLIDATION_JOB_KEY)
        .bind(now)
        .bind(now)
        .bind(cooldown_cutoff)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        tx.commit().await?;
        if rows_affected == 0 {
            Ok(Phase2JobClaimOutcome::SkippedRunning)
        } else {
            Ok(Phase2JobClaimOutcome::Claimed {
                ownership_token,
                input_watermark: input_watermark_value,
            })
        }
    }

    /// Extends the lease for an owned running phase-2 global job.
    ///
    /// Query behavior:
    /// - `UPDATE jobs SET lease_until = ?` for the singleton global row
    /// - requires `status='running'` and matching `ownership_token`
    pub async fn heartbeat_global_phase2_job(
        &self,
        ownership_token: &str,
        lease_seconds: i64,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let lease_until = now.saturating_add(lease_seconds.max(0));
        let rows_affected = sqlx::query(
            r#"
UPDATE jobs
SET lease_until = ?
WHERE kind = ? AND job_key = ?
  AND status = 'running' AND ownership_token = ?
            "#,
        )
        .bind(lease_until)
        .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
        .bind(MEMORY_CONSOLIDATION_JOB_KEY)
        .bind(ownership_token)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected();

        Ok(rows_affected > 0)
    }

    /// Marks the owned running global phase-2 job as succeeded.
    ///
    /// Query behavior:
    /// - updates only the owned running singleton global row
    /// - sets `status='done'`, clears lease/errors
    /// - advances `last_success_watermark` to
    ///   `max(existing_last_success_watermark, completed_watermark)`
    /// - rewrites `selected_for_phase2` so only the exact selected stage-1
    ///   snapshots remain marked as part of the latest successful phase-2
    ///   selection, and persists each selected snapshot's `source_updated_at`
    pub async fn mark_global_phase2_job_succeeded(
        &self,
        ownership_token: &str,
        completed_watermark: i64,
        selected_outputs: &[Stage1Output],
    ) -> anyhow::Result<bool> {
        let mut tx = self.pool.begin().await?;
        let rows_affected =
            mark_global_phase2_job_succeeded_row(&mut *tx, ownership_token, completed_watermark)
                .await?;

        if rows_affected == 0 {
            tx.commit().await?;
            return Ok(false);
        }

        sqlx::query(
            r#"
UPDATE stage1_outputs
SET
    selected_for_phase2 = 0,
    selected_for_phase2_source_updated_at = NULL
WHERE selected_for_phase2 != 0 OR selected_for_phase2_source_updated_at IS NOT NULL
            "#,
        )
        .execute(&mut *tx)
        .await?;

        for output in selected_outputs {
            sqlx::query(
                r#"
UPDATE stage1_outputs
SET
    selected_for_phase2 = 1,
    selected_for_phase2_source_updated_at = ?
WHERE thread_id = ? AND source_updated_at = ?
                "#,
            )
            .bind(output.source_updated_at.timestamp())
            .bind(output.thread_id.to_string())
            .bind(output.source_updated_at.timestamp())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(true)
    }

    /// Marks the owned running global phase-2 job as failed and schedules retry.
    ///
    /// Query behavior:
    /// - updates only the owned running singleton global row
    /// - sets `status='error'`, clears lease
    /// - writes failure reason and retry time
    /// - decrements `retry_remaining` without going below zero
    pub async fn mark_global_phase2_job_failed(
        &self,
        ownership_token: &str,
        failure_reason: &str,
        retry_delay_seconds: i64,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let retry_at = now.saturating_add(retry_delay_seconds.max(0));
        let rows_affected = sqlx::query(
            r#"
UPDATE jobs
SET
    status = 'error',
    finished_at = ?,
    lease_until = NULL,
    retry_at = ?,
    retry_remaining = max(retry_remaining - 1, 0),
    last_error = ?
WHERE kind = ? AND job_key = ?
  AND status = 'running' AND ownership_token = ?
            "#,
        )
        .bind(now)
        .bind(retry_at)
        .bind(failure_reason)
        .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
        .bind(MEMORY_CONSOLIDATION_JOB_KEY)
        .bind(ownership_token)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected();

        Ok(rows_affected > 0)
    }

    /// Fallback failure finalization when ownership may have been lost.
    ///
    /// Query behavior:
    /// - same state transition as [`Self::mark_global_phase2_job_failed`]
    /// - matches rows where `ownership_token = ? OR ownership_token IS NULL`
    /// - allows recovering a stuck unowned running row
    pub async fn mark_global_phase2_job_failed_if_unowned(
        &self,
        ownership_token: &str,
        failure_reason: &str,
        retry_delay_seconds: i64,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let retry_at = now.saturating_add(retry_delay_seconds.max(0));
        let rows_affected = sqlx::query(
            r#"
UPDATE jobs
SET
    status = 'error',
    finished_at = ?,
    lease_until = NULL,
    retry_at = ?,
    retry_remaining = max(retry_remaining - 1, 0),
    last_error = ?
WHERE kind = ? AND job_key = ?
  AND status = 'running'
  AND (ownership_token = ? OR ownership_token IS NULL)
            "#,
        )
        .bind(now)
        .bind(retry_at)
        .bind(failure_reason)
        .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
        .bind(MEMORY_CONSOLIDATION_JOB_KEY)
        .bind(ownership_token)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected();

        Ok(rows_affected > 0)
    }
}

async fn mark_global_phase2_job_succeeded_row<'e, E>(
    executor: E,
    ownership_token: &str,
    completed_watermark: i64,
) -> anyhow::Result<u64>
where
    E: Executor<'e, Database = Sqlite>,
{
    let now = Utc::now().timestamp();
    let rows_affected = sqlx::query(
        r#"
UPDATE jobs
SET
    status = 'done',
    finished_at = ?,
    lease_until = NULL,
    last_error = NULL,
    last_success_watermark = max(COALESCE(last_success_watermark, 0), ?)
WHERE kind = ? AND job_key = ?
  AND status = 'running' AND ownership_token = ?
            "#,
    )
    .bind(now)
    .bind(completed_watermark)
    .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
    .bind(MEMORY_CONSOLIDATION_JOB_KEY)
    .bind(ownership_token)
    .execute(executor)
    .await?
    .rows_affected();

    Ok(rows_affected)
}

async fn enqueue_global_consolidation_with_executor<'e, E>(
    executor: E,
    input_watermark: i64,
) -> anyhow::Result<()>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        r#"
INSERT INTO jobs (
    kind,
    job_key,
    status,
    worker_id,
    ownership_token,
    started_at,
    finished_at,
    lease_until,
    retry_at,
    retry_remaining,
    last_error,
    input_watermark,
    last_success_watermark
) VALUES (?, ?, 'pending', NULL, NULL, NULL, NULL, NULL, NULL, ?, NULL, ?, 0)
ON CONFLICT(kind, job_key) DO UPDATE SET
    status = CASE
        WHEN jobs.status = 'running' THEN 'running'
        ELSE 'pending'
    END,
    retry_at = CASE
        WHEN jobs.status = 'running' THEN jobs.retry_at
        ELSE NULL
    END,
    retry_remaining = max(jobs.retry_remaining, excluded.retry_remaining),
    input_watermark = CASE
        WHEN excluded.input_watermark > COALESCE(jobs.input_watermark, 0)
            THEN excluded.input_watermark
        ELSE COALESCE(jobs.input_watermark, 0) + 1
    END
        "#,
    )
    .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
    .bind(MEMORY_CONSOLIDATION_JOB_KEY)
    .bind(DEFAULT_RETRY_REMAINING)
    .bind(input_watermark)
    .execute(executor)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests;
