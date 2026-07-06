use super::StateRuntime;
use chrono::DateTime;
use chrono::Utc;
use protocol::ThreadId;
use protocol::dynamic_tools::DynamicToolSpec;
use protocol::protocol::RolloutItem;
use protocol::protocol::ThreadSkill;
use serde_json::Value;
use state_api::AgentJob;
use state_api::AgentJobCreateParams;
use state_api::AgentJobItem;
use state_api::AgentJobItemCreateParams;
use state_api::AgentJobItemStatus;
use state_api::AgentJobProgress;
use state_api::AgentJobStateRuntime;
use state_api::Anchor;
use state_api::DirectionalThreadSpawnEdgeStatus;
use state_api::GoalStateRuntime;
use state_api::LogEntry;
use state_api::LogStateRuntime;
use state_api::MemoryStateRuntime;
use state_api::Phase2JobClaimOutcome;
use state_api::RemoteControlEnrollmentRecord;
use state_api::RemoteControlStateRuntime;
use state_api::SortDirection;
use state_api::SortKey;
use state_api::Stage1JobClaim;
use state_api::Stage1Output;
use state_api::Stage1StartupClaimParams;
use state_api::StateApiFuture;
use state_api::ThreadGoal;
use state_api::ThreadGoalAccountingMode;
use state_api::ThreadGoalAccountingOutcome;
use state_api::ThreadGoalStatus;
use state_api::ThreadGoalUpdate;
use state_api::ThreadMetadata;
use state_api::ThreadMetadataBuilder;
use state_api::ThreadStateRuntime;
use state_api::ThreadsPage;
use std::path::Path;
use std::path::PathBuf;

fn stage1_output_from_state(value: crate::Stage1Output) -> Stage1Output {
    Stage1Output {
        thread_id: value.thread_id,
        rollout_path: value.rollout_path,
        source_updated_at: value.source_updated_at,
        raw_memory: value.raw_memory,
        rollout_summary: value.rollout_summary,
        rollout_slug: value.rollout_slug,
        cwd: value.cwd,
        git_branch: value.git_branch,
        generated_at: value.generated_at,
    }
}

fn stage1_job_claim_from_state(value: crate::Stage1JobClaim) -> Stage1JobClaim {
    Stage1JobClaim {
        thread: value.thread,
        ownership_token: value.ownership_token,
    }
}

fn phase2_job_claim_outcome_from_state(
    value: crate::Phase2JobClaimOutcome,
) -> Phase2JobClaimOutcome {
    match value {
        crate::Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        },
        crate::Phase2JobClaimOutcome::SkippedRetryUnavailable => {
            Phase2JobClaimOutcome::SkippedRetryUnavailable
        }
        crate::Phase2JobClaimOutcome::SkippedCooldown => Phase2JobClaimOutcome::SkippedCooldown,
        crate::Phase2JobClaimOutcome::SkippedRunning => Phase2JobClaimOutcome::SkippedRunning,
    }
}

fn remote_control_enrollment_record_from_state(
    value: crate::RemoteControlEnrollmentRecord,
) -> RemoteControlEnrollmentRecord {
    RemoteControlEnrollmentRecord {
        websocket_url: value.websocket_url,
        account_id: value.account_id,
        app_server_client_name: value.app_server_client_name,
        server_id: value.server_id,
        environment_id: value.environment_id,
        server_name: value.server_name,
    }
}

fn remote_control_enrollment_record_to_state(
    value: &RemoteControlEnrollmentRecord,
) -> crate::RemoteControlEnrollmentRecord {
    crate::RemoteControlEnrollmentRecord {
        websocket_url: value.websocket_url.clone(),
        account_id: value.account_id.clone(),
        app_server_client_name: value.app_server_client_name.clone(),
        server_id: value.server_id.clone(),
        environment_id: value.environment_id.clone(),
        server_name: value.server_name.clone(),
    }
}

fn log_entry_to_state(value: &LogEntry) -> crate::LogEntry {
    crate::LogEntry {
        ts: value.ts,
        ts_nanos: value.ts_nanos,
        level: value.level.clone(),
        target: value.target.clone(),
        message: value.message.clone(),
        feedback_log_body: value.feedback_log_body.clone(),
        thread_id: value.thread_id.clone(),
        process_uuid: value.process_uuid.clone(),
        module_path: value.module_path.clone(),
        file: value.file.clone(),
        line: value.line,
    }
}

