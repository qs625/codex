//! Core goal runtime integration.
//!
//! The pure goal planning/state helpers live in owner crates such as
//! `codex-agent-runtime` and `codex-state-api`. This module owns the concrete
//! `Session` bridge: state-db IO, typed event emission, metrics, and
//! continuation scheduling.

mod runtime;

pub(crate) use runtime::GoalRuntimeEvent;
