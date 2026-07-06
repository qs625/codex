use super::*;

#[tokio::test]
async fn stage1_claim_skips_when_up_to_date() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let metadata = test_thread_metadata(&codex_home, thread_id, codex_home.join("a"));
    runtime
        .upsert_thread(&metadata)
        .await
        .expect("upsert thread");

    let owner_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");

    let claim = runtime
        .try_claim_stage1_job(
            thread_id, owner_a, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 job");
    let ownership_token = match claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected claim outcome: {other:?}"),
    };

    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id,
                ownership_token.as_str(),
                /*source_updated_at*/ 100,
                "raw",
                "sum",
                /*rollout_slug*/ None,
            )
            .await
            .expect("mark stage1 succeeded"),
        "stage1 success should finalize for current token"
    );

    let up_to_date = runtime
        .try_claim_stage1_job(
            thread_id, owner_b, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 up-to-date");
    assert_eq!(up_to_date, Stage1JobClaimOutcome::SkippedUpToDate);

    let needs_rerun = runtime
        .try_claim_stage1_job(
            thread_id, owner_b, /*source_updated_at*/ 101, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 newer source");
    assert!(
        matches!(needs_rerun, Stage1JobClaimOutcome::Claimed { .. }),
        "newer source_updated_at should be claimable"
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn stage1_running_stale_can_be_stolen_but_fresh_running_is_skipped() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let owner_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let cwd = codex_home.join("workspace");
    runtime
        .upsert_thread(&test_thread_metadata(&codex_home, thread_id, cwd))
        .await
        .expect("upsert thread");

    let claim_a = runtime
        .try_claim_stage1_job(
            thread_id, owner_a, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim a");
    assert!(matches!(claim_a, Stage1JobClaimOutcome::Claimed { .. }));

    let claim_b_fresh = runtime
        .try_claim_stage1_job(
            thread_id, owner_b, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim b fresh");
    assert_eq!(claim_b_fresh, Stage1JobClaimOutcome::SkippedRunning);

    sqlx::query("UPDATE jobs SET lease_until = 0 WHERE kind = 'memory_stage1' AND job_key = ?")
        .bind(thread_id.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("force stale lease");

    let claim_b_stale = runtime
        .try_claim_stage1_job(
            thread_id, owner_b, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim b stale");
    assert!(matches!(
        claim_b_stale,
        Stage1JobClaimOutcome::Claimed { .. }
    ));

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn stage1_concurrent_claim_for_same_thread_is_conflict_safe() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_id,
            codex_home.join("workspace"),
        ))
        .await
        .expect("upsert thread");

    let owner_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let thread_id_a = thread_id;
    let thread_id_b = thread_id;
    let runtime_a = Arc::clone(&runtime);
    let runtime_b = Arc::clone(&runtime);
    let claim_with_retry = |runtime: Arc<StateRuntime>, thread_id: ThreadId, owner: ThreadId| async move {
        for attempt in 0..5 {
            match runtime
                .try_claim_stage1_job(
                    thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3_600,
                    /*max_running_jobs*/ 64,
                )
                .await
            {
                Ok(outcome) => return outcome,
                Err(err) if err.to_string().contains("database is locked") && attempt < 4 => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(err) => panic!("claim stage1 should not fail: {err}"),
            }
        }
        panic!("claim stage1 should have returned within retry budget")
    };

    let (claim_a, claim_b) = tokio::join!(
        claim_with_retry(runtime_a, thread_id_a, owner_a),
        claim_with_retry(runtime_b, thread_id_b, owner_b),
    );

    let claim_outcomes = vec![claim_a, claim_b];
    let claimed_count = claim_outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Stage1JobClaimOutcome::Claimed { .. }))
        .count();
    assert_eq!(claimed_count, 1);
    assert!(
        claim_outcomes.iter().all(|outcome| {
            matches!(
                outcome,
                Stage1JobClaimOutcome::Claimed { .. } | Stage1JobClaimOutcome::SkippedRunning
            )
        }),
        "unexpected claim outcomes: {claim_outcomes:?}"
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn stage1_concurrent_claims_respect_running_cap() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let thread_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_a,
            codex_home.join("workspace-a"),
        ))
        .await
        .expect("upsert thread a");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_b,
            codex_home.join("workspace-b"),
        ))
        .await
        .expect("upsert thread b");

    let owner_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let runtime_a = Arc::clone(&runtime);
    let runtime_b = Arc::clone(&runtime);

    let (claim_a, claim_b) = tokio::join!(
        async move {
            runtime_a
                .try_claim_stage1_job(
                    thread_a, owner_a, /*source_updated_at*/ 100,
                    /*lease_seconds*/ 3_600, /*max_running_jobs*/ 1,
                )
                .await
                .expect("claim stage1 thread a")
        },
        async move {
            runtime_b
                .try_claim_stage1_job(
                    thread_b, owner_b, /*source_updated_at*/ 101,
                    /*lease_seconds*/ 3_600, /*max_running_jobs*/ 1,
                )
                .await
                .expect("claim stage1 thread b")
        },
    );

    let claim_outcomes = vec![claim_a, claim_b];
    let claimed_count = claim_outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Stage1JobClaimOutcome::Claimed { .. }))
        .count();
    assert_eq!(claimed_count, 1);
    assert!(
        claim_outcomes
            .iter()
            .any(|outcome| { matches!(outcome, Stage1JobClaimOutcome::SkippedRunning) }),
        "one concurrent claim should be throttled by running cap: {claim_outcomes:?}"
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn claim_stage1_jobs_filters_by_age_idle_and_current_thread() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let now = Utc::now();
    let fresh_at = now - Duration::hours(1);
    let just_under_idle_at = now - Duration::hours(12) + Duration::minutes(1);
    let eligible_idle_at = now - Duration::hours(12) - Duration::minutes(1);
    let old_at = now - Duration::days(31);

    let current_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("current thread id");
    let fresh_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("fresh thread id");
    let just_under_idle_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("just under idle thread id");
    let eligible_idle_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("eligible idle thread id");
    let old_thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("old thread id");

    let mut current =
        test_thread_metadata(&codex_home, current_thread_id, codex_home.join("current"));
    current.created_at = now;
    current.updated_at = now;
    runtime
        .upsert_thread(&current)
        .await
        .expect("upsert current");

    let mut fresh = test_thread_metadata(&codex_home, fresh_thread_id, codex_home.join("fresh"));
    fresh.created_at = fresh_at;
    fresh.updated_at = fresh_at;
    runtime.upsert_thread(&fresh).await.expect("upsert fresh");

    let mut just_under_idle = test_thread_metadata(
        &codex_home,
        just_under_idle_thread_id,
        codex_home.join("just-under-idle"),
    );
    just_under_idle.created_at = just_under_idle_at;
    just_under_idle.updated_at = just_under_idle_at;
    runtime
        .upsert_thread(&just_under_idle)
        .await
        .expect("upsert just-under-idle");

    let mut eligible_idle = test_thread_metadata(
        &codex_home,
        eligible_idle_thread_id,
        codex_home.join("eligible-idle"),
    );
    eligible_idle.created_at = eligible_idle_at;
    eligible_idle.updated_at = eligible_idle_at;
    runtime
        .upsert_thread(&eligible_idle)
        .await
        .expect("upsert eligible-idle");

    let mut old = test_thread_metadata(&codex_home, old_thread_id, codex_home.join("old"));
    old.created_at = old_at;
    old.updated_at = old_at;
    runtime.upsert_thread(&old).await.expect("upsert old");

    let allowed_sources = vec!["cli".to_string()];
    let claims = runtime
        .claim_stage1_jobs_for_startup(
            current_thread_id,
            Stage1StartupClaimParams {
                scan_limit: 1,
                max_claimed: 5,
                max_age_days: 30,
                min_rollout_idle_hours: 12,
                allowed_sources: allowed_sources.as_slice(),
                lease_seconds: 3600,
            },
        )
        .await
        .expect("claim stage1 jobs");

    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].thread.id, eligible_idle_thread_id);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn claim_stage1_jobs_prefilters_threads_with_up_to_date_memory() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let now = Utc::now();
    let eligible_newer_at = now - Duration::hours(13);
    let eligible_older_at = now - Duration::hours(14);

    let current_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("current thread id");
    let up_to_date_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("up-to-date thread id");
    let stale_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("stale thread id");
    let worker_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("worker id");

    let mut current =
        test_thread_metadata(&codex_home, current_thread_id, codex_home.join("current"));
    current.created_at = now;
    current.updated_at = now;
    runtime
        .upsert_thread(&current)
        .await
        .expect("upsert current thread");

    let mut up_to_date = test_thread_metadata(
        &codex_home,
        up_to_date_thread_id,
        codex_home.join("up-to-date"),
    );
    up_to_date.created_at = eligible_newer_at;
    up_to_date.updated_at = eligible_newer_at;
    runtime
        .upsert_thread(&up_to_date)
        .await
        .expect("upsert up-to-date thread");

    let up_to_date_claim = runtime
        .try_claim_stage1_job(
            up_to_date_thread_id,
            worker_id,
            up_to_date.updated_at.timestamp(),
            /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim up-to-date thread for seed");
    let up_to_date_token = match up_to_date_claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected seed claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                up_to_date_thread_id,
                up_to_date_token.as_str(),
                up_to_date.updated_at.timestamp(),
                "raw",
                "summary",
                /*rollout_slug*/ None,
            )
            .await
            .expect("mark up-to-date thread succeeded"),
        "seed stage1 success should complete for up-to-date thread"
    );

    let mut stale = test_thread_metadata(&codex_home, stale_thread_id, codex_home.join("stale"));
    stale.created_at = eligible_older_at;
    stale.updated_at = eligible_older_at;
    runtime
        .upsert_thread(&stale)
        .await
        .expect("upsert stale thread");

    let allowed_sources = vec!["cli".to_string()];
    let claims = runtime
        .claim_stage1_jobs_for_startup(
            current_thread_id,
            Stage1StartupClaimParams {
                scan_limit: 1,
                max_claimed: 1,
                max_age_days: 30,
                min_rollout_idle_hours: 12,
                allowed_sources: allowed_sources.as_slice(),
                lease_seconds: 3600,
            },
        )
        .await
        .expect("claim stage1 startup jobs");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].thread.id, stale_thread_id);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn claim_stage1_jobs_skips_threads_with_disabled_memory_mode() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let now = Utc::now();
    let eligible_at = now - Duration::hours(13);

    let current_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("current thread id");
    let disabled_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("disabled thread id");
    let enabled_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("enabled thread id");

    let mut current =
        test_thread_metadata(&codex_home, current_thread_id, codex_home.join("current"));
    current.created_at = now;
    current.updated_at = now;
    runtime
        .upsert_thread(&current)
        .await
        .expect("upsert current thread");

    let mut disabled =
        test_thread_metadata(&codex_home, disabled_thread_id, codex_home.join("disabled"));
    disabled.created_at = eligible_at;
    disabled.updated_at = eligible_at;
    runtime
        .upsert_thread(&disabled)
        .await
        .expect("upsert disabled thread");
    sqlx::query("UPDATE threads SET memory_mode = 'disabled' WHERE id = ?")
        .bind(disabled_thread_id.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("disable thread memory mode");

    let mut enabled =
        test_thread_metadata(&codex_home, enabled_thread_id, codex_home.join("enabled"));
    enabled.created_at = eligible_at;
    enabled.updated_at = eligible_at;
    runtime
        .upsert_thread(&enabled)
        .await
        .expect("upsert enabled thread");

    let allowed_sources = vec!["cli".to_string()];
    let claims = runtime
        .claim_stage1_jobs_for_startup(
            current_thread_id,
            Stage1StartupClaimParams {
                scan_limit: 10,
                max_claimed: 10,
                max_age_days: 30,
                min_rollout_idle_hours: 12,
                allowed_sources: allowed_sources.as_slice(),
                lease_seconds: 3600,
            },
        )
        .await
        .expect("claim stage1 startup jobs");

    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].thread.id, enabled_thread_id);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn clear_memory_data_clears_rows_and_preserves_thread_memory_modes() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let now = Utc::now() - Duration::hours(13);
    let worker_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("worker id");
    let enabled_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("enabled thread id");
    let disabled_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("disabled thread id");

    let mut enabled =
        test_thread_metadata(&codex_home, enabled_thread_id, codex_home.join("enabled"));
    enabled.created_at = now;
    enabled.updated_at = now;
    runtime
        .upsert_thread(&enabled)
        .await
        .expect("upsert enabled thread");

    let claim = runtime
        .try_claim_stage1_job(
            enabled_thread_id,
            worker_id,
            enabled.updated_at.timestamp(),
            /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim enabled thread");
    let ownership_token = match claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                enabled_thread_id,
                ownership_token.as_str(),
                enabled.updated_at.timestamp(),
                "raw",
                "summary",
                /*rollout_slug*/ None,
            )
            .await
            .expect("mark enabled thread succeeded"),
        "stage1 success should be recorded"
    );
    runtime
        .enqueue_global_consolidation(enabled.updated_at.timestamp())
        .await
        .expect("enqueue global consolidation");

    let mut disabled =
        test_thread_metadata(&codex_home, disabled_thread_id, codex_home.join("disabled"));
    disabled.created_at = now;
    disabled.updated_at = now;
    runtime
        .upsert_thread(&disabled)
        .await
        .expect("upsert disabled thread");
    sqlx::query("UPDATE threads SET memory_mode = 'disabled' WHERE id = ?")
        .bind(disabled_thread_id.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("disable existing thread");

    runtime
        .clear_memory_data()
        .await
        .expect("clear memory data");

    let stage1_outputs_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stage1_outputs")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("count stage1 outputs");
    assert_eq!(stage1_outputs_count, 0);

    let memory_jobs_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE kind = ? OR kind = ?")
            .bind(JOB_KIND_MEMORY_STAGE1)
            .bind(JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL)
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("count memory jobs");
    assert_eq!(memory_jobs_count, 0);

    let enabled_memory_mode: String =
        sqlx::query_scalar("SELECT memory_mode FROM threads WHERE id = ?")
            .bind(enabled_thread_id.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("read enabled thread memory mode");
    assert_eq!(enabled_memory_mode, "enabled");

    let disabled_memory_mode: String =
        sqlx::query_scalar("SELECT memory_mode FROM threads WHERE id = ?")
            .bind(disabled_thread_id.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("read disabled thread memory mode");
    assert_eq!(disabled_memory_mode, "disabled");

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn claim_stage1_jobs_enforces_global_running_cap() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let current_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("current thread id");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            current_thread_id,
            codex_home.join("current"),
        ))
        .await
        .expect("upsert current");

    let now = Utc::now();
    let started_at = now.timestamp();
    let lease_until = started_at + 3600;
    let eligible_at = now - Duration::hours(13);
    let existing_running = 10usize;
    let total_candidates = 80usize;

    for idx in 0..total_candidates {
        let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
        let mut metadata = test_thread_metadata(
            &codex_home,
            thread_id,
            codex_home.join(format!("thread-{idx}")),
        );
        metadata.created_at = eligible_at - Duration::seconds(idx as i64);
        metadata.updated_at = eligible_at - Duration::seconds(idx as i64);
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("upsert thread");

        if idx < existing_running {
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
) VALUES (?, ?, 'running', ?, ?, ?, NULL, ?, NULL, ?, NULL, ?, NULL)
                    "#,
            )
            .bind("memory_stage1")
            .bind(thread_id.to_string())
            .bind(current_thread_id.to_string())
            .bind(Uuid::new_v4().to_string())
            .bind(started_at)
            .bind(lease_until)
            .bind(3)
            .bind(metadata.updated_at.timestamp())
            .execute(runtime.pool.as_ref())
            .await
            .expect("seed running stage1 job");
        }
    }

    let allowed_sources = vec!["cli".to_string()];
    let claims = runtime
        .claim_stage1_jobs_for_startup(
            current_thread_id,
            Stage1StartupClaimParams {
                scan_limit: 200,
                max_claimed: 64,
                max_age_days: 30,
                min_rollout_idle_hours: 12,
                allowed_sources: allowed_sources.as_slice(),
                lease_seconds: 3600,
            },
        )
        .await
        .expect("claim stage1 jobs");
    assert_eq!(claims.len(), 54);

    let running_count = sqlx::query(
        r#"
SELECT COUNT(*) AS count
FROM jobs
WHERE kind = 'memory_stage1'
  AND status = 'running'
  AND lease_until IS NOT NULL
  AND lease_until > ?
            "#,
    )
    .bind(Utc::now().timestamp())
    .fetch_one(runtime.pool.as_ref())
    .await
    .expect("count running stage1 jobs")
    .try_get::<i64, _>("count")
    .expect("running count value");
    assert_eq!(running_count, 64);

    let more_claims = runtime
        .claim_stage1_jobs_for_startup(
            current_thread_id,
            Stage1StartupClaimParams {
                scan_limit: 200,
                max_claimed: 64,
                max_age_days: 30,
                min_rollout_idle_hours: 12,
                allowed_sources: allowed_sources.as_slice(),
                lease_seconds: 3600,
            },
        )
        .await
        .expect("claim stage1 jobs with cap reached");
    assert_eq!(more_claims.len(), 0);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn claim_stage1_jobs_processes_two_full_batches_across_startup_passes() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let current_thread_id =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("current thread id");
    let mut current =
        test_thread_metadata(&codex_home, current_thread_id, codex_home.join("current"));
    current.created_at = Utc::now();
    current.updated_at = Utc::now();
    runtime
        .upsert_thread(&current)
        .await
        .expect("upsert current");

    let eligible_at = Utc::now() - Duration::hours(13);
    for idx in 0..200 {
        let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
        let mut metadata = test_thread_metadata(
            &codex_home,
            thread_id,
            codex_home.join(format!("thread-{idx}")),
        );
        metadata.created_at = eligible_at - Duration::seconds(idx as i64);
        metadata.updated_at = eligible_at - Duration::seconds(idx as i64);
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("upsert eligible thread");
    }

    let allowed_sources = vec!["cli".to_string()];
    let first_claims = runtime
        .claim_stage1_jobs_for_startup(
            current_thread_id,
            Stage1StartupClaimParams {
                scan_limit: 5_000,
                max_claimed: 64,
                max_age_days: 30,
                min_rollout_idle_hours: 12,
                allowed_sources: allowed_sources.as_slice(),
                lease_seconds: 3_600,
            },
        )
        .await
        .expect("first stage1 startup claim");
    assert_eq!(first_claims.len(), 64);

    for claim in first_claims {
        assert!(
            runtime
                .mark_stage1_job_succeeded(
                    claim.thread.id,
                    claim.ownership_token.as_str(),
                    claim.thread.updated_at.timestamp(),
                    "raw",
                    "summary",
                    /*rollout_slug*/ None,
                )
                .await
                .expect("mark first-batch stage1 success"),
            "first batch stage1 completion should succeed"
        );
    }

    let second_claims = runtime
        .claim_stage1_jobs_for_startup(
            current_thread_id,
            Stage1StartupClaimParams {
                scan_limit: 5_000,
                max_claimed: 64,
                max_age_days: 30,
                min_rollout_idle_hours: 12,
                allowed_sources: allowed_sources.as_slice(),
                lease_seconds: 3_600,
            },
        )
        .await
        .expect("second stage1 startup claim");
    assert_eq!(second_claims.len(), 64);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn stage1_output_cascades_on_thread_delete() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let cwd = codex_home.join("workspace");
    runtime
        .upsert_thread(&test_thread_metadata(&codex_home, thread_id, cwd))
        .await
        .expect("upsert thread");

    let claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1");
    let ownership_token = match claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id,
                ownership_token.as_str(),
                /*source_updated_at*/ 100,
                "raw",
                "sum",
                /*rollout_slug*/ None,
            )
            .await
            .expect("mark stage1 succeeded"),
        "mark stage1 succeeded should write stage1_outputs"
    );

    let count_before =
        sqlx::query("SELECT COUNT(*) AS count FROM stage1_outputs WHERE thread_id = ?")
            .bind(thread_id.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("count before delete")
            .try_get::<i64, _>("count")
            .expect("count value");
    assert_eq!(count_before, 1);

    sqlx::query("DELETE FROM threads WHERE id = ?")
        .bind(thread_id.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("delete thread");

    let count_after =
        sqlx::query("SELECT COUNT(*) AS count FROM stage1_outputs WHERE thread_id = ?")
            .bind(thread_id.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("count after delete")
            .try_get::<i64, _>("count")
            .expect("count value");
    assert_eq!(count_after, 0);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn mark_stage1_job_succeeded_no_output_skips_phase2_when_output_was_already_absent() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_id,
            codex_home.join("workspace"),
        ))
        .await
        .expect("upsert thread");

    let claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1");
    let ownership_token = match claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded_no_output(thread_id, ownership_token.as_str())
            .await
            .expect("mark stage1 succeeded without output"),
        "stage1 no-output success should complete the job"
    );

    let output_row_count =
        sqlx::query("SELECT COUNT(*) AS count FROM stage1_outputs WHERE thread_id = ?")
            .bind(thread_id.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("load stage1 output count")
            .try_get::<i64, _>("count")
            .expect("stage1 output count");
    assert_eq!(
        output_row_count, 0,
        "stage1 no-output success should not persist empty stage1 outputs"
    );

    let up_to_date = runtime
        .try_claim_stage1_job(
            thread_id, owner_b, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 up-to-date");
    assert_eq!(up_to_date, Stage1JobClaimOutcome::SkippedUpToDate);

    let global_job_row_count = sqlx::query("SELECT COUNT(*) AS count FROM jobs WHERE kind = ?")
        .bind("memory_consolidate_global")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("load phase2 job row count")
        .try_get::<i64, _>("count")
        .expect("phase2 job row count");
    assert_eq!(
        global_job_row_count, 0,
        "no-output without an existing stage1 output should not enqueue phase2"
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn mark_stage1_job_succeeded_no_output_enqueues_phase2_when_deleting_output() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_id,
            codex_home.join("workspace"),
        ))
        .await
        .expect("upsert thread");

    let first_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim initial stage1");
    let first_token = match first_claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected initial stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id,
                first_token.as_str(),
                /*source_updated_at*/ 100,
                "raw",
                "sum",
                /*rollout_slug*/ None
            )
            .await
            .expect("mark initial stage1 succeeded"),
        "initial stage1 success should create stage1 output"
    );

    let phase2_claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2 after initial output");
    let (phase2_token, phase2_input_watermark) = match phase2_claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected phase2 claim after initial output: {other:?}"),
    };
    assert_eq!(phase2_input_watermark, 100);
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(phase2_token.as_str(), phase2_input_watermark, &[],)
            .await
            .expect("mark initial phase2 succeeded"),
        "initial phase2 success should finalize the global job"
    );

    let no_output_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner_b, /*source_updated_at*/ 101, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 for no-output delete");
    let no_output_token = match no_output_claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected no-output stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded_no_output(thread_id, no_output_token.as_str())
            .await
            .expect("mark stage1 no-output after existing output"),
        "no-output should succeed when deleting an existing stage1 output"
    );

    let output_row_count =
        sqlx::query("SELECT COUNT(*) AS count FROM stage1_outputs WHERE thread_id = ?")
            .bind(thread_id.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("load stage1 output count after delete")
            .try_get::<i64, _>("count")
            .expect("stage1 output count");
    assert_eq!(output_row_count, 0);

    age_phase2_success_beyond_cooldown(&runtime).await;
    let claim_phase2 = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2 after no-output deletion");
    let (phase2_token, phase2_input_watermark) = match claim_phase2 {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected phase2 claim after no-output deletion: {other:?}"),
    };
    assert_eq!(phase2_input_watermark, 101);
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(phase2_token.as_str(), phase2_input_watermark, &[],)
            .await
            .expect("mark phase2 succeeded after no-output delete")
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn stage1_retry_exhaustion_does_not_block_newer_watermark() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_id,
            codex_home.join("workspace"),
        ))
        .await
        .expect("upsert thread");

    for attempt in 0..3 {
        let claim = runtime
            .try_claim_stage1_job(
                thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3_600,
                /*max_running_jobs*/ 64,
            )
            .await
            .expect("claim stage1 for retry exhaustion");
        let ownership_token = match claim {
            Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
            other => panic!(
                "attempt {} should claim stage1 before retries are exhausted: {other:?}",
                attempt + 1
            ),
        };
        assert!(
            runtime
                .mark_stage1_job_failed(
                    thread_id,
                    ownership_token.as_str(),
                    "boom",
                    /*retry_delay_seconds*/ 0
                )
                .await
                .expect("mark stage1 failed"),
            "attempt {} should decrement retry budget",
            attempt + 1
        );
    }

    let exhausted_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3_600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 after retry exhaustion");
    assert_eq!(
        exhausted_claim,
        Stage1JobClaimOutcome::SkippedRetryExhausted
    );

    let newer_source_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 101, /*lease_seconds*/ 3_600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 with newer source watermark");
    assert!(
        matches!(newer_source_claim, Stage1JobClaimOutcome::Claimed { .. }),
        "newer source watermark should reset retry budget and be claimable"
    );

    let job_row = sqlx::query(
        "SELECT retry_remaining, input_watermark FROM jobs WHERE kind = ? AND job_key = ?",
    )
    .bind("memory_stage1")
    .bind(thread_id.to_string())
    .fetch_one(runtime.pool.as_ref())
    .await
    .expect("load stage1 job row after newer-source claim");
    assert_eq!(
        job_row
            .try_get::<i64, _>("retry_remaining")
            .expect("retry_remaining"),
        3
    );
    assert_eq!(
        job_row
            .try_get::<i64, _>("input_watermark")
            .expect("input_watermark"),
        101
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