impl ThreadStateRuntime for StateRuntime {
    fn get_thread(&self, thread_id: ThreadId) -> StateApiFuture<'_, Option<ThreadMetadata>> {
        Box::pin(async move { StateRuntime::get_thread(self, thread_id).await })
    }

    fn insert_thread_if_absent(&self, metadata: ThreadMetadata) -> StateApiFuture<'_, bool> {
        Box::pin(async move { StateRuntime::insert_thread_if_absent(self, &metadata).await })
    }

    fn get_dynamic_tools(
        &self,
        thread_id: ThreadId,
    ) -> StateApiFuture<'_, Option<Vec<DynamicToolSpec>>> {
        Box::pin(async move { StateRuntime::get_dynamic_tools(self, thread_id).await })
    }

    fn get_thread_memory_mode(&self, thread_id: ThreadId) -> StateApiFuture<'_, Option<String>> {
        Box::pin(async move { StateRuntime::get_thread_memory_mode(self, thread_id).await })
    }

    fn mark_thread_memory_mode_polluted(&self, thread_id: ThreadId) -> StateApiFuture<'_, ()> {
        Box::pin(async move {
            StateRuntime::mark_thread_memory_mode_polluted(self, thread_id)
                .await
                .map(|_| ())
        })
    }

    fn find_rollout_path_by_id(
        &self,
        thread_id: ThreadId,
        archived_only: Option<bool>,
    ) -> StateApiFuture<'_, Option<PathBuf>> {
        Box::pin(async move {
            StateRuntime::find_rollout_path_by_id(self, thread_id, archived_only).await
        })
    }

    fn find_thread_by_exact_title<'a>(
        &'a self,
        title: &'a str,
        allowed_sources: &'a [String],
        model_providers: Option<&'a [String]>,
        archived_only: bool,
        cwd: Option<&'a Path>,
    ) -> StateApiFuture<'a, Option<ThreadMetadata>> {
        Box::pin(async move {
            StateRuntime::find_thread_by_exact_title(
                self,
                title,
                allowed_sources,
                model_providers,
                archived_only,
                cwd,
            )
            .await
        })
    }

    fn list_thread_spawn_children_with_status(
        &self,
        parent_thread_id: ThreadId,
        status: DirectionalThreadSpawnEdgeStatus,
    ) -> StateApiFuture<'_, Vec<ThreadId>> {
        Box::pin(async move {
            StateRuntime::list_thread_spawn_children_with_status(self, parent_thread_id, status)
                .await
        })
    }

    fn list_thread_spawn_descendants_with_status(
        &self,
        root_thread_id: ThreadId,
        status: DirectionalThreadSpawnEdgeStatus,
    ) -> StateApiFuture<'_, Vec<ThreadId>> {
        Box::pin(async move {
            StateRuntime::list_thread_spawn_descendants_with_status(self, root_thread_id, status)
                .await
        })
    }

    fn list_thread_spawn_descendants(
        &self,
        root_thread_id: ThreadId,
    ) -> StateApiFuture<'_, Vec<ThreadId>> {
        Box::pin(
            async move { StateRuntime::list_thread_spawn_descendants(self, root_thread_id).await },
        )
    }

    fn set_thread_spawn_edge_status(
        &self,
        child_thread_id: ThreadId,
        status: DirectionalThreadSpawnEdgeStatus,
    ) -> StateApiFuture<'_, ()> {
        Box::pin(async move {
            StateRuntime::set_thread_spawn_edge_status(self, child_thread_id, status).await
        })
    }

    fn upsert_thread_spawn_edge(
        &self,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
        status: DirectionalThreadSpawnEdgeStatus,
    ) -> StateApiFuture<'_, ()> {
        Box::pin(async move {
            StateRuntime::upsert_thread_spawn_edge(self, parent_thread_id, child_thread_id, status)
                .await
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn list_threads<'a>(
        &'a self,
        page_size: usize,
        archived_only: bool,
        allowed_sources: &'a [String],
        model_providers: Option<&'a [String]>,
        cwd_filters: Option<&'a [PathBuf]>,
        anchor: Option<&'a Anchor>,
        sort_key: SortKey,
        sort_direction: SortDirection,
        search_term: Option<&'a str>,
    ) -> StateApiFuture<'a, ThreadsPage> {
        Box::pin(async move {
            StateRuntime::list_threads(
                self,
                page_size,
                super::threads::ThreadFilterOptions {
                    archived_only,
                    allowed_sources,
                    model_providers,
                    cwd_filters,
                    anchor,
                    sort_key,
                    sort_direction,
                    search_term,
                },
            )
            .await
        })
    }

    fn list_thread_ids<'a>(
        &'a self,
        limit: usize,
        anchor: Option<&'a Anchor>,
        sort_key: SortKey,
        allowed_sources: &'a [String],
        model_providers: Option<&'a [String]>,
        archived_only: bool,
    ) -> StateApiFuture<'a, Vec<ThreadId>> {
        Box::pin(async move {
            StateRuntime::list_thread_ids(
                self,
                limit,
                anchor,
                sort_key,
                allowed_sources,
                model_providers,
                archived_only,
            )
            .await
        })
    }

    fn upsert_thread<'a>(&'a self, metadata: &'a ThreadMetadata) -> StateApiFuture<'a, ()> {
        Box::pin(async move { StateRuntime::upsert_thread(self, metadata).await })
    }

    fn mark_archived<'a>(
        &'a self,
        thread_id: ThreadId,
        rollout_path: &'a std::path::Path,
        archived_at: DateTime<Utc>,
    ) -> StateApiFuture<'a, ()> {
        Box::pin(async move {
            StateRuntime::mark_archived(self, thread_id, rollout_path, archived_at).await
        })
    }

    fn mark_unarchived<'a>(
        &'a self,
        thread_id: ThreadId,
        rollout_path: &'a std::path::Path,
    ) -> StateApiFuture<'a, ()> {
        Box::pin(async move { StateRuntime::mark_unarchived(self, thread_id, rollout_path).await })
    }

    fn delete_thread(&self, thread_id: ThreadId) -> StateApiFuture<'_, u64> {
        Box::pin(async move { StateRuntime::delete_thread(self, thread_id).await })
    }

    fn persist_dynamic_tools<'a>(
        &'a self,
        thread_id: ThreadId,
        tools: Option<&'a [DynamicToolSpec]>,
    ) -> StateApiFuture<'a, ()> {
        Box::pin(async move { StateRuntime::persist_dynamic_tools(self, thread_id, tools).await })
    }

    fn get_thread_skills(
        &self,
        thread_id: ThreadId,
    ) -> StateApiFuture<'_, Option<Vec<ThreadSkill>>> {
        Box::pin(async move { StateRuntime::get_thread_skills(self, thread_id).await })
    }

    fn persist_thread_skills<'a>(
        &'a self,
        thread_id: ThreadId,
        skills: Option<&'a [ThreadSkill]>,
    ) -> StateApiFuture<'a, ()> {
        Box::pin(async move { StateRuntime::persist_thread_skills(self, thread_id, skills).await })
    }

    fn set_thread_memory_mode<'a>(
        &'a self,
        thread_id: ThreadId,
        memory_mode: &'a str,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(
            async move { StateRuntime::set_thread_memory_mode(self, thread_id, memory_mode).await },
        )
    }

    fn update_thread_title<'a>(
        &'a self,
        thread_id: ThreadId,
        title: &'a str,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move { StateRuntime::update_thread_title(self, thread_id, title).await })
    }

    fn update_thread_git_info<'a>(
        &'a self,
        thread_id: ThreadId,
        git_sha: Option<Option<&'a str>>,
        git_branch: Option<Option<&'a str>>,
        git_origin_url: Option<Option<&'a str>>,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::update_thread_git_info(
                self,
                thread_id,
                git_sha,
                git_branch,
                git_origin_url,
            )
            .await
        })
    }

    fn apply_rollout_items<'a>(
        &'a self,
        builder: &'a ThreadMetadataBuilder,
        items: &'a [RolloutItem],
        new_thread_memory_mode: Option<&'a str>,
        updated_at_override: Option<DateTime<Utc>>,
    ) -> StateApiFuture<'a, ()> {
        Box::pin(async move {
            StateRuntime::apply_rollout_items(
                self,
                builder,
                items,
                new_thread_memory_mode,
                updated_at_override,
            )
            .await
        })
    }

    fn touch_thread_updated_at(
        &self,
        thread_id: ThreadId,
        updated_at: DateTime<Utc>,
    ) -> StateApiFuture<'_, bool> {
        Box::pin(
            async move { StateRuntime::touch_thread_updated_at(self, thread_id, updated_at).await },
        )
    }
}

