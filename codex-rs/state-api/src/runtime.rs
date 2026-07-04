use crate::AgentJob;
use crate::AgentJobCreateParams;
use crate::AgentJobItem;
use crate::AgentJobItemCreateParams;
use crate::AgentJobItemStatus;
use crate::AgentJobProgress;
use crate::Anchor;
use crate::DirectionalThreadSpawnEdgeStatus;
use crate::LogEntry;
use crate::Phase2JobClaimOutcome;
use crate::RemoteControlEnrollmentRecord;
use crate::SortDirection;
use crate::SortKey;
use crate::Stage1JobClaim;
use crate::Stage1Output;
use crate::Stage1StartupClaimParams;
use crate::ThreadGoal;
use crate::ThreadGoalAccountingMode;
use crate::ThreadGoalAccountingOutcome;
use crate::ThreadGoalStatus;
use crate::ThreadGoalUpdate;
use crate::ThreadMetadata;
use crate::ThreadMetadataBuilder;
use crate::ThreadsPage;
use chrono::DateTime;
use chrono::Utc;
use protocol::ThreadId;
use protocol::dynamic_tools::DynamicToolSpec;
use protocol::protocol::RolloutItem;
use protocol::protocol::ThreadSkill;
use serde_json::Value;
use std::future::Future;
use std::path::Path;
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

    fn get_thread_memory_mode(&self, thread_id: ThreadId) -> StateApiFuture<'_, Option<String>>;

    fn mark_thread_memory_mode_polluted(&self, thread_id: ThreadId) -> StateApiFuture<'_, ()>;

    fn find_rollout_path_by_id(
        &self,
        thread_id: ThreadId,
        archived_only: Option<bool>,
    ) -> StateApiFuture<'_, Option<PathBuf>>;

    fn find_thread_by_exact_title<'a>(
        &'a self,
        title: &'a str,
        allowed_sources: &'a [String],
        model_providers: Option<&'a [String]>,
        archived_only: bool,
        cwd: Option<&'a Path>,
    ) -> StateApiFuture<'a, Option<ThreadMetadata>>;

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

    fn list_thread_spawn_descendants(
        &self,
        root_thread_id: ThreadId,
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
    ) -> StateApiFuture<'a, ThreadsPage>;

    fn list_thread_ids<'a>(
        &'a self,
        limit: usize,
        anchor: Option<&'a Anchor>,
        sort_key: SortKey,
        allowed_sources: &'a [String],
        model_providers: Option<&'a [String]>,
        archived_only: bool,
    ) -> StateApiFuture<'a, Vec<ThreadId>>;

    fn upsert_thread<'a>(&'a self, metadata: &'a ThreadMetadata) -> StateApiFuture<'a, ()>;

    fn mark_archived<'a>(
        &'a self,
        thread_id: ThreadId,
        rollout_path: &'a std::path::Path,
        archived_at: DateTime<Utc>,
    ) -> StateApiFuture<'a, ()>;

    fn mark_unarchived<'a>(
        &'a self,
        thread_id: ThreadId,
        rollout_path: &'a std::path::Path,
    ) -> StateApiFuture<'a, ()>;

    fn delete_thread(&self, thread_id: ThreadId) -> StateApiFuture<'_, u64>;

    fn persist_dynamic_tools<'a>(
        &'a self,
        thread_id: ThreadId,
        tools: Option<&'a [DynamicToolSpec]>,
    ) -> StateApiFuture<'a, ()>;

    fn get_thread_skills(
        &self,
        thread_id: ThreadId,
    ) -> StateApiFuture<'_, Option<Vec<ThreadSkill>>>;

    fn persist_thread_skills<'a>(
        &'a self,
        thread_id: ThreadId,
        skills: Option<&'a [ThreadSkill]>,
    ) -> StateApiFuture<'a, ()>;

    fn set_thread_memory_mode<'a>(
        &'a self,
        thread_id: ThreadId,
        memory_mode: &'a str,
    ) -> StateApiFuture<'a, bool>;

    fn update_thread_title<'a>(
        &'a self,
        thread_id: ThreadId,
        title: &'a str,
    ) -> StateApiFuture<'a, bool>;

    fn update_thread_git_info<'a>(
        &'a self,
        thread_id: ThreadId,
        git_sha: Option<Option<&'a str>>,
        git_branch: Option<Option<&'a str>>,
        git_origin_url: Option<Option<&'a str>>,
    ) -> StateApiFuture<'a, bool>;

    fn apply_rollout_items<'a>(
        &'a self,
        builder: &'a ThreadMetadataBuilder,
        items: &'a [RolloutItem],
        new_thread_memory_mode: Option<&'a str>,
        updated_at_override: Option<DateTime<Utc>>,
    ) -> StateApiFuture<'a, ()>;

    fn touch_thread_updated_at(
        &self,
        thread_id: ThreadId,
        updated_at: DateTime<Utc>,
    ) -> StateApiFuture<'_, bool>;
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

    fn delete_thread_goal(&self, thread_id: ThreadId) -> StateApiFuture<'_, bool>;
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
    fn prune_stage1_outputs_for_retention(
        &self,
        max_unused_days: i64,
        prune_batch_size: usize,
    ) -> StateApiFuture<'_, usize>;

    fn claim_stage1_jobs_for_startup<'a>(
        &'a self,
        current_thread_id: ThreadId,
        params: &'a Stage1StartupClaimParams,
    ) -> StateApiFuture<'a, Vec<Stage1JobClaim>>;

    fn mark_stage1_job_failed<'a>(
        &'a self,
        thread_id: ThreadId,
        ownership_token: &'a str,
        error_message: &'a str,
        retry_delay_seconds: i64,
    ) -> StateApiFuture<'a, bool>;

    fn mark_stage1_job_succeeded_no_output<'a>(
        &'a self,
        thread_id: ThreadId,
        ownership_token: &'a str,
    ) -> StateApiFuture<'a, bool>;

    fn mark_stage1_job_succeeded<'a>(
        &'a self,
        thread_id: ThreadId,
        ownership_token: &'a str,
        source_updated_at: i64,
        raw_memory: &'a str,
        rollout_summary: &'a str,
        rollout_slug: Option<&'a str>,
    ) -> StateApiFuture<'a, bool>;

    fn get_phase2_input_selection(
        &self,
        max_raw_memories: usize,
        max_unused_days: i64,
    ) -> StateApiFuture<'_, Vec<Stage1Output>>;

    fn try_claim_global_phase2_job(
        &self,
        current_thread_id: ThreadId,
        lease_seconds: i64,
    ) -> StateApiFuture<'_, Phase2JobClaimOutcome>;

    fn mark_global_phase2_job_failed<'a>(
        &'a self,
        ownership_token: &'a str,
        error_message: &'a str,
        retry_delay_seconds: i64,
    ) -> StateApiFuture<'a, bool>;

    fn mark_global_phase2_job_failed_if_unowned<'a>(
        &'a self,
        ownership_token: &'a str,
        error_message: &'a str,
        retry_delay_seconds: i64,
    ) -> StateApiFuture<'a, bool>;

    fn mark_global_phase2_job_succeeded<'a>(
        &'a self,
        ownership_token: &'a str,
        completion_watermark: i64,
        selected_outputs: &'a [Stage1Output],
    ) -> StateApiFuture<'a, bool>;

    fn heartbeat_global_phase2_job<'a>(
        &'a self,
        ownership_token: &'a str,
        lease_seconds: i64,
    ) -> StateApiFuture<'a, bool>;

    fn record_stage1_output_usage<'a>(
        &'a self,
        thread_ids: &'a [ThreadId],
    ) -> StateApiFuture<'a, usize>;

    fn clear_memory_data(&self) -> StateApiFuture<'_, ()>;

    fn query_feedback_logs_for_threads<'a>(
        &'a self,
        thread_ids: &'a [&'a str],
    ) -> StateApiFuture<'a, Vec<u8>>;
}

