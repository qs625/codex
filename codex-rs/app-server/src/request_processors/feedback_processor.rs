use super::*;
use crate::live_thread_runtime::AppServerLiveThreadFeedbackRuntime;
use crate::live_thread_runtime::AppServerLiveThreadInspectionRuntime;
use codex_feedback::FeedbackAttachment;
use std::collections::BTreeMap;
use thread_service_api::ThreadAgentDirectoryRuntime;

#[derive(Clone)]
pub(crate) struct FeedbackRequestProcessor {
    auth_manager: Arc<AuthManager>,
    thread_runtime: Arc<dyn AppServerLiveThreadFeedbackRuntime>,
    live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime>,
    config: Arc<Config>,
    feedback: CodexFeedback,
    log_db: Option<LogDbLayer>,
    state_db: Option<StateDbHandle>,
}

impl FeedbackRequestProcessor {
    pub(crate) fn new<R>(
        auth_manager: Arc<AuthManager>,
        thread_runtime: Arc<R>,
        config: Arc<Config>,
        feedback: CodexFeedback,
        log_db: Option<LogDbLayer>,
        state_db: Option<StateDbHandle>,
    ) -> Self
    where
        R: AppServerLiveThreadFeedbackRuntime
            + AppServerLiveThreadInspectionRuntime
            + ThreadAgentDirectoryRuntime
            + 'static,
    {
        let live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime> =
            thread_runtime.clone();
        let thread_runtime: Arc<dyn AppServerLiveThreadFeedbackRuntime> = thread_runtime;
        Self {
            auth_manager,
            thread_runtime,
            live_thread_inspection,
            config,
            feedback,
            log_db,
            state_db,
        }
    }

