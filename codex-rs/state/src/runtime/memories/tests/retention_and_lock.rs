use super::*;

#[tokio::test]
async fn prune_stage1_outputs_for_retention_prunes_stale_unselected_rows_only() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let stale_unused = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("stale unused");
    let stale_used = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("stale used");
    let stale_selected =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("stale selected");
    let fresh_used = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("fresh used");

    for (thread_id, workspace) in [
        (stale_unused, "workspace-stale-unused"),
        (stale_used, "workspace-stale-used"),
        (stale_selected, "workspace-stale-selected"),
        (fresh_used, "workspace-fresh-used"),
    ] {
        runtime
            .upsert_thread(&test_thread_metadata(
                &codex_home,
                thread_id,
                codex_home.join(workspace),
            ))
            .await
            .expect("upsert thread");
    }

    let now = Utc::now().timestamp();
    for (thread_id, source_updated_at, summary) in [
        (
            stale_unused,
            now - Duration::days(60).num_seconds(),
            "stale-unused",
        ),
        (
            stale_used,
            now - Duration::days(50).num_seconds(),
            "stale-used",
        ),
        (
            stale_selected,
            now - Duration::days(45).num_seconds(),
            "stale-selected",
        ),
        (
            fresh_used,
            now - Duration::days(10).num_seconds(),
            "fresh-used",
        ),
    ] {
        let claim = runtime
            .try_claim_stage1_job(
                thread_id,
                owner,
                source_updated_at,
                /*lease_seconds*/ 3600,
                /*max_running_jobs*/ 64,
            )
            .await
            .expect("claim stage1");
        let ownership_token = match claim {
            Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
            other => panic!("unexpected stage1 claim outcome: {other:?}"),
        };
        assert!(
            runtime
                .mark_stage1_job_succeeded(
                    thread_id,
                    ownership_token.as_str(),
                    source_updated_at,
                    &format!("raw-{summary}"),
                    summary,
                    /*rollout_slug*/ None,
                )
                .await
                .expect("mark stage1 success"),
            "stage1 success should persist output"
        );
    }

    sqlx::query("UPDATE stage1_outputs SET usage_count = ?, last_usage = ? WHERE thread_id = ?")
        .bind(3_i64)
        .bind(now - Duration::days(40).num_seconds())
        .bind(stale_used.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("set stale used metadata");
    sqlx::query(
            "UPDATE stage1_outputs SET selected_for_phase2 = 1, selected_for_phase2_source_updated_at = source_updated_at WHERE thread_id = ?",
        )
        .bind(stale_selected.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("mark selected for phase2");
    sqlx::query("UPDATE stage1_outputs SET usage_count = ?, last_usage = ? WHERE thread_id = ?")
        .bind(8_i64)
        .bind(now - Duration::days(2).num_seconds())
        .bind(fresh_used.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("set fresh used metadata");

    let before_jobs_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE kind = 'memory_stage1'")
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("count stage1 jobs before prune");

    let pruned = runtime
        .prune_stage1_outputs_for_retention(/*max_unused_days*/ 30, /*limit*/ 100)
        .await
        .expect("prune stage1 outputs");
    assert_eq!(pruned, 2);

    let remaining =
        sqlx::query_scalar::<_, String>("SELECT thread_id FROM stage1_outputs ORDER BY thread_id")
            .fetch_all(runtime.pool.as_ref())
            .await
            .expect("load remaining stage1 outputs");
    let mut expected_remaining = vec![fresh_used.to_string(), stale_selected.to_string()];
    expected_remaining.sort();
    assert_eq!(remaining, expected_remaining);

    let after_jobs_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE kind = 'memory_stage1'")
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("count stage1 jobs after prune");
    assert_eq!(after_jobs_count, before_jobs_count);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn prune_stage1_outputs_for_retention_respects_batch_limit() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let thread_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread a");
    let thread_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread b");
    let thread_c = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread c");

    for (thread_id, workspace) in [
        (thread_a, "workspace-a"),
        (thread_b, "workspace-b"),
        (thread_c, "workspace-c"),
    ] {
        runtime
            .upsert_thread(&test_thread_metadata(
                &codex_home,
                thread_id,
                codex_home.join(workspace),
            ))
            .await
            .expect("upsert thread");
    }

    let now = Utc::now().timestamp();
    for (thread_id, source_updated_at, summary) in [
        (thread_a, now - Duration::days(60).num_seconds(), "stale-a"),
        (thread_b, now - Duration::days(50).num_seconds(), "stale-b"),
        (thread_c, now - Duration::days(40).num_seconds(), "stale-c"),
    ] {
        let claim = runtime
            .try_claim_stage1_job(
                thread_id,
                owner,
                source_updated_at,
                /*lease_seconds*/ 3600,
                /*max_running_jobs*/ 64,
            )
            .await
            .expect("claim stage1");
        let ownership_token = match claim {
            Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
            other => panic!("unexpected stage1 claim outcome: {other:?}"),
        };
        assert!(
            runtime
                .mark_stage1_job_succeeded(
                    thread_id,
                    ownership_token.as_str(),
                    source_updated_at,
                    &format!("raw-{summary}"),
                    summary,
                    /*rollout_slug*/ None,
                )
                .await
                .expect("mark stage1 success"),
            "stage1 success should persist output"
        );
    }

    let pruned = runtime
        .prune_stage1_outputs_for_retention(/*max_unused_days*/ 30, /*limit*/ 2)
        .await
        .expect("prune stage1 outputs with limit");
    assert_eq!(pruned, 2);

    let remaining_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stage1_outputs")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("count remaining stage1 outputs");
    assert_eq!(remaining_count, 1);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn mark_stage1_job_succeeded_enqueues_global_consolidation() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id a");
    let thread_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id b");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");

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

    let claim_a = runtime
        .try_claim_stage1_job(
            thread_a, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 a");
    let token_a = match claim_a {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage1 claim outcome for thread a: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_a,
                token_a.as_str(),
                /*source_updated_at*/ 100,
                "raw-a",
                "summary-a",
                /*rollout_slug*/ None,
            )
            .await
            .expect("mark stage1 succeeded a"),
        "stage1 success should persist output for thread a"
    );

    let claim_b = runtime
        .try_claim_stage1_job(
            thread_b, owner, /*source_updated_at*/ 101, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 b");
    let token_b = match claim_b {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage1 claim outcome for thread b: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_b,
                token_b.as_str(),
                /*source_updated_at*/ 101,
                "raw-b",
                "summary-b",
                /*rollout_slug*/ None,
            )
            .await
            .expect("mark stage1 succeeded b"),
        "stage1 success should persist output for thread b"
    );

    let claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim global consolidation");
    let input_watermark = match claim {
        Phase2JobClaimOutcome::Claimed {
            input_watermark, ..
        } => input_watermark,
        other => panic!("unexpected global consolidation claim outcome: {other:?}"),
    };
    assert_eq!(input_watermark, 101);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn phase2_global_lock_allows_only_one_fresh_runner() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    runtime
        .enqueue_global_consolidation(/*input_watermark*/ 200)
        .await
        .expect("enqueue global consolidation");

    let owner_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner a");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner b");

    let running_claim = runtime
        .try_claim_global_phase2_job(owner_a, /*lease_seconds*/ 3600)
        .await
        .expect("claim global lock");
    assert!(
        matches!(running_claim, Phase2JobClaimOutcome::Claimed { .. }),
        "first owner should claim global lock"
    );

    let second_claim = runtime
        .try_claim_global_phase2_job(owner_b, /*lease_seconds*/ 3600)
        .await
        .expect("claim global lock from second owner");
    assert_eq!(second_claim, Phase2JobClaimOutcome::SkippedRunning);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn phase2_global_lock_creates_missing_job_row() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let owner_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner a");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner b");

    let claim = runtime
        .try_claim_global_phase2_job(owner_a, /*lease_seconds*/ 3_600)
        .await
        .expect("claim global phase2 lock");
    let ownership_token = match claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => {
            assert_eq!(input_watermark, 0);
            ownership_token
        }
        other => panic!("unexpected phase2 lock claim outcome: {other:?}"),
    };

    let second_claim = runtime
        .try_claim_global_phase2_job(owner_b, /*lease_seconds*/ 3_600)
        .await
        .expect("claim global phase2 lock from second owner");
    assert_eq!(second_claim, Phase2JobClaimOutcome::SkippedRunning);

    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                ownership_token.as_str(),
                /*completed_watermark*/ 0,
                &[]
            )
            .await
            .expect("mark phase2 lock success")
    );
    let claim_after_success = runtime
        .try_claim_global_phase2_job(owner_b, /*lease_seconds*/ 3_600)
        .await
        .expect("claim global phase2 lock after success");
    assert_eq!(claim_after_success, Phase2JobClaimOutcome::SkippedCooldown);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn phase2_global_lock_stale_lease_allows_takeover() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    runtime
        .enqueue_global_consolidation(/*input_watermark*/ 300)
        .await
        .expect("enqueue global consolidation");

    let owner_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner a");
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner b");

    let initial_claim = runtime
        .try_claim_global_phase2_job(owner_a, /*lease_seconds*/ 3600)
        .await
        .expect("claim initial global lock");
    let token_a = match initial_claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token, ..
        } => ownership_token,
        other => panic!("unexpected initial claim outcome: {other:?}"),
    };

    sqlx::query("UPDATE jobs SET lease_until = ? WHERE kind = ? AND job_key = ?")
        .bind(Utc::now().timestamp() - 1)
        .bind("memory_consolidate_global")
        .bind("global")
        .execute(runtime.pool.as_ref())
        .await
        .expect("expire global consolidation lease");

    let takeover_claim = runtime
        .try_claim_global_phase2_job(owner_b, /*lease_seconds*/ 3600)
        .await
        .expect("claim stale global lock");
    let (token_b, input_watermark) = match takeover_claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected takeover claim outcome: {other:?}"),
    };
    assert_ne!(token_a, token_b);
    assert_eq!(input_watermark, 300);

    assert_eq!(
        runtime
            .mark_global_phase2_job_succeeded(
                token_a.as_str(),
                /*completed_watermark*/ 300,
                &[]
            )
            .await
            .expect("mark stale owner success result"),
        false,
        "stale owner should lose finalization ownership after takeover"
    );
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                token_b.as_str(),
                /*completed_watermark*/ 300,
                &[]
            )
            .await
            .expect("mark takeover owner success"),
        "takeover owner should finalize consolidation"
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn enqueue_global_consolidation_keeps_phase2_input_watermark_monotonic() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    runtime
        .enqueue_global_consolidation(/*input_watermark*/ 500)
        .await
        .expect("enqueue initial consolidation");
    let owner_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner a");
    let claim_a = runtime
        .try_claim_global_phase2_job(owner_a, /*lease_seconds*/ 3_600)
        .await
        .expect("claim initial consolidation");
    let token_a = match claim_a {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => {
            assert_eq!(input_watermark, 500);
            ownership_token
        }
        other => panic!("unexpected initial phase2 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                token_a.as_str(),
                /*completed_watermark*/ 500,
                &[]
            )
            .await
            .expect("mark initial phase2 success"),
        "initial phase2 success should finalize"
    );

    runtime
        .enqueue_global_consolidation(/*input_watermark*/ 400)
        .await
        .expect("enqueue lower-watermark consolidation");

    age_phase2_success_beyond_cooldown(&runtime).await;
    let owner_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner b");
    let claim_b = runtime
        .try_claim_global_phase2_job(owner_b, /*lease_seconds*/ 3_600)
        .await
        .expect("claim lower-watermark consolidation");
    match claim_b {
        Phase2JobClaimOutcome::Claimed {
            input_watermark, ..
        } => {
            assert!(
                input_watermark > 500,
                "lower-watermark enqueue should still advance the bookkeeping watermark"
            );
        }
        other => panic!("unexpected lower-watermark phase2 claim outcome: {other:?}"),
    }

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn phase2_failure_fallback_updates_unowned_running_job() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    runtime
        .enqueue_global_consolidation(/*input_watermark*/ 400)
        .await
        .expect("enqueue global consolidation");

    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner");
    let claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3_600)
        .await
        .expect("claim global consolidation");
    let ownership_token = match claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token, ..
        } => ownership_token,
        other => panic!("unexpected claim outcome: {other:?}"),
    };

    sqlx::query("UPDATE jobs SET ownership_token = NULL WHERE kind = ? AND job_key = ?")
        .bind("memory_consolidate_global")
        .bind("global")
        .execute(runtime.pool.as_ref())
        .await
        .expect("clear ownership token");

    assert_eq!(
        runtime
            .mark_global_phase2_job_failed(
                ownership_token.as_str(),
                "lost",
                /*retry_delay_seconds*/ 3_600
            )
            .await
            .expect("mark phase2 failed with strict ownership"),
        false,
        "strict failure update should not match unowned running job"
    );
    assert!(
        runtime
            .mark_global_phase2_job_failed_if_unowned(
                ownership_token.as_str(),
                "lost",
                /*retry_delay_seconds*/ 3_600
            )
            .await
            .expect("fallback failure update should match unowned running job"),
        "fallback failure update should transition the unowned running job"
    );

    let claim = runtime
        .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
        .await
        .expect("claim after fallback failure");
    assert_eq!(claim, Phase2JobClaimOutcome::SkippedRetryUnavailable);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}
