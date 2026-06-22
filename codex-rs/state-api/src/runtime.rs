use crate::AgentJob;
use crate::AgentJobCreateParams;
use crate::AgentJobItem;
use crate::AgentJobItemCreateParams;
use crate::AgentJobItemStatus;
use crate::AgentJobProgress;
use crate::DirectionalThreadSpawnEdgeStatus;
use crate::ThreadGoal;
use crate::ThreadGoalAccountingMode;
use crate::ThreadGoalAccountingOutcome;
use crate::ThreadGoalStatus;
use crate::ThreadGoalUpdate;
use crate::ThreadMetadata;
use codex_protocol::ThreadId;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

pub type StateApiFuture<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

/// Thread metadata and spawn-edge capability required by core.
///
/// Implementations own their storage and consistency model. Callers should treat
/// this as a narrow runtime boundary, not as permission to depend on the full
/// state database implementation.
pub trait ThreadStateRuntime: Send + Sync {
    fn get_thread(&self, thread_id: ThreadId) -> StateApiFuture<'_, Option<ThreadMetadata>>;

    fn insert_thread_if_absent(&self, metadata: ThreadMetadata) -> StateApiFuture<'_, bool>;

    fn get_dynamic_tools(
        &self,
        thread_id: ThreadId,
    ) -> StateApiFuture<'_, Option<Vec<DynamicToolSpec>>>;

    fn mark_thread_memory_mode_polluted(&self, thread_id: ThreadId) -> StateApiFuture<'_, ()>;

    fn find_rollout_path_by_id(
        &self,
        thread_id: ThreadId,
        archived_only: Option<bool>,
    ) -> StateApiFuture<'_, Option<PathBuf>>;

    fn list_thread_spawn_children_with_status(
        &self,
        parent_thread_id: ThreadId,
        status: DirectionalThreadSpawnEdgeStatus,
    ) -> StateApiFuture<'_, Vec<ThreadId>>;

    fn list_thread_spawn_descendants_with_status(
        &self,
        root_thread_id: ThreadId,
        status: DirectionalThreadSpawnEdgeStatus,
    ) -> StateApiFuture<'_, Vec<ThreadId>>;

    fn set_thread_spawn_edge_status(
        &self,
        child_thread_id: ThreadId,
        status: DirectionalThreadSpawnEdgeStatus,
    ) -> StateApiFuture<'_, ()>;

    fn upsert_thread_spawn_edge(
        &self,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
        status: DirectionalThreadSpawnEdgeStatus,
    ) -> StateApiFuture<'_, ()>;
}

/// Thread goal persistence capability required by core.
pub trait GoalStateRuntime: Send + Sync {
    fn get_thread_goal(&self, thread_id: ThreadId) -> StateApiFuture<'_, Option<ThreadGoal>>;

    fn replace_thread_goal<'a>(
        &'a self,
        thread_id: ThreadId,
        objective: &'a str,
        status: ThreadGoalStatus,
        token_budget: Option<i64>,
    ) -> StateApiFuture<'a, ThreadGoal>;

    fn insert_thread_goal<'a>(
        &'a self,
        thread_id: ThreadId,
        objective: &'a str,
        status: ThreadGoalStatus,
        token_budget: Option<i64>,
    ) -> StateApiFuture<'a, Option<ThreadGoal>>;

    fn update_thread_goal(
        &self,
        thread_id: ThreadId,
        update: ThreadGoalUpdate,
    ) -> StateApiFuture<'_, Option<ThreadGoal>>;

    fn pause_active_thread_goal(
        &self,
        thread_id: ThreadId,
    ) -> StateApiFuture<'_, Option<ThreadGoal>>;

    fn account_thread_goal_usage<'a>(
        &'a self,
        thread_id: ThreadId,
        time_delta_seconds: i64,
        token_delta: i64,
        mode: ThreadGoalAccountingMode,
        expected_goal_id: Option<&'a str>,
    ) -> StateApiFuture<'a, ThreadGoalAccountingOutcome>;
}

/// Agent-job persistence capability required by core's generic CSV worker tool.
pub trait AgentJobStateRuntime: Send + Sync {
    fn create_agent_job<'a>(
        &'a self,
        params: &'a AgentJobCreateParams,
        items: &'a [AgentJobItemCreateParams],
    ) -> StateApiFuture<'a, AgentJob>;

    fn get_agent_job<'a>(&'a self, job_id: &'a str) -> StateApiFuture<'a, Option<AgentJob>>;

    fn list_agent_job_items<'a>(
        &'a self,
        job_id: &'a str,
        status: Option<AgentJobItemStatus>,
        limit: Option<usize>,
    ) -> StateApiFuture<'a, Vec<AgentJobItem>>;

    fn get_agent_job_item<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
    ) -> StateApiFuture<'a, Option<AgentJobItem>>;

    fn mark_agent_job_running<'a>(&'a self, job_id: &'a str) -> StateApiFuture<'a, ()>;

    fn mark_agent_job_completed<'a>(&'a self, job_id: &'a str) -> StateApiFuture<'a, ()>;

    fn mark_agent_job_failed<'a>(
        &'a self,
        job_id: &'a str,
        error_message: &'a str,
    ) -> StateApiFuture<'a, ()>;

    fn mark_agent_job_cancelled<'a>(
        &'a self,
        job_id: &'a str,
        reason: &'a str,
    ) -> StateApiFuture<'a, bool>;

    fn is_agent_job_cancelled<'a>(&'a self, job_id: &'a str) -> StateApiFuture<'a, bool>;

    fn mark_agent_job_item_pending<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
        error_message: Option<&'a str>,
    ) -> StateApiFuture<'a, bool>;

    fn mark_agent_job_item_running_with_thread<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
        thread_id: &'a str,
    ) -> StateApiFuture<'a, bool>;

    fn report_agent_job_item_result<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
        reporting_thread_id: &'a str,
        result_json: &'a Value,
    ) -> StateApiFuture<'a, bool>;

    fn mark_agent_job_item_completed<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
    ) -> StateApiFuture<'a, bool>;

    fn mark_agent_job_item_failed<'a>(
        &'a self,
        job_id: &'a str,
        item_id: &'a str,
        error_message: &'a str,
    ) -> StateApiFuture<'a, bool>;

    fn get_agent_job_progress<'a>(
        &'a self,
        job_id: &'a str,
    ) -> StateApiFuture<'a, AgentJobProgress>;
}

/// Memory state side effects required by core while processing model output.
pub trait MemoryStateRuntime: Send + Sync {
    fn record_stage1_output_usage<'a>(
        &'a self,
        thread_ids: &'a [ThreadId],
    ) -> StateApiFuture<'a, usize>;
}

/// Core-facing state runtime facade.
pub trait StateDbRuntime:
    ThreadStateRuntime + GoalStateRuntime + AgentJobStateRuntime + MemoryStateRuntime
{
}

impl<T> StateDbRuntime for T where
    T: ThreadStateRuntime + GoalStateRuntime + AgentJobStateRuntime + MemoryStateRuntime
{
}

pub type SharedStateDbRuntime = Arc<dyn StateDbRuntime>;