pub trait LogStateRuntime: Send + Sync {
    fn insert_logs<'a>(&'a self, entries: &'a [LogEntry]) -> StateApiFuture<'a, ()>;
}

pub trait RemoteControlStateRuntime: Send + Sync {
    fn get_remote_control_enrollment<'a>(
        &'a self,
        websocket_url: &'a str,
        account_id: &'a str,
        app_server_client_name: Option<&'a str>,
    ) -> StateApiFuture<'a, Option<RemoteControlEnrollmentRecord>>;

    fn upsert_remote_control_enrollment<'a>(
        &'a self,
        enrollment: &'a RemoteControlEnrollmentRecord,
    ) -> StateApiFuture<'a, ()>;

    fn delete_remote_control_enrollment<'a>(
        &'a self,
        websocket_url: &'a str,
        account_id: &'a str,
        app_server_client_name: Option<&'a str>,
    ) -> StateApiFuture<'a, u64>;
}

/// Core-facing state runtime facade.
pub trait StateDbRuntime:
    ThreadStateRuntime
    + GoalStateRuntime
    + AgentJobStateRuntime
    + MemoryStateRuntime
    + LogStateRuntime
    + RemoteControlStateRuntime
{
}

impl<T> StateDbRuntime for T where
    T: ThreadStateRuntime
        + GoalStateRuntime
        + AgentJobStateRuntime
        + MemoryStateRuntime
        + LogStateRuntime
        + RemoteControlStateRuntime
{
}

pub type SharedStateDbRuntime = Arc<dyn StateDbRuntime>;
