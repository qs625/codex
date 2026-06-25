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

mod review;
mod review_session;

use std::time::Duration;

pub(crate) use codex_protocol::protocol::GuardianAssessmentOutcome;

pub(crate) use codex_guardian::AUTO_REVIEW_DENIAL_WINDOW_SIZE;
pub(crate) use codex_guardian::AUTO_REVIEW_DENIED_ACTION_APPROVAL_DEVELOPER_PREFIX;
pub(crate) use codex_guardian::GuardianApprovalRequest;
pub(crate) use codex_guardian::GuardianAssessment;
#[cfg(test)]
pub(crate) use codex_guardian::GuardianMcpAnnotations;
pub(crate) use codex_guardian::GuardianNetworkAccessTrigger;
pub(crate) use codex_guardian::GuardianRejection;
pub(crate) use codex_guardian::GuardianRejectionCircuitBreaker;
pub(crate) use codex_guardian::GuardianRejectionCircuitBreakerAction;
#[cfg(test)]
pub(crate) use codex_guardian::guardian_approval_request_to_json;
pub(crate) use review::guardian_rejection_message;
pub(crate) use review::guardian_timeout_message;
pub(crate) use review::is_guardian_reviewer_source;
pub(crate) use review::new_guardian_review_id;
#[cfg(test)]
pub(crate) use review::record_guardian_denial_for_test;
pub(crate) use review::review_approval_request;
#[cfg(test)]
pub(crate) use review::review_approval_request_with_cancel;
pub(crate) use review::routes_approval_to_guardian;
pub(crate) use review::spawn_approval_request_review;
pub(crate) use review_session::GuardianReviewSessionManager;
#[cfg(test)]
pub(crate) use review_session::GuardianReviewSessionReuseKey;

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
use review::GuardianReviewOutcome;
#[cfg(test)]
use review::run_guardian_review_session as run_guardian_review_session_for_test;
#[cfg(test)]
use review_session::build_guardian_prompt_items_from_session_history as build_guardian_prompt_items;
#[cfg(test)]
use review_session::build_guardian_review_session_config as build_guardian_review_session_config_for_test;

#[cfg(test)]
mod tests;
