//! Protocol-neutral MCP elicitation reviewer API.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use protocol::approvals::ElicitationRequest;
use protocol::mcp::RequestId;

use crate::ElicitationResponse;

pub type ElicitationReviewResult = anyhow::Result<Option<ElicitationResponse>>;

pub type ElicitationReviewFuture =
    Pin<Box<dyn Future<Output = ElicitationReviewResult> + Send + 'static>>;

#[derive(Debug, Clone)]
pub struct ElicitationReviewRequest {
    pub server_name: String,
    pub request_id: RequestId,
    pub elicitation: ElicitationRequest,
}

/// Reviews a server-initiated MCP elicitation before it is shown to the user.
///
/// Implementations should return `Ok(Some(response))` when the reviewer has a
/// definitive accept/decline/cancel decision, and `Ok(None)` when normal MCP
/// elicitation handling should continue.
pub trait ElicitationReviewer: Send + Sync {
    fn review(&self, request: ElicitationReviewRequest) -> ElicitationReviewFuture;
}

pub type ElicitationReviewerHandle = Arc<dyn ElicitationReviewer>;
