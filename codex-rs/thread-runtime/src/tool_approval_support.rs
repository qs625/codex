use crate::state::SessionServices;
use codex_protocol::error::CodexErr;
use codex_protocol::protocol::ReviewDecision;
use futures::Future;
use serde::Serialize;

pub(crate) use codex_thread_api::PermissionRequestPayload;

#[derive(Debug)]
pub(crate) enum ToolError {
    Rejected(String),
    Codex(CodexErr),
}

pub(crate) fn permission_request_hook_payload(
    payload: PermissionRequestPayload,
) -> codex_hooks::PermissionRequestHookPayload {
    codex_hooks::PermissionRequestHookPayload {
        tool_name: payload.tool_name.name().to_string(),
        matcher_aliases: payload.tool_name.matcher_aliases().to_vec(),
        tool_input: payload.tool_input,
    }
}

/// Takes a vector of approval keys and returns a ReviewDecision.
/// There will be one key in most cases, but apply_patch can modify multiple files at once.
///
/// - If all keys are already approved for session, we skip prompting.
/// - If the user approves for session, we store the decision for each key individually
///   so future requests touching any subset can also skip prompting.
pub(crate) async fn with_cached_approval<K, F, Fut>(
    services: &SessionServices,
    tool_name: &str,
    keys: Vec<K>,
    fetch: F,
) -> ReviewDecision
where
    K: Serialize,
    F: FnOnce() -> Fut,
    Fut: Future<Output = ReviewDecision>,
{
    if keys.is_empty() {
        return fetch().await;
    }

    let already_approved = {
        let store = services.tool_approvals.lock().await;
        keys.iter()
            .all(|key| matches!(store.get(key), Some(ReviewDecision::ApprovedForSession)))
    };

    if already_approved {
        return ReviewDecision::ApprovedForSession;
    }

    let decision = fetch().await;

    services.session_telemetry.counter(
        "codex.approval.requested",
        /*inc*/ 1,
        &[
            ("tool", tool_name),
            ("approved", decision.to_opaque_string()),
        ],
    );

    if matches!(decision, ReviewDecision::ApprovedForSession) {
        let mut store = services.tool_approvals.lock().await;
        for key in keys {
            store.put(key, ReviewDecision::ApprovedForSession);
        }
    }

    decision
}