impl GoalStateRuntime for StateRuntime {
    fn get_thread_goal(&self, thread_id: ThreadId) -> StateApiFuture<'_, Option<ThreadGoal>> {
        Box::pin(async move { StateRuntime::get_thread_goal(self, thread_id).await })
    }

    fn replace_thread_goal<'a>(
        &'a self,
        thread_id: ThreadId,
        objective: &'a str,
        status: ThreadGoalStatus,
        token_budget: Option<i64>,
    ) -> StateApiFuture<'a, ThreadGoal> {
        Box::pin(async move {
            StateRuntime::replace_thread_goal(self, thread_id, objective, status, token_budget)
                .await
        })
    }

    fn insert_thread_goal<'a>(
        &'a self,
        thread_id: ThreadId,
        objective: &'a str,
        status: ThreadGoalStatus,
        token_budget: Option<i64>,
    ) -> StateApiFuture<'a, Option<ThreadGoal>> {
        Box::pin(async move {
            StateRuntime::insert_thread_goal(self, thread_id, objective, status, token_budget).await
        })
    }

    fn update_thread_goal(
        &self,
        thread_id: ThreadId,
        update: ThreadGoalUpdate,
    ) -> StateApiFuture<'_, Option<ThreadGoal>> {
        Box::pin(async move { StateRuntime::update_thread_goal(self, thread_id, update).await })
    }

    fn pause_active_thread_goal(
        &self,
        thread_id: ThreadId,
    ) -> StateApiFuture<'_, Option<ThreadGoal>> {
        Box::pin(async move { StateRuntime::pause_active_thread_goal(self, thread_id).await })
    }

    fn account_thread_goal_usage<'a>(
        &'a self,
        thread_id: ThreadId,
        time_delta_seconds: i64,
        token_delta: i64,
        mode: ThreadGoalAccountingMode,
        expected_goal_id: Option<&'a str>,
    ) -> StateApiFuture<'a, ThreadGoalAccountingOutcome> {
        Box::pin(async move {
            StateRuntime::account_thread_goal_usage(
                self,
                thread_id,
                time_delta_seconds,
                token_delta,
                mode,
                expected_goal_id,
            )
            .await
        })
    }

    fn delete_thread_goal(&self, thread_id: ThreadId) -> StateApiFuture<'_, bool> {
        Box::pin(async move { StateRuntime::delete_thread_goal(self, thread_id).await })
    }
}

