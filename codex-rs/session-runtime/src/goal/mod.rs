//! Core goal runtime integration.
//!
//! The pure goal planning/state helpers live in owner crates such as
//! `codex-agent-runtime` and `codex-state-api`. This module owns the concrete
//! `Session` bridge: state-db IO, typed event emission, metrics, continuation
//! scheduling, and the goal tool host adapter.

mod runtime;
mod tool_host;

pub(crate) use codex_agent_runtime::CreateGoalRequest;
pub(crate) use codex_agent_runtime::SetGoalRequest;
pub(crate) use runtime::GoalRuntimeEvent;
