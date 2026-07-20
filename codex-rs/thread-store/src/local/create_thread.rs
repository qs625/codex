use super::LocalThreadStore;
use crate::CreateThreadParams;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;
use protocol::protocol::ThreadMemoryMode;
use rollout::RolloutConfig;
use rollout::RolloutRecorder;
use rollout::RolloutRecorderParams;

pub(super) async fn create_thread(
    store: &LocalThreadStore,
    params: CreateThreadParams,
) -> ThreadStoreResult<RolloutRecorder> {
    let cwd = params
        .metadata
        .cwd
        .clone()
        .ok_or_else(|| ThreadStoreError::InvalidRequest {
            message: "local thread store requires a cwd".to_string(),
        })?;
    let config = RolloutConfig {
        codex_home: store.config.codex_home.clone(),
        sqlite_home: store.config.sqlite_home.clone(),
        cwd,
        model_provider_id: params.metadata.model_provider.clone(),
        generate_memories: matches!(params.metadata.memory_mode, ThreadMemoryMode::Enabled),
    };
    let recorder = RolloutRecorder::new(
        &config,
        RolloutRecorderParams::new(
            params.thread_id,
            params.forked_from_id,
            params.source,
            params.thread_source,
            params.metadata.root_agent_role,
            params.metadata.root_agent_path,
            params.base_instructions,
            params.dynamic_tools,
        ),
    )
    .await
    .map_err(|err| ThreadStoreError::Internal {
        message: format!("failed to initialize local thread recorder: {err}"),
    })?;

    Ok(recorder)
}