impl MemoryStateRuntime for StateRuntime {
    fn prune_stage1_outputs_for_retention(
        &self,
        max_unused_days: i64,
        prune_batch_size: usize,
    ) -> StateApiFuture<'_, usize> {
        Box::pin(async move {
            StateRuntime::prune_stage1_outputs_for_retention(
                self,
                max_unused_days,
                prune_batch_size,
            )
            .await
        })
    }

    fn claim_stage1_jobs_for_startup<'a>(
        &'a self,
        current_thread_id: ThreadId,
        params: &'a Stage1StartupClaimParams,
    ) -> StateApiFuture<'a, Vec<Stage1JobClaim>> {
        Box::pin(async move {
            StateRuntime::claim_stage1_jobs_for_startup(
                self,
                current_thread_id,
                crate::Stage1StartupClaimParams {
                    scan_limit: params.scan_limit,
                    max_claimed: params.max_claimed,
                    max_age_days: params.max_age_days,
                    min_rollout_idle_hours: params.min_rollout_idle_hours,
                    allowed_sources: params.allowed_sources.as_slice(),
                    lease_seconds: params.lease_seconds,
                },
            )
            .await
            .map(|claims| {
                claims
                    .into_iter()
                    .map(stage1_job_claim_from_state)
                    .collect()
            })
        })
    }

    fn mark_stage1_job_failed<'a>(
        &'a self,
        thread_id: ThreadId,
        ownership_token: &'a str,
        error_message: &'a str,
        retry_delay_seconds: i64,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::mark_stage1_job_failed(
                self,
                thread_id,
                ownership_token,
                error_message,
                retry_delay_seconds,
            )
            .await
        })
    }

    fn mark_stage1_job_succeeded_no_output<'a>(
        &'a self,
        thread_id: ThreadId,
        ownership_token: &'a str,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::mark_stage1_job_succeeded_no_output(self, thread_id, ownership_token)
                .await
        })
    }

    fn mark_stage1_job_succeeded<'a>(
        &'a self,
        thread_id: ThreadId,
        ownership_token: &'a str,
        source_updated_at: i64,
        raw_memory: &'a str,
        rollout_summary: &'a str,
        rollout_slug: Option<&'a str>,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::mark_stage1_job_succeeded(
                self,
                thread_id,
                ownership_token,
                source_updated_at,
                raw_memory,
                rollout_summary,
                rollout_slug,
            )
            .await
        })
    }

    fn get_phase2_input_selection(
        &self,
        max_raw_memories: usize,
        max_unused_days: i64,
    ) -> StateApiFuture<'_, Vec<Stage1Output>> {
        Box::pin(async move {
            StateRuntime::get_phase2_input_selection(self, max_raw_memories, max_unused_days)
                .await
                .map(|outputs| outputs.into_iter().map(stage1_output_from_state).collect())
        })
    }

    fn try_claim_global_phase2_job(
        &self,
        current_thread_id: ThreadId,
        lease_seconds: i64,
    ) -> StateApiFuture<'_, Phase2JobClaimOutcome> {
        Box::pin(async move {
            StateRuntime::try_claim_global_phase2_job(self, current_thread_id, lease_seconds)
                .await
                .map(phase2_job_claim_outcome_from_state)
        })
    }

    fn mark_global_phase2_job_failed<'a>(
        &'a self,
        ownership_token: &'a str,
        error_message: &'a str,
        retry_delay_seconds: i64,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::mark_global_phase2_job_failed(
                self,
                ownership_token,
                error_message,
                retry_delay_seconds,
            )
            .await
        })
    }

    fn mark_global_phase2_job_failed_if_unowned<'a>(
        &'a self,
        ownership_token: &'a str,
        error_message: &'a str,
        retry_delay_seconds: i64,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::mark_global_phase2_job_failed_if_unowned(
                self,
                ownership_token,
                error_message,
                retry_delay_seconds,
            )
            .await
        })
    }

    fn mark_global_phase2_job_succeeded<'a>(
        &'a self,
        ownership_token: &'a str,
        completion_watermark: i64,
        selected_outputs: &'a [Stage1Output],
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            let selected_outputs = selected_outputs
                .iter()
                .cloned()
                .map(|output| crate::Stage1Output {
                    thread_id: output.thread_id,
                    rollout_path: output.rollout_path,
                    source_updated_at: output.source_updated_at,
                    raw_memory: output.raw_memory,
                    rollout_summary: output.rollout_summary,
                    rollout_slug: output.rollout_slug,
                    cwd: output.cwd,
                    git_branch: output.git_branch,
                    generated_at: output.generated_at,
                })
                .collect::<Vec<_>>();
            StateRuntime::mark_global_phase2_job_succeeded(
                self,
                ownership_token,
                completion_watermark,
                selected_outputs.as_slice(),
            )
            .await
        })
    }

    fn heartbeat_global_phase2_job<'a>(
        &'a self,
        ownership_token: &'a str,
        lease_seconds: i64,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::heartbeat_global_phase2_job(self, ownership_token, lease_seconds).await
        })
    }

    fn record_stage1_output_usage<'a>(
        &'a self,
        thread_ids: &'a [ThreadId],
    ) -> StateApiFuture<'a, usize> {
        Box::pin(async move { StateRuntime::record_stage1_output_usage(self, thread_ids).await })
    }

    fn clear_memory_data(&self) -> StateApiFuture<'_, ()> {
        Box::pin(async move { StateRuntime::clear_memory_data(self).await })
    }

    fn query_feedback_logs_for_threads<'a>(
        &'a self,
        thread_ids: &'a [&'a str],
    ) -> StateApiFuture<'a, Vec<u8>> {
        Box::pin(
            async move { StateRuntime::query_feedback_logs_for_threads(self, thread_ids).await },
        )
    }
}

