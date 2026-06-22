use super::StateRuntime;
use codex_protocol::ThreadId;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_state_api::AgentJob;
use codex_state_api::AgentJobCreateParams;
use codex_state_api::AgentJobItem;
use codex_state_api::AgentJobItemCreateParams;
use codex_state_api::AgentJobItemStatus;
use codex_state_api::AgentJobProgress;
use codex_state_api::AgentJobStateRuntime;
use codex_state_api::DirectionalThreadSpawnEdgeStatus;
use codex_state_api::GoalStateRuntime;
use codex_state_api::MemoryStateRuntime;
use codex_state_api::StateApiFuture;
use codex_state_api::ThreadGoal;
use codex_state_api::ThreadGoalAccountingMode;
use codex_state_api::ThreadGoalAccountingOutcome;
use codex_state_api::ThreadGoalStatus;
use codex_state_api::ThreadGoalUpdate;
use codex_state_api::ThreadMetadata;
use codex_state_api::ThreadStateRuntime;
use serde_json::Value;
use std::path::PathBuf;

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
}

impl MemoryStateRuntime for StateRuntime {
    fn record_stage1_output_usage<'a>(
        &'a self,
        thread_ids: &'a [ThreadId],
    ) -> StateApiFuture<'a, usize> {
        Box::pin(async move { StateRuntime::record_stage1_output_usage(self, thread_ids).await })
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
