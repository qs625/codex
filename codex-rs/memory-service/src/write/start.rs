use crate::extensions::seed_extension_instructions;
use crate::guard;
use crate::memory_root;
use crate::metrics::MEMORY_STARTUP;
use crate::phase1;
use crate::phase2;
use crate::runtime::MemoryStartupContext;
use codex_login::AuthManager;
use memory_service_api::MemoryStartupRuntime;
use memory_service_api::MemoryStartupSettings;
use protocol::ThreadId;
use std::sync::Arc;
use tracing::warn;

/// Starts the asynchronous startup memory pipeline for an eligible root session.
///
/// The pipeline is skipped for ephemeral sessions, disabled feature flags, and
/// subagent sessions.
pub fn start_memories_startup_task(
    runtime: Arc<dyn MemoryStartupRuntime>,
    auth_manager: Arc<AuthManager>,
    thread_id: ThreadId,
    settings: Arc<MemoryStartupSettings>,
) {
    if settings.ephemeral
        || !settings.memory_tool_enabled
        || settings.session_source.is_non_root_agent()
    {
        return;
    }

    let context = Arc::new(MemoryStartupContext::new(thread_id, runtime));

    if context.state_db().is_none() {
        warn!("state db unavailable for memories startup pipeline; skipping");
        return;
    }

    tokio::spawn(async move {
        let root = memory_root(&settings.codex_home);
        if let Err(err) = seed_extension_instructions(&root).await {
            warn!("failed seeding memory extension instructions: {err}");
        }

        // Clean memories to make preserve DB size. This does not consume tokens so can be
        // done before the quota check.
        phase1::prune(context.as_ref(), settings.as_ref()).await;

        if !guard::rate_limits_ok(&auth_manager, settings.as_ref()).await {
            context.counter(
                MEMORY_STARTUP,
                /*inc*/ 1,
                &[("status", "skipped_rate_limit")],
            );
            return;
        }

        // Run phase 1.
        phase1::run(Arc::clone(&context), Arc::clone(&settings)).await;
        // Run phase 2.
        phase2::run(context, settings).await;
    });
}
