//! Core goal runtime integration.
//!
//! The pure goal planning/state helpers live in owner crates such as
//! `codex-agent-runtime` and `codex-state-api`. This module owns the concrete
//! `Session` bridge: state-db IO, typed event emission, metrics, continuation
//! scheduling, plus the thread-runtime owned goal service implementation.

mod runtime;
mod service;

pub(crate) use codex_agent_runtime::CreateGoalRequest;
pub(crate) use codex_agent_runtime::SetGoalRequest;
pub use service::GoalService;
pub(crate) use runtime::GoalRuntimeEvent;
