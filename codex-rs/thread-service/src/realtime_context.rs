use crate::session::session::Session;
use thread_store_api::ListThreadsParams;
use thread_store_api::SortDirection;
use thread_store_api::StoredThread;
use thread_store_api::ThreadSortKey;
use tracing::warn;

pub(crate) use codex_realtime::REALTIME_TURN_TOKEN_BUDGET;
pub(crate) use codex_realtime::truncate_realtime_text_to_token_budget;

const MAX_RECENT_THREADS: usize = 40;

pub(crate) async fn build_realtime_startup_context(
    sess: &Session,
    budget_tokens: usize,
) -> Option<String> {
    let config = sess.get_config().await;
    let cwd = config.cwd.clone();
    let history = sess.clone_history().await;
    let recent_threads = load_recent_threads(sess).await;

    codex_realtime::build_realtime_startup_context(
        &cwd,
        history.raw_items(),
        &recent_threads,
        budget_tokens,
    )
    .await
}

async fn load_recent_threads(sess: &Session) -> Vec<StoredThread> {
    match sess
        .services
        .thread_store
        .list_threads(ListThreadsParams {
            page_size: MAX_RECENT_THREADS,
            cursor: None,
            sort_key: ThreadSortKey::UpdatedAt,
            sort_direction: SortDirection::Desc,
            allowed_sources: Vec::new(),
            model_providers: None,
            cwd_filters: None,
            archived: false,
            search_term: None,
            use_state_db_only: false,
        })
        .await
    {
        Ok(page) => page.items,
        Err(err) => {
            warn!("failed to load realtime startup threads from thread store: {err}");
            Vec::new()
        }
    }
}
