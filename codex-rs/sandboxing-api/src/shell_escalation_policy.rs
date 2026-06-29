use std::future::Future;
use std::pin::Pin;

use codex_utils_absolute_path::AbsolutePathBuf;

use crate::shell_escalation::EscalationDecision;

/// Future returned by [`EscalationPolicy::determine_action`].
pub type EscalationPolicyFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<EscalationDecision>> + Send + 'a>>;

/// Decides what action to take in response to an execve request from a client.
pub trait EscalationPolicy: Send + Sync {
    fn determine_action<'a>(
        &'a self,
        file: &'a AbsolutePathBuf,
        argv: &'a [String],
        workdir: &'a AbsolutePathBuf,
    ) -> EscalationPolicyFuture<'a>;
}