    pub(crate) async fn feedback_upload(
        &self,
        params: FeedbackUploadParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.upload_feedback_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    async fn upload_feedback_response(
        &self,
        params: FeedbackUploadParams,
    ) -> Result<FeedbackUploadResponse, JSONRPCErrorError> {
        if !self.config.feedback_enabled {
            return Err(invalid_request(
                "sending feedback is disabled by configuration",
            ));
        }

        let FeedbackUploadParams {
            classification,
            reason,
            thread_id,
            include_logs,
            extra_log_files,
            tags,
        } = params;
        let mut upload_tags = tags.unwrap_or_default();

        let conversation_id = match thread_id.as_deref() {
            Some(thread_id) => match ThreadId::from_string(thread_id) {
                Ok(conversation_id) => Some(conversation_id),
                Err(err) => return Err(invalid_request(format!("invalid thread id: {err}"))),
            },
            None => None,
        };

        if let Some(chatgpt_user_id) = self
            .auth_manager
            .auth_cached()
            .and_then(|auth| auth.get_chatgpt_user_id())
        {
            tracing::info!(target: "feedback_tags", chatgpt_user_id);
        }
        if let Some(account_id) = self
            .auth_manager
            .auth_cached()
            .and_then(|auth| auth.get_account_id())
        {
            tracing::info!(target: "feedback_tags", account_id);
        }
        let snapshot = self.feedback.snapshot(conversation_id);
        let thread_id = snapshot.thread_id.clone();
        let log_plan = self
            .feedback_log_upload_plan(conversation_id, include_logs)
            .await;
        let attachment_paths = self
            .feedback_attachment_paths(
                conversation_id,
                include_logs,
                &log_plan,
                extra_log_files.unwrap_or_default(),
            )
            .await;
        let extra_attachments = self
            .feedback_extra_attachments(include_logs, &mut upload_tags)
            .await;

        let session_source = self.thread_runtime.session_source();

        let upload_result = tokio::task::spawn_blocking(move || {
            let tags = (!upload_tags.is_empty()).then_some(&upload_tags);
            snapshot.upload_feedback(FeedbackUploadOptions {
                classification: &classification,
                reason: reason.as_deref(),
                tags,
                include_logs,
                extra_attachments: &extra_attachments,
                extra_attachment_paths: &attachment_paths,
                session_source: Some(session_source),
                logs_override: log_plan.sqlite_feedback_logs,
            })
        })
        .await;

        let upload_result = match upload_result {
            Ok(result) => result,
            Err(join_err) => {
                return Err(internal_error(format!(
                    "failed to upload feedback: {join_err}"
                )));
            }
        };

        upload_result.map_err(|err| internal_error(format!("failed to upload feedback: {err}")))?;
        Ok(FeedbackUploadResponse { thread_id })
    }

    async fn feedback_log_upload_plan(
        &self,
        conversation_id: Option<ThreadId>,
        include_logs: bool,
    ) -> FeedbackLogUploadPlan {
        if !include_logs {
            return FeedbackLogUploadPlan::default();
        }
        if let Some(log_db) = self.log_db.as_ref() {
            log_db.flush().await;
        }
        let state_db_ctx = self.state_db.clone();
        let feedback_thread_ids = self.feedback_thread_ids(conversation_id).await;
        let sqlite_feedback_logs =
            query_sqlite_feedback_logs(state_db_ctx.as_ref(), &feedback_thread_ids).await;
        FeedbackLogUploadPlan {
            feedback_thread_ids,
            sqlite_feedback_logs,
            state_db_ctx,
        }
    }

    async fn feedback_thread_ids(&self, conversation_id: Option<ThreadId>) -> Vec<ThreadId> {
        let Some(conversation_id) = conversation_id else {
            return Vec::new();
        };
        match self
            .thread_runtime
            .list_agent_subtree_thread_ids(conversation_id)
            .await
        {
            Ok(thread_ids) => thread_ids,
            Err(err) => {
                warn!("failed to list feedback subtree for thread_id={conversation_id}: {err}");
                vec![conversation_id]
            }
        }
    }

    async fn feedback_attachment_paths(
        &self,
        conversation_id: Option<ThreadId>,
        include_logs: bool,
        log_plan: &FeedbackLogUploadPlan,
        extra_log_files: Vec<PathBuf>,
    ) -> Vec<FeedbackAttachmentPath> {
        let mut builder = FeedbackAttachmentPathBuilder::default();
        if include_logs {
            for feedback_thread_id in &log_plan.feedback_thread_ids {
                if let Some(rollout_path) = self
                    .resolve_rollout_path(*feedback_thread_id, log_plan.state_db_ctx.as_ref())
                    .await
                {
                    builder.push_rollout(rollout_path);
                }
            }
            if let Some(conversation_id) = conversation_id
                && let Some(guardian_rollout_path) = self
                    .thread_runtime
                    .thread_guardian_trunk_rollout_path(conversation_id)
                    .await
                    .ok()
                    .flatten()
            {
                builder.push_guardian_rollout(conversation_id, guardian_rollout_path);
            }
        }
        builder.extend_extra_logs(extra_log_files);
        builder.into_paths()
    }

    async fn feedback_extra_attachments(
        &self,
        include_logs: bool,
        upload_tags: &mut BTreeMap<String, String>,
    ) -> Vec<FeedbackAttachment> {
        if !include_logs {
            return Vec::new();
        }
        let Some(doctor_report) =
            super::feedback_doctor_report::doctor_feedback_report(&self.config).await
        else {
            return Vec::new();
        };
        for (key, value) in doctor_report.tags {
            upload_tags.entry(key).or_insert(value);
        }
        vec![doctor_report.attachment]
    }

    async fn resolve_rollout_path(
        &self,
        conversation_id: ThreadId,
        state_db_ctx: Option<&StateDbHandle>,
    ) -> Option<PathBuf> {
        if let Ok(live_info) = self
            .live_thread_inspection
            .live_thread_info(conversation_id)
            .await
            && let Some(rollout_path) = live_info.rollout_path
        {
            return Some(rollout_path);
        }

        let state_db_ctx = state_db_ctx?;
        state_db_ctx
            .find_rollout_path_by_id(conversation_id, /*archived_only*/ None)
            .await
            .unwrap_or_else(|err| {
                warn!("failed to resolve rollout path for thread_id={conversation_id}: {err}");
                None
            })
    }
}

fn auto_review_rollout_filename(thread_id: ThreadId) -> String {
    format!("auto-review-rollout-{thread_id}.jsonl")
}

#[derive(Default)]
struct FeedbackLogUploadPlan {
    feedback_thread_ids: Vec<ThreadId>,
    sqlite_feedback_logs: Option<Vec<u8>>,
    state_db_ctx: Option<StateDbHandle>,
}

async fn query_sqlite_feedback_logs(
    state_db_ctx: Option<&StateDbHandle>,
    feedback_thread_ids: &[ThreadId],
) -> Option<Vec<u8>> {
    let state_db_ctx = state_db_ctx?;
    if feedback_thread_ids.is_empty() {
        return None;
    }
    let thread_id_texts = feedback_thread_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let thread_id_refs = thread_id_texts
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    match state_db_ctx
        .query_feedback_logs_for_threads(&thread_id_refs)
        .await
    {
        Ok(logs) if logs.is_empty() => None,
        Ok(logs) => Some(logs),
        Err(err) => {
            let thread_ids = thread_id_texts.join(", ");
            warn!("failed to query feedback logs from sqlite for thread_ids=[{thread_ids}]: {err}");
            None
        }
    }
}

#[derive(Default)]
struct FeedbackAttachmentPathBuilder {
    paths: Vec<FeedbackAttachmentPath>,
    seen_paths: HashSet<PathBuf>,
}

impl FeedbackAttachmentPathBuilder {
    fn push_rollout(&mut self, path: PathBuf) {
        self.push(path, None);
    }

