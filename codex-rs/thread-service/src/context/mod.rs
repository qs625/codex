//! Context fragments injected into model input.

mod environment_context;
mod runtime_activity;

pub(crate) use environment_context::environment_context_from_turn_context;
pub(crate) use runtime_activity::RuntimeActivityContext;
pub(crate) use runtime_activity::RuntimePollEventSnapshot;
