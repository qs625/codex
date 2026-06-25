use codex_protocol::ThreadId;
use codex_protocol::dynamic_tools::DynamicToolSpec;
#[cfg(not(any(test, feature = "test-support")))]
use codex_state_api::SharedStateDbRuntime;
use codex_state_api::StateDbRuntime;
use codex_state_api::ThreadMetadata;
use codex_utils_path::normalize_for_path_comparison;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;
use tracing::warn;

#[cfg(test)]
use crate::config::Config;

#[cfg(any(test, feature = "test-support"))]
pub type StateDbHandle = Arc<codex_state::StateRuntime>;

#[cfg(not(any(test, feature = "test-support")))]
pub type StateDbHandle = SharedStateDbRuntime;

#[cfg(test)]
pub async fn init_state_db(config: &Config) -> Option<StateDbHandle> {
    match codex_state::StateRuntime::init(
        config.sqlite_home.clone(),
        config.model_provider_id.clone(),
    )
    .await
    {
        Ok(runtime) => {
            let runtime: StateDbHandle = runtime;
            Some(runtime)
        }
        Err(err) => {
            warn!("state db init failed during test setup: {err}");
            None
        }
    }
}

pub async fn get_dynamic_tools<T>(
    context: Option<&T>,
    thread_id: ThreadId,
    stage: &str,
) -> Option<Vec<DynamicToolSpec>>
where
    T: StateDbRuntime + ?Sized,
{
    let ctx = context?;
    match ctx.get_dynamic_tools(thread_id).await {
        Ok(tools) => tools,
        Err(err) => {
            warn!("state db get_dynamic_tools failed during {stage}: {err}");
            None
        }
    }
}

pub async fn mark_thread_memory_mode_polluted<T>(
    context: Option<&T>,
    thread_id: ThreadId,
    stage: &str,
) where
    T: StateDbRuntime + ?Sized,
{
    let Some(ctx) = context else {
        return;
    };
    if let Err(err) = ctx.mark_thread_memory_mode_polluted(thread_id).await {
        warn!("state db mark_thread_memory_mode_polluted failed during {stage}: {err}");
    }
}

pub async fn insert_thread_metadata_if_absent<T>(
    context: &T,
    mut metadata: ThreadMetadata,
    stage: &str,
) -> anyhow::Result<bool>
where
    T: StateDbRuntime + ?Sized,
{
    metadata.cwd =
        normalize_for_path_comparison(&metadata.cwd).unwrap_or_else(|_| metadata.cwd.clone());
    let thread_id = metadata.id;
    context
        .insert_thread_if_absent(metadata)
        .await
        .map_err(|err| {
            warn!("state db insert_thread_if_absent failed during {stage} for {thread_id}: {err}");
            err
        })
}

pub async fn record_stage1_output_usage<T>(
    context: Option<&T>,
    thread_ids: &[ThreadId],
    stage: &str,
) where
    T: StateDbRuntime + ?Sized,
{
    let Some(ctx) = context else {
        return;
    };
    if let Err(err) = ctx.record_stage1_output_usage(thread_ids).await {
        warn!("state db record_stage1_output_usage failed during {stage}: {err}");
    }
}
