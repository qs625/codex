//! Guardian review decides whether an `on-request` approval should be granted
//! automatically instead of shown to the user.
//!
//! High-level approach:
//! 1. Reconstruct a compact transcript that preserves user intent plus the most
//!    relevant recent assistant and tool context.
//! 2. Ask a dedicated guardian review session to assess the exact planned
//!    action and return strict JSON.
//!    The guardian clones the parent config, so it inherits any managed
//!    network proxy / allowlist that the parent turn already had.
//! 3. Fail closed on timeout, execution failure, or malformed output.
//! 4. Apply the guardian's explicit allow/deny outcome.

#[cfg(test)]
use std::time::Duration;

pub(crate) use codex_protocol::protocol::GuardianAssessmentOutcome;

pub(crate) use codex_guardian::AUTO_REVIEW_DENIAL_WINDOW_SIZE;
pub(crate) use codex_guardian::AUTO_REVIEW_DENIED_ACTION_APPROVAL_DEVELOPER_PREFIX;
pub use codex_guardian::GuardianApprovalRequest;
pub(crate) use codex_guardian::GuardianAssessment;
#[cfg(test)]
pub(crate) use codex_guardian::GuardianMcpAnnotations;
pub(crate) use codex_guardian::GuardianNetworkAccessTrigger;
pub(crate) use codex_guardian::GuardianRejection;
pub(crate) use codex_guardian::GuardianRejectionCircuitBreaker;
pub(crate) use codex_guardian::GuardianRejectionCircuitBreakerAction;
#[cfg(test)]
pub(crate) use codex_guardian::guardian_approval_request_to_json;
#[cfg(test)]
pub(crate) use crate::session_capability::approval_review_runtime_impl::GuardianReviewOutcome;
#[cfg(test)]
pub(crate) use crate::session_capability::approval_review_runtime_impl::record_guardian_denial_for_test;
#[cfg(test)]
pub(crate) use crate::session_capability::approval_review_runtime_impl::review_approval_request_with_cancel;
#[cfg(test)]
pub(crate) use crate::session::session::approval_review_session_impl::GuardianReviewSessionReuseKey;

#[cfg(test)]
pub(crate) const GUARDIAN_REVIEW_TIMEOUT: Duration = Duration::from_secs(90);
pub(crate) const GUARDIAN_REVIEWER_NAME: &str = "guardian";
#[cfg(test)]
use codex_guardian::GuardianPromptMode;
#[cfg(test)]
use codex_guardian::GuardianTranscriptCursor;
#[cfg(test)]
use codex_guardian::format_guardian_action_pretty;
#[cfg(test)]
use codex_guardian::guardian_assessment_action;
#[cfg(test)]
use codex_guardian::guardian_output_schema;
#[cfg(test)]
pub(crate) use codex_guardian::guardian_policy_prompt;
#[cfg(test)]
pub(crate) use codex_guardian::guardian_policy_prompt_with_config;
#[cfg(test)]
use codex_guardian::guardian_request_turn_id;
#[cfg(test)]
use crate::session_capability::approval_review_runtime_impl::GuardianReviewOutcome;
#[cfg(test)]
use crate::session_capability::approval_review_runtime_impl::run_review_session as run_review_session_for_test;
#[cfg(test)]
use crate::session::session::approval_review_session_impl::build_guardian_prompt_items_from_session_history as build_guardian_prompt_items;
#[cfg(test)]
use crate::session::session::approval_review_session_impl::build_guardian_review_session_config as build_guardian_review_session_config_for_test;

#[cfg(test)]
#[path = "guardian_internal_tests.rs"]
mod tests;