impl LogStateRuntime for StateRuntime {
    fn insert_logs<'a>(&'a self, entries: &'a [LogEntry]) -> StateApiFuture<'a, ()> {
        Box::pin(async move {
            let entries = entries.iter().map(log_entry_to_state).collect::<Vec<_>>();
            StateRuntime::insert_logs(self, entries.as_slice()).await
        })
    }
}

impl RemoteControlStateRuntime for StateRuntime {
    fn get_remote_control_enrollment<'a>(
        &'a self,
        websocket_url: &'a str,
        account_id: &'a str,
        app_server_client_name: Option<&'a str>,
    ) -> StateApiFuture<'a, Option<RemoteControlEnrollmentRecord>> {
        Box::pin(async move {
            StateRuntime::get_remote_control_enrollment(
                self,
                websocket_url,
                account_id,
                app_server_client_name,
            )
            .await
            .map(|value| value.map(remote_control_enrollment_record_from_state))
        })
    }

    fn upsert_remote_control_enrollment<'a>(
        &'a self,
        enrollment: &'a RemoteControlEnrollmentRecord,
    ) -> StateApiFuture<'a, ()> {
        Box::pin(async move {
            let enrollment = remote_control_enrollment_record_to_state(enrollment);
            StateRuntime::upsert_remote_control_enrollment(self, &enrollment).await
        })
    }

    fn delete_remote_control_enrollment<'a>(
        &'a self,
        websocket_url: &'a str,
        account_id: &'a str,
        app_server_client_name: Option<&'a str>,
    ) -> StateApiFuture<'a, u64> {
        Box::pin(async move {
            StateRuntime::delete_remote_control_enrollment(
                self,
                websocket_url,
                account_id,
                app_server_client_name,
            )
            .await
        })
    }
}