    fn push_guardian_rollout(&mut self, thread_id: ThreadId, path: PathBuf) {
        self.push(path, Some(auto_review_rollout_filename(thread_id)));
    }

    fn extend_extra_logs(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            self.push(path, None);
        }
    }

    fn push(&mut self, path: PathBuf, attachment_filename_override: Option<String>) {
        if self.seen_paths.insert(path.clone()) {
            self.paths.push(FeedbackAttachmentPath {
                path,
                attachment_filename_override,
            });
        }
    }

    fn into_paths(self) -> Vec<FeedbackAttachmentPath> {
        self.paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_attachment_path_builder_preserves_first_seen_path_order() {
        let thread_id = test_thread_id();
        let rollout_path = PathBuf::from("/tmp/thread.jsonl");
        let guardian_path = PathBuf::from("/tmp/guardian.jsonl");
        let extra_path = PathBuf::from("/tmp/extra.log");
        let mut builder = FeedbackAttachmentPathBuilder::default();

        builder.push_rollout(rollout_path.clone());
        builder.push_guardian_rollout(thread_id, guardian_path.clone());
        builder.extend_extra_logs(vec![
            rollout_path.clone(),
            extra_path.clone(),
            guardian_path.clone(),
        ]);

        let paths = builder.into_paths();

        assert_eq!(paths.len(), 3);
        assert_eq!(paths[0].path, rollout_path);
        assert_eq!(paths[0].attachment_filename_override, None);
        assert_eq!(paths[1].path, guardian_path);
        assert_eq!(
            paths[1].attachment_filename_override,
            Some(format!("auto-review-rollout-{thread_id}.jsonl"))
        );
        assert_eq!(paths[2].path, extra_path);
        assert_eq!(paths[2].attachment_filename_override, None);
    }

    #[test]
    fn feedback_attachment_path_builder_keeps_rollout_when_guardian_duplicates_path() {
        let thread_id = test_thread_id();
        let shared_path = PathBuf::from("/tmp/shared.jsonl");
        let mut builder = FeedbackAttachmentPathBuilder::default();

        builder.push_rollout(shared_path.clone());
        builder.push_guardian_rollout(thread_id, shared_path.clone());

        let paths = builder.into_paths();

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].path, shared_path);
        assert_eq!(paths[0].attachment_filename_override, None);
    }

    fn test_thread_id() -> ThreadId {
        ThreadId::from_string("00000000-0000-4000-8000-000000000128").expect("valid thread id")
    }
}
