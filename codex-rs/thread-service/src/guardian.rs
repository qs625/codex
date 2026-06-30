#![cfg(test)]

use std::time::Duration;

pub(crate) use approval_service::guardian::GUARDIAN_REVIEWER_NAME;
pub(crate) use codex_guardian::GuardianMcpAnnotations;
pub(crate) use codex_guardian::guardian_approval_request_to_json;
pub(crate) use codex_guardian::guardian_policy_prompt;
pub(crate) use codex_guardian::guardian_policy_prompt_with_config;
pub(crate) use crate::session::session::approval_review_session_impl::GuardianReviewSessionReuseKey;
pub(crate) use crate::session::session::approval_review_runtime_impl::GuardianReviewOutcome;
pub(crate) use crate::session::session::approval_review_runtime_impl::record_guardian_denial_for_test;
pub(crate) use crate::session::session::approval_review_runtime_impl::review_approval_request_with_cancel;
pub(crate) const GUARDIAN_REVIEW_TIMEOUT: Duration = Duration::from_secs(90);
#[path = "guardian_internal_tests.rs"]
mod tests;
