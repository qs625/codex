use std::sync::Arc;
use std::future::Future;

use super::SessionTask;
use super::SessionTaskContext;
use codex_extension_api::ExtensionData;
use crate::session::session::Session;
use crate::session::turn::run_turn;
use crate::session::turn_context::TurnContext;
use crate::state::TaskKind;
use protocol::user_input::UserInput;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Default)]
pub(crate) struct CompactTask;

async fn transition_active_task_to_regular(
    session: &SessionTaskContext,
    turn_context: &TurnContext,
) -> bool {
    let sess = session.clone_session();
    let mut active_turn = sess.active_turn.lock().await;
    let Some(active_turn) = active_turn.as_mut() else {
        return false;
    };

    active_turn.update_task_metadata(
        &turn_context.sub_id,
        TaskKind::Regular,
        /*records_turn_token_usage_on_span*/ true,
    )
}

pub(crate) async fn continue_compact_turn_after_success<F, Fut>(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    cancellation_token: CancellationToken,
    mut run_continuation: F,
) -> Option<String>
where
    F: FnMut(Arc<Session>, Arc<TurnContext>, Arc<ExtensionData>, CancellationToken) -> Fut,
    Fut: Future<Output = Option<String>>,
{
    if !transition_active_task_to_regular(session.as_ref(), ctx.as_ref()).await {
        return None;
    }

    let sess = session.clone_session();
    let turn_extension_data = session.turn_extension_data();
    let mut last_agent_message = None;
    while sess.has_pending_input().await {
        last_agent_message = run_continuation(
            Arc::clone(&sess),
            Arc::clone(&ctx),
            Arc::clone(&turn_extension_data),
            cancellation_token.child_token(),
        )
        .await;
    }

    last_agent_message
}

impl SessionTask for CompactTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Compact
    }

    fn span_name(&self) -> &'static str {
        "session_task.compact"
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        let sess = session.clone_session();
        sess.services.session_telemetry.counter(
            "codex.task.compact",
            /*inc*/ 1,
            &[("type", "local")],
        );
        if crate::compact::run_compact_task(Arc::clone(&sess), Arc::clone(&ctx), input)
            .await
            .is_err()
        {
            return None;
        }

        continue_compact_turn_after_success(
            session,
            ctx,
            cancellation_token,
            |sess, ctx, turn_extension_data, cancellation_token| async move {
                sess.set_server_reasoning_included(/*included*/ false).await;
                run_turn(
                    sess,
                    ctx,
                    turn_extension_data,
                    Vec::new(),
                    /*prewarmed_client_session*/ None,
                    cancellation_token,
                )
                .await
            },
        )
        .await
    }
}
