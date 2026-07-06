use super::*;

use super::*;

#[tokio::test]
async fn phase2_global_lock_respects_success_cooldown() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");

    runtime
        .enqueue_global_consolidation(/*input_watermark*/ 100)
        .await
        .expect("enqueue global consolidation");

    let claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2");
    let (ownership_token, input_watermark) = match claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected phase2 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(ownership_token.as_str(), input_watermark, &[],)
            .await
            .expect("mark phase2 succeeded"),
        "phase2 success should finalize for current token"
    );

    let claim_after_success = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2 after success");
    assert_eq!(claim_after_success, Phase2JobClaimOutcome::SkippedCooldown);

    runtime
        .enqueue_global_consolidation(/*input_watermark*/ 101)
        .await
        .expect("enqueue global consolidation after success");
    let claim_after_enqueue = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2 after enqueue");
    assert_eq!(claim_after_enqueue, Phase2JobClaimOutcome::SkippedCooldown);

    age_phase2_success_beyond_cooldown(&runtime).await;
    let claim_after_cooldown = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2 after cooldown");
    assert!(matches!(
        claim_after_cooldown,
        Phase2JobClaimOutcome::Claimed { .. }
    ));

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn phase2_global_lock_can_be_claimed_after_retry_budget_is_exhausted() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    runtime
        .enqueue_global_consolidation(/*input_watermark*/ 100)
        .await
        .expect("enqueue global consolidation");

    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    for attempt in 0..3 {
        let claim = runtime
            .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3_600)
            .await
            .expect("claim phase2 before retry exhaustion");
        let ownership_token = match claim {
            Phase2JobClaimOutcome::Claimed {
                ownership_token, ..
            } => ownership_token,
            other => panic!(
                "attempt {} should claim phase2 before retries are exhausted: {other:?}",
                attempt + 1
            ),
        };
        assert!(
            runtime
                .mark_global_phase2_job_failed(
                    ownership_token.as_str(),
                    "boom",
                    /*retry_delay_seconds*/ 0,
                )
                .await
                .expect("mark phase2 failed"),
            "attempt {} should decrement retry budget",
            attempt + 1
        );
    }

    let job_row = sqlx::query("SELECT retry_remaining FROM jobs WHERE kind = ? AND job_key = ?")
        .bind("memory_consolidate_global")
        .bind("global")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("load phase2 job row after retry exhaustion");
    assert_eq!(
        job_row
            .try_get::<i64, _>("retry_remaining")
            .expect("retry_remaining"),
        0
    );

    let claim_after_exhaustion = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3_600)
        .await
        .expect("claim phase2 after retry exhaustion");
    assert!(
        matches!(
            claim_after_exhaustion,
            Phase2JobClaimOutcome::Claimed { .. }
        ),
        "phase2 claim should only lock; workspace diffing decides whether there is work"
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn list_stage1_outputs_for_global_returns_latest_outputs() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id_a = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let thread_id_b = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_id_a,
            codex_home.join("workspace-a"),
        ))
        .await
        .expect("upsert thread a");
    let mut metadata_b =
        test_thread_metadata(&codex_home, thread_id_b, codex_home.join("workspace-b"));
    metadata_b.git_branch = Some("feature/stage1-b".to_string());
    runtime
        .upsert_thread(&metadata_b)
        .await
        .expect("upsert thread b");

    let claim = runtime
        .try_claim_stage1_job(
            thread_id_a,
            owner,
            /*source_updated_at*/ 100,
            /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 a");
    let ownership_token = match claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id_a,
                ownership_token.as_str(),
                /*source_updated_at*/ 100,
                "raw memory a",
                "summary a",
                /*rollout_slug*/ None,
            )
            .await
            .expect("mark stage1 succeeded a"),
        "stage1 success should persist output a"
    );

    let claim = runtime
        .try_claim_stage1_job(
            thread_id_b,
            owner,
            /*source_updated_at*/ 101,
            /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim stage1 b");
    let ownership_token = match claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id_b,
                ownership_token.as_str(),
                /*source_updated_at*/ 101,
                "raw memory b",
                "summary b",
                Some("rollout-b"),
            )
            .await
            .expect("mark stage1 succeeded b"),
        "stage1 success should persist output b"
    );

    let outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 10)
        .await
        .expect("list stage1 outputs for global");
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].thread_id, thread_id_b);
    assert_eq!(outputs[0].rollout_summary, "summary b");
    assert_eq!(outputs[0].rollout_slug.as_deref(), Some("rollout-b"));
    assert_eq!(outputs[0].cwd, codex_home.join("workspace-b"));
    assert_eq!(outputs[0].git_branch.as_deref(), Some("feature/stage1-b"));
    assert_eq!(outputs[1].thread_id, thread_id_a);
    assert_eq!(outputs[1].rollout_summary, "summary a");
    assert_eq!(outputs[1].rollout_slug, None);
    assert_eq!(outputs[1].cwd, codex_home.join("workspace-a"));
    assert_eq!(outputs[1].git_branch, None);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn list_stage1_outputs_for_global_skips_empty_payloads() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id_non_empty =
        ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let thread_id_empty = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_id_non_empty,
            codex_home.join("workspace-non-empty"),
        ))
        .await
        .expect("upsert non-empty thread");
    runtime
        .upsert_thread(&test_thread_metadata(
            &codex_home,
            thread_id_empty,
            codex_home.join("workspace-empty"),
        ))
        .await
        .expect("upsert empty thread");

    sqlx::query(
        r#"
INSERT INTO stage1_outputs (thread_id, source_updated_at, raw_memory, rollout_summary, generated_at)
VALUES (?, ?, ?, ?, ?)
            "#,
    )
    .bind(thread_id_non_empty.to_string())
    .bind(100_i64)
    .bind("raw memory")
    .bind("summary")
    .bind(100_i64)
    .execute(runtime.pool.as_ref())
    .await
    .expect("insert non-empty stage1 output");
    sqlx::query(
        r#"
INSERT INTO stage1_outputs (thread_id, source_updated_at, raw_memory, rollout_summary, generated_at)
VALUES (?, ?, ?, ?, ?)
            "#,
    )
    .bind(thread_id_empty.to_string())
    .bind(101_i64)
    .bind("")
    .bind("")
    .bind(101_i64)
    .execute(runtime.pool.as_ref())
    .await
    .expect("insert empty stage1 output");

    let outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 1)
        .await
        .expect("list stage1 outputs for global");
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].thread_id, thread_id_non_empty);
    assert_eq!(outputs[0].rollout_summary, "summary");
    assert_eq!(outputs[0].cwd, codex_home.join("workspace-non-empty"));

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn list_stage1_outputs_for_global_skips_polluted_threads() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id_enabled = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let thread_id_polluted = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");

    for (thread_id, workspace) in [
        (thread_id_enabled, "workspace-enabled"),
        (thread_id_polluted, "workspace-polluted"),
    ] {
        runtime
            .upsert_thread(&test_thread_metadata(
                &codex_home,
                thread_id,
                codex_home.join(workspace),
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
            other => panic!("unexpected stage1 claim outcome: {other:?}"),
        };
        assert!(
            runtime
                .mark_stage1_job_succeeded(
                    thread_id,
                    ownership_token.as_str(),
                    /*source_updated_at*/ 100,
                    "raw memory",
                    "summary",
                    /*rollout_slug*/ None,
                )
                .await
                .expect("mark stage1 succeeded"),
            "stage1 success should persist output"
        );
    }

    runtime
        .set_thread_memory_mode(thread_id_polluted, "polluted")
        .await
        .expect("mark thread polluted");

    let outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 10)
        .await
        .expect("list stage1 outputs for global");
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].thread_id, thread_id_enabled);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn get_phase2_input_selection_returns_current_selected_rows() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id_a = stable_thread_id("00000000-0000-4000-8000-000000000001");
    let thread_id_b = stable_thread_id("00000000-0000-4000-8000-000000000002");
    let thread_id_c = stable_thread_id("00000000-0000-4000-8000-000000000003");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");

    for (thread_id, workspace) in [
        (thread_id_a, "workspace-a"),
        (thread_id_b, "workspace-b"),
        (thread_id_c, "workspace-c"),
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
        (thread_id_a, 100, Some("rollout-a")),
        (thread_id_b, 101, Some("rollout-b")),
        (thread_id_c, 102, Some("rollout-c")),
    ] {
        let claim = runtime
            .try_claim_stage1_job(
                thread_id, owner, updated_at, /*lease_seconds*/ 3600,
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

    let claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2");
    let (ownership_token, input_watermark) = match claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected phase2 claim outcome: {other:?}"),
    };
    assert_eq!(input_watermark, 102);
    let selected_outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 10)
        .await
        .expect("list stage1 outputs for global")
        .into_iter()
        .filter(|output| output.thread_id == thread_id_c || output.thread_id == thread_id_a)
        .collect::<Vec<_>>();
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                ownership_token.as_str(),
                input_watermark,
                &selected_outputs,
            )
            .await
            .expect("mark phase2 success with selection"),
        "phase2 success should persist selected rows"
    );

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 2, /*max_unused_days*/ 36_500)
        .await
        .expect("load phase2 input selection");

    assert_eq!(selection.len(), 2);
    assert_eq!(
        selection
            .iter()
            .map(|output| output.thread_id)
            .collect::<Vec<_>>(),
        vec![thread_id_b, thread_id_c]
    );
    let selected_c = selection
        .iter()
        .find(|output| output.thread_id == thread_id_c)
        .expect("thread c should be selected");
    assert_eq!(
        selected_c.rollout_path,
        codex_home.join(format!("rollout-{thread_id_c}.jsonl"))
    );

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn get_phase2_input_selection_excludes_polluted_previous_selection() {
    let codex_home = unique_temp_dir();
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
        .await
        .expect("initialize runtime");

    let thread_id_enabled = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let thread_id_polluted = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("thread id");
    let owner = ThreadId::from_string(&Uuid::new_v4().to_string()).expect("owner id");

    for (thread_id, updated_at) in [(thread_id_enabled, 100), (thread_id_polluted, 101)] {
        runtime
            .upsert_thread(&test_thread_metadata(
                &codex_home,
                thread_id,
                codex_home.join(thread_id.to_string()),
            ))
            .await
            .expect("upsert thread");

        let claim = runtime
            .try_claim_stage1_job(
                thread_id, owner, updated_at, /*lease_seconds*/ 3600,
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
                    updated_at,
                    &format!("raw-{updated_at}"),
                    &format!("summary-{updated_at}"),
                    /*rollout_slug*/ None,
                )
                .await
                .expect("mark stage1 succeeded"),
            "stage1 success should persist output"
        );
    }

    let claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2");
    let (ownership_token, input_watermark) = match claim {
        Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => (ownership_token, input_watermark),
        other => panic!("unexpected phase2 claim outcome: {other:?}"),
    };
    let selected_outputs = runtime
        .list_stage1_outputs_for_global(/*n*/ 10)
        .await
        .expect("list stage1 outputs for global");
    assert!(
        runtime
            .mark_global_phase2_job_succeeded(
                ownership_token.as_str(),
                input_watermark,
                &selected_outputs,
            )
            .await
            .expect("mark phase2 success"),
        "phase2 success should persist selected rows"
    );

    runtime
        .set_thread_memory_mode(thread_id_polluted, "polluted")
        .await
        .expect("mark thread polluted");

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 2, /*max_unused_days*/ 36_500)
        .await
        .expect("load phase2 input selection");

    assert_eq!(selection.len(), 1);
    assert_eq!(selection[0].thread_id, thread_id_enabled);

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn mark_thread_memory_mode_polluted_enqueues_phase2_for_selected_threads() {
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

    let claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
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
                /*source_updated_at*/ 100,
                "raw",
                "summary",
                /*rollout_slug*/ None,
            )
            .await
            .expect("mark stage1 succeeded"),
        "stage1 success should persist output"
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
        .list_stage1_outputs_for_global(/*n*/ 10)
        .await
        .expect("list stage1 outputs");
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

    assert!(
        runtime
            .mark_thread_memory_mode_polluted(thread_id)
            .await
            .expect("mark thread polluted"),
        "thread should transition to polluted"
    );

    age_phase2_success_beyond_cooldown(&runtime).await;
    let next_claim = runtime
        .try_claim_global_phase2_job(owner, /*lease_seconds*/ 3600)
        .await
        .expect("claim phase2 after pollution");
    assert!(matches!(next_claim, Phase2JobClaimOutcome::Claimed { .. }));

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

