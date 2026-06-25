mod child_completion_state;
mod control_plan;
mod fork_history;
mod goal_context;
mod goal_lifecycle;
mod goal_mutation;
mod goal_mutation_plan;
mod goal_runtime_state;
mod registry;
mod role_config;
#[cfg(test)]
mod role_config_tests;
mod status;
mod thread_post_turn;
mod tool_session_api;

pub use child_completion_state::ChildCompletionState;
pub use control_plan::AgentThreadActivityInputs;
pub use control_plan::ListAgentsPlan;
pub use control_plan::ListedAgent;
pub use control_plan::ListedAgentCandidate;
pub use control_plan::LiveAgent;
pub use control_plan::LiveAgentShutdownAction;
pub use control_plan::SpawnAgentOptions;
pub use control_plan::ThreadSpawnChild;
pub use control_plan::ThreadSpawnPlanInput;
pub use control_plan::agent_matches_prefix;
pub use control_plan::agent_nickname_candidates;
pub use control_plan::agent_subtree_thread_ids;
pub use control_plan::agent_thread_is_active_from_inputs;
pub use control_plan::any_agent_thread_active;
pub use control_plan::build_thread_spawn_children_by_parent;
pub use control_plan::current_agent_path_for_session;
pub use control_plan::default_agent_nickname_list;
pub use control_plan::direct_subagent_paths_from_children;
pub use control_plan::list_agents_plan;
pub use control_plan::live_agent_shutdown_action;
pub use control_plan::prepare_thread_spawn_plan;
pub use control_plan::render_input_preview;
pub use control_plan::resolve_agent_reference_path;
pub use control_plan::root_listed_agent;
pub use control_plan::should_ignore_descendant_shutdown_error;
pub use control_plan::should_register_session_root;
pub use control_plan::should_release_agent_after_thread_request_error;
pub use control_plan::thread_spawn_depth;
pub use control_plan::thread_spawn_descendants;
pub use control_plan::thread_spawn_parent_thread_id;
pub use fork_history::SpawnAgentForkMode;
pub use fork_history::select_forked_rollout_items;
pub use goal_context::goal_budget_limit_steering_item;
pub use goal_context::goal_continuation_input_item;
pub use goal_context::goal_objective_updated_steering_item;
pub use goal_context::should_ignore_goal_for_mode;
pub use goal_lifecycle::BudgetLimitSteering;
pub use goal_lifecycle::GoalRuntimeEvent;
pub use goal_lifecycle::GoalRuntimeLifecycleHost;
pub use goal_lifecycle::TerminalMetricEmission;
pub use goal_lifecycle::apply_goal_runtime_event;
pub use goal_mutation::CreateGoalRequest;
pub use goal_mutation::SetGoalRequest;
pub use goal_mutation::ThreadGoalMutationHost;
pub use goal_mutation::create_thread_goal;
pub use goal_mutation::set_thread_goal;
pub use goal_mutation_plan::ExternalGoalMutationPlan;
pub use goal_mutation_plan::ExternalGoalStatusAction;
pub use goal_mutation_plan::ThreadGoalMutationPlan;
pub use goal_mutation_plan::create_thread_goal_mutation_plan;
pub use goal_mutation_plan::external_goal_mutation_plan;
pub use goal_mutation_plan::set_thread_goal_mutation_plan;
pub use goal_runtime_state::GoalRuntimeState;
pub use registry::AgentMetadata;
pub use registry::AgentMode;
pub use registry::AgentRegistry;
pub use registry::SpawnReservation;
pub use registry::exceeds_thread_spawn_depth_limit;
pub use registry::next_thread_spawn_depth;
pub use role_config::AGENT_TYPE_UNAVAILABLE_ERROR;
pub use role_config::apply_role_to_config;
pub use status::agent_status_from_event;
pub use status::is_final;
pub use thread_post_turn::ThreadIdleReason;
pub use thread_post_turn::ThreadPostTurnInputs;
pub use thread_post_turn::ThreadPostTurnState;
pub use thread_post_turn::select_thread_post_turn_state;
pub use tool_session_api::CloseAgentToolResult;
pub use tool_session_api::ListAgentsToolResult;
pub use tool_session_api::MultiAgentToolSession;
pub use tool_session_api::SpawnAgentToolRequest;
pub use tool_session_api::SpawnAgentToolResult;
pub use tool_session_api::WaitAgentReason;
pub use tool_session_api::WaitAgentToolResult;
pub use tool_session_api::wait_agent_result_from_message;

const DEFAULT_AGENT_JOB_CONCURRENCY: usize = 16;
const MAX_AGENT_JOB_CONCURRENCY: usize = 64;

pub fn bounded_agent_job_concurrency(
    requested: Option<usize>,
    max_threads: Option<usize>,
) -> usize {
    let requested = requested.unwrap_or(DEFAULT_AGENT_JOB_CONCURRENCY).max(1);
    let requested = requested.min(MAX_AGENT_JOB_CONCURRENCY);
    if let Some(max_threads) = max_threads {
        requested.min(max_threads.max(1))
    } else {
        requested
    }
}
