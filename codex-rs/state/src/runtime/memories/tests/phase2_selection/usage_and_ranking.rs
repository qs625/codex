use super::*;

#[tokio::test]
async fn get_phase2_input_selection_uses_current_ranking_after_refreshes() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id_a = stable_thread_id("00000000-0000-4000-8000-000000000001");
    let thread_id_b = stable_thread_id("00000000-0000-4000-8000-000000000002");
    let thread_id_c = stable_thread_id("00000000-0000-4000-8000-000000000003");
    let thread_id_d = stable_thread_id("00000000-0000-4000-8000-000000000004");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");

    for (thread_id, workspace) in [
        (thread_id_a, "workspace-a"),
        (thread_id_b, "workspace-b"),
        (thread_id_c, "workspace-c"),
        (thread_id_d, "workspace-d"),
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

    for (thread_id, updated_at, slug) in [
        (thread_id_a, 100, Some("rollout-a-100")),
        (thread_id_b, 101, Some("rollout-b-101")),
        (thread_id_c, 99, Some("rollout-c-99")),
        (thread_id_d, 98, Some("rollout-d-98")),
    ] {
        let claim = runtime
            .try_claim_stage1_job(
                thread_id, owner, updated_at, /*lease_seconds*/ 3600,
                /*max_running_jobs*/ 64,
            )
            .await
            .expect("claim initial stage1");
        let ownership_token = match claim {
            Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
            other => panic!("unexpected stage1 claim outcome: {other:?}"),
        };
        assert!(
            runtime
                .mark_stage1_job_succeeded(
                    thread_id,
                    ownership_token.as_str(),
                    updated_at,
                    &format!("raw-{updated_at}"),
                    &format!("summary-{updated_at}"),
                    slug,
                )
                .await
                .expect("mark stage1 succeeded"),
            "stage1 success should persist output"
        );
    }

    let phase2_claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2");
    let (phase2_token, input_watermark) = match phase2_claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected phase2 claim outcome: {other:?}"),
    };
    let selected_outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 2)
        .await
        .expect("list selected outputs");
    assert_eq!(
        selected_outputs
            .iter()
            .map(|output| output.thread_id)
            .collect::<Vec<_>>(),
        vec![thread_id_b, thread_id_a]
    );
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                phase2_token.as_str(),
                input_watermark,
                &selected_outputs,
            )
            .await
            .expect("mark phase2 success"),
        "phase2 success should persist selected rows"
    );

    for (thread_id, updated_at, slug) in [
        (thread_id_a, 102, Some("rollout-a-102")),
        (thread_id_c, 103, Some("rollout-c-103")),
        (thread_id_d, 104, Some("rollout-d-104")),
    ] {
        let claim = runtime
            .try_claim_stage1_job(
                thread_id, owner, updated_at, /*lease_seconds*/ 3600,
                /*max_running_jobs*/ 64,
            )
            .await
            .expect("claim refreshed stage1");
        let ownership_token = match claim {
            Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
            other => panic!("unexpected stage1 claim outcome: {other:?}"),
        };
        assert!(
            runtime
                .mark_stage1_job_succeeded(
                    thread_id,
                    ownership_token.as_str(),
                    updated_at,
                    &format!("raw-{updated_at}"),
                    &format!("summary-{updated_at}"),
                    slug,
                )
                .await
                .expect("mark refreshed stage1 success"),
            "refreshed stage1 success should persist output"
        );
    }

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 2, /*max_unused_days*/ 36_500)
        .await
        .expect("load phase2 input selection");
    assert_eq!(
        selection
            .iter()
            .map(|output| output.thread_id)
            .collect::<Vec<_>>(),
        vec![thread_id_c, thread_id_d]
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn mark_global_phase2_job_succeeded_updates_selected_snapshot_timestamp() {
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

    let initial_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim initial stage1");
    let initial_token = match initial_claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id,
                initial_token.as_str(),
                /*source_updated_at*/ 100,
                "raw-100",
                "summary-100",
                Some("rollout-100"),
            )
            .await
            .expect("mark initial stage1 success"),
        "initial stage1 success should persist output"
    );

    let first_phase2_claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim first phase2");
    let (first_phase2_token, first_input_watermark) = match first_phase2_claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected first phase2 claim outcome: {other:?}"),
    };
    let first_selected_outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 1)
        .await
        .expect("list first selected outputs");
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                first_phase2_token.as_str(),
                first_input_watermark,
                &first_selected_outputs,
            )
            .await
            .expect("mark first phase2 success"),
        "first phase2 success should persist selected rows"
    );

    let refreshed_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 101, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim refreshed stage1");
    let refreshed_token = match refreshed_claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected refreshed stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id,
                refreshed_token.as_str(),
                /*source_updated_at*/ 101,
                "raw-101",
                "summary-101",
                Some("rollout-101"),
            )
            .await
            .expect("mark refreshed stage1 success"),
        "refreshed stage1 success should persist output"
    );

    age_phase2_success_beyond_cooldown(&runtime).await;
    let second_phase2_claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim second phase2");
    let (second_phase2_token, second_input_watermark) = match second_phase2_claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected second phase2 claim outcome: {other:?}"),
    };
    let second_selected_outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 1)
        .await
        .expect("list second selected outputs");
    assert_eq!(
        second_selected_outputs[0].source_updated_at.timestamp(),
        101
    );
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                second_phase2_token.as_str(),
                second_input_watermark,
                &second_selected_outputs,
            )
            .await
            .expect("mark second phase2 success"),
        "second phase2 success should persist selected rows"
    );

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 1, /*max_unused_days*/ 36_500)
        .await
        .expect("load phase2 input selection after refresh");
    assert_eq!(selection.len(), 1);
    assert_eq!(selection[0].thread_id, thread_id);

    let (selected_for_phase2, selected_for_phase2_source_updated_at) =
            sqlx::query_as::<_, (i64, Option<i64>)>(
                "SELECT selected_for_phase2, selected_for_phase2_source_updated_at FROM stage1_outputs WHERE thread_id = ?",
            )
            .bind(thread_id.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("load selected snapshot after phase2");
    assert_eq!(selected_for_phase2, 1);
    assert_eq!(selected_for_phase2_source_updated_at, Some(101));

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn mark_global_phase2_job_succeeded_only_marks_exact_selected_snapshots() {
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

    let initial_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim initial stage1");
    let initial_token = match initial_claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id,
                initial_token.as_str(),
                /*source_updated_at*/ 100,
                "raw-100",
                "summary-100",
                Some("rollout-100"),
            )
            .await
            .expect("mark initial stage1 success"),
        "initial stage1 success should persist output"
    );

    let phase2_claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2");
    let (phase2_token, input_watermark) = match phase2_claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected phase2 claim outcome: {other:?}"),
    };
    let selected_outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 1)
        .await
        .expect("list selected outputs");
    assert_eq!(selected_outputs[0].source_updated_at.timestamp(), 100);

    let refreshed_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 101, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim refreshed stage1");
    let refreshed_token = match refreshed_claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id,
                refreshed_token.as_str(),
                /*source_updated_at*/ 101,
                "raw-101",
                "summary-101",
                Some("rollout-101"),
            )
            .await
            .expect("mark refreshed stage1 success"),
        "refreshed stage1 success should persist output"
    );

    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                phase2_token.as_str(),
                input_watermark,
                &selected_outputs,
            )
            .await
            .expect("mark phase2 success"),
        "phase2 success should still complete"
    );

    let (selected_for_phase2, selected_for_phase2_source_updated_at) =
            sqlx::query_as::<_, (i64, Option<i64>)>(
                "SELECT selected_for_phase2, selected_for_phase2_source_updated_at FROM stage1_outputs WHERE thread_id = ?",
            )
            .bind(thread_id.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("load selected_for_phase2");
    assert_eq!(selected_for_phase2, 0);
    assert_eq!(selected_for_phase2_source_updated_at, None);

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 1, /*max_unused_days*/ 36_500)
        .await
        .expect("load phase2 input selection");
    assert_eq!(selection.len(), 1);
    assert_eq!(selection[0].source_updated_at.timestamp(), 101);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn record_stage1_output_usage_updates_usage_metadata() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id a");
    let thread_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id b");
    let missing = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("missing id");
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
        other => panic!("unexpected stage1 claim outcome for a: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_a,
                token_a.as_str(),
                /*source_updated_at*/ 100,
                "raw a",
                "sum a",
                /*rollout_slug*/ None
            )
            .await
            .expect("mark stage1 succeeded a")
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
        other => panic!("unexpected stage1 claim outcome for b: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_b,
                token_b.as_str(),
                /*source_updated_at*/ 101,
                "raw b",
                "sum b",
                /*rollout_slug*/ None
            )
            .await
            .expect("mark stage1 succeeded b")
    );

    let updated_rows = runtime
        .record_stage1_output_usage(&[thread_a, thread_a, thread_b, missing])
        .await
        .expect("record stage1 output usage");
    assert_eq!(updated_rows, 3);

    let row_a =
        sqlx::query("SELECT usage_count, last_usage FROM stage1_outputs WHERE thread_id = ?")
            .bind(thread_a.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("load stage1 usage row a");
    let row_b =
        sqlx::query("SELECT usage_count, last_usage FROM stage1_outputs WHERE thread_id = ?")
            .bind(thread_b.to_string())
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("load stage1 usage row b");

    assert_eq!(
        row_a
            .try_get::<i64, _>("usage_count")
            .expect("usage_count a"),
        2
    );
    assert_eq!(
        row_b
            .try_get::<i64, _>("usage_count")
            .expect("usage_count b"),
        1
    );

    let last_usage_a = row_a.try_get::<i64, _>("last_usage").expect("last_usage a");
    let last_usage_b = row_b.try_get::<i64, _>("last_usage").expect("last_usage b");
    assert_eq!(last_usage_a, last_usage_b);
    assert!(last_usage_a > 0);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn get_phase2_input_selection_prioritizes_usage_count_then_recent_usage() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let now = Utc::now();
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let thread_a = stable_thread_id("00000000-0000-4000-8000-000000000001");
    let thread_b = stable_thread_id("00000000-0000-4000-8000-000000000002");
    let thread_c = stable_thread_id("00000000-0000-4000-8000-000000000003");

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

    for (thread_id, generated_at, summary) in [
        (thread_a, now - Duration::days(3), "summary-a"),
        (thread_b, now - Duration::days(2), "summary-b"),
        (thread_c, now - Duration::days(1), "summary-c"),
    ] {
        let source_updated_at = generated_at.timestamp();
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

    for (thread_id, usage_count, last_usage) in [
        (thread_a, 5_i64, now - Duration::days(10)),
        (thread_b, 5_i64, now - Duration::days(1)),
        (thread_c, 1_i64, now - Duration::hours(1)),
    ] {
        sqlx::query(
            "UPDATE stage1_outputs SET usage_count = ?, last_usage = ? WHERE thread_id = ?",
        )
        .bind(usage_count)
        .bind(last_usage.timestamp())
        .bind(thread_id.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("update usage metadata");
    }

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 1, /*max_unused_days*/ 30)
        .await
        .expect("load phase2 input selection");

    assert_eq!(
        selection
            .iter()
            .map(|output| output.thread_id)
            .collect::<Vec<_>>(),
        vec![thread_b]
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn get_phase2_input_selection_excludes_stale_used_memories_but_keeps_fresh_never_used() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let now = Utc::now();
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let thread_a = stable_thread_id("00000000-0000-4000-8000-000000000001");
    let thread_b = stable_thread_id("00000000-0000-4000-8000-000000000002");
    let thread_c = stable_thread_id("00000000-0000-4000-8000-000000000003");

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

    for (thread_id, generated_at, summary) in [
        (thread_a, now - Duration::days(40), "summary-a"),
        (thread_b, now - Duration::days(2), "summary-b"),
        (thread_c, now - Duration::days(50), "summary-c"),
    ] {
        let source_updated_at = generated_at.timestamp();
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

    for (thread_id, usage_count, last_usage) in [
        (thread_a, Some(9_i64), Some(now - Duration::days(31))),
        (thread_b, None, None),
        (thread_c, Some(1_i64), Some(now - Duration::days(1))),
    ] {
        sqlx::query(
            "UPDATE stage1_outputs SET usage_count = ?, last_usage = ? WHERE thread_id = ?",
        )
        .bind(usage_count)
        .bind(last_usage.map(|value| value.timestamp()))
        .bind(thread_id.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("update usage metadata");
    }

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 3, /*max_unused_days*/ 30)
        .await
        .expect("load phase2 input selection");

    assert_eq!(
        selection
            .iter()
            .map(|output| output.thread_id)
            .collect::<Vec<_>>(),
        vec![thread_b, thread_c]
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn get_phase2_input_selection_prefers_recent_thread_updates_over_recent_generation() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    let older_thread = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("older thread id");
    let newer_thread = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("newer thread id");

    for (thread_id, workspace) in [
        (older_thread, "workspace-older"),
        (newer_thread, "workspace-newer"),
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

    for (thread_id, source_updated_at, summary) in [
        (older_thread, 100_i64, "summary-older"),
        (newer_thread, 200_i64, "summary-newer"),
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

    sqlx::query("UPDATE stage1_outputs SET generated_at = ? WHERE thread_id = ?")
        .bind(300_i64)
        .bind(older_thread.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("update older generated_at");
    sqlx::query("UPDATE stage1_outputs SET generated_at = ? WHERE thread_id = ?")
        .bind(150_i64)
        .bind(newer_thread.to_string())
        .execute(runtime.pool.as_ref())
        .await
        .expect("update newer generated_at");

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 1, /*max_unused_days*/ 36_500)
        .await
        .expect("load phase2 input selection");

    assert_eq!(selection.len(), 1);
    assert_eq!(selection[0].thread_id, newer_thread);
    assert_eq!(selection[0].source_updated_at.timestamp(), 200);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