#[tokio::test]
async fn get_phase2_input_selection_returns_regenerated_selected_rows() {
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

    let first_claim = runtime
        .try_claim_stage1_job(
            thread_id, owner, /*source_updated_at*/ 100, /*lease_seconds*/ 3600,
            /*max_running_jobs*/ 64,
        )
        .await
        .expect("claim initial stage1");
    let first_token = match first_claim {
        Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage1 claim outcome: {other:?}"),
    };
    assert!(
        runtime
            .mark_stage1_job_succeeded(
                thread_id,
                first_token.as_str(),
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

    let selection = runtime
        .get_phase2_input_selection(/*n*/ 1, /*max_unused_days*/ 36_500)
        .await
        .expect("load phase2 input selection");
    assert_eq!(selection.len(), 1);
    assert_eq!(selection[0].thread_id, thread_id);
    assert_eq!(selection[0].source_updated_at.timestamp(), 101);

    let (selected_for_phase2, selected_for_phase2_source_updated_at) =
            sqlx::query_as::<_, (i64, Option<i64>)>(
                "SELECT selected_for_phase2, selected_for_phase2_source_updated_at FROM stage1_outputs WHERE thread_id = ?",
            )
        .bind(thread_id.to_string())
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("load selected_for_phase2");
    assert_eq!(selected_for_phase2, 1);
    assert_eq!(selected_for_phase2_source_updated_at, Some(100));

    let _ = tokio::fs::remove_dir_all(codex_home).await;
}