impl AgentJobStateRuntime for StateRuntime {
    fn create_agent_job<'a>(
        &'a self,
        params: &'a AgentJobCreateParams,
        items: &'a [AgentJobItemCreateParams],
    ) -> StateApiFuture<'a, AgentJob> {
        Box::pin(async move { StateRuntime::create_agent_job(self, params, items).await })
    }

    fn get_agent_job<'a>(&'a self, job_id: &'a str) -> StateApiFuture<'a, Option<AgentJob>> {
        Box::pin(async move { StateRuntime::get_agent_job(self, job_id).await })
    }

    fn list_agent_job_items<'a>(
        &'a self,
        job_id: &'a str,
        status: Option<AgentJobItemStatus>,
        limit: Option<usize>,
    ) -> StateApiFuture<'a, Vec<AgentJobItem>> {
        Box::pin(
            async move { StateRuntime::list_agent_job_items(self, job_id, status, limit).await },
        )
    }

    fn get_agent_job_item<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
    ) -> StateApiFuture<'a, Option<AgentJobItem>> {
        Box::pin(async move { StateRuntime::get_agent_job_item(self, job_id, item_id).await })
    }

    fn mark_agent_job_running<'a>(&'a self, job_id: &'a str) -> StateApiFuture<'a, ()> {
        Box::pin(async move { StateRuntime::mark_agent_job_running(self, job_id).await })
    }

    fn mark_agent_job_completed<'a>(&'a self, job_id: &'a str) -> StateApiFuture<'a, ()> {
        Box::pin(async move { StateRuntime::mark_agent_job_completed(self, job_id).await })
    }

    fn mark_agent_job_failed<'a>(
        &'a self,
        job_id: &'a str,
        error_message: &'a str,
    ) -> StateApiFuture<'a, ()> {
        Box::pin(
            async move { StateRuntime::mark_agent_job_failed(self, job_id, error_message).await },
        )
    }

    fn mark_agent_job_cancelled<'a>(
        &'a self,
        job_id: &'a str,
        reason: &'a str,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move { StateRuntime::mark_agent_job_cancelled(self, job_id, reason).await })
    }

    fn is_agent_job_cancelled<'a>(&'a self, job_id: &'a str) -> StateApiFuture<'a, bool> {
        Box::pin(async move { StateRuntime::is_agent_job_cancelled(self, job_id).await })
    }

    fn mark_agent_job_item_pending<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
        error_message: Option<&'a str>,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::mark_agent_job_item_pending(self, job_id, item_id, error_message).await
        })
    }

    fn mark_agent_job_item_running_with_thread<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
        thread_id: &'a str,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::mark_agent_job_item_running_with_thread(self, job_id, item_id, thread_id)
                .await
        })
    }

    fn report_agent_job_item_result<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
        reporting_thread_id: &'a str,
        result_json: &'a Value,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::report_agent_job_item_result(
                self,
                job_id,
                item_id,
                reporting_thread_id,
                result_json,
            )
            .await
        })
    }

    fn mark_agent_job_item_completed<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(
            async move { StateRuntime::mark_agent_job_item_completed(self, job_id, item_id).await },
        )
    }

    fn mark_agent_job_item_failed<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
        error_message: &'a str,
    ) -> StateApiFuture<'a, bool> {
        Box::pin(async move {
            StateRuntime::mark_agent_job_item_failed(self, job_id, item_id, error_message).await
        })
    }

    fn get_agent_job_progress<'a>(
        &'a self,
        job_id: &'a str,
    ) -> StateApiFuture<'a, AgentJobProgress> {
        Box::pin(async move { StateRuntime::get_agent_job_progress(self, job_id).await })
    }
}
