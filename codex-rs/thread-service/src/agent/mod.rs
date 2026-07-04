pub(crate) mod agent_resolver;
pub(crate) mod control;
pub(crate) mod job_tools;
pub(crate) mod multi_agent;
pub(crate) mod role;
pub(crate) mod spawn_support;

pub(crate) mod registry {
    pub(crate) use codex_agent_runtime::AgentMode;
    pub(crate) use codex_agent_runtime::SpawnAgentOptions;
    pub(crate) use codex_agent_runtime::exceeds_thread_spawn_depth_limit;
    pub(crate) use codex_agent_runtime::next_thread_spawn_depth;
}

pub(crate) mod status {
    pub(crate) use codex_agent_runtime::agent_status_from_event;
    pub(crate) use codex_agent_runtime::is_final;
}

pub(crate) use control::AgentControl;
pub(crate) use protocol::protocol::AgentStatus;
pub(crate) use registry::AgentMode;
pub(crate) use registry::SpawnAgentOptions;
pub(crate) use registry::exceeds_thread_spawn_depth_limit;
pub(crate) use registry::next_thread_spawn_depth;
pub(crate) use status::agent_status_from_event;
