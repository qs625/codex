use super::*;
use crate::live_thread_runtime::AppServerLiveThreadHandle;
use futures::future::BoxFuture;
use protocol::protocol::validate_thread_goal_objective;
use state_api::protocol_goal_from_state;
use state_api::state_goal_status_from_protocol;
use state_api::validate_thread_goal_budget;

pub(crate) trait ThreadGoalRuntime: Send + Sync {
    fn live_thread_info(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, protocol::error::Result<LiveThreadInfo>>;

    fn prepare_thread_external_goal_mutation(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, protocol::error::Result<()>>;

    fn apply_thread_external_goal_set(
        &self,
        thread_id: ThreadId,
        external_set: ExternalGoalSet,
    ) -> BoxFuture<'_, protocol::error::Result<()>>;

    fn apply_thread_external_goal_clear(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, protocol::error::Result<()>>;
}

impl<T> ThreadGoalRuntime for T
where
    T: LiveThreadRegistry + Send + Sync,
{
    fn live_thread_info(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, protocol::error::Result<LiveThreadInfo>> {
        Box::pin(LiveThreadRegistry::live_thread_info(self, thread_id))
    }

    fn prepare_thread_external_goal_mutation(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, protocol::error::Result<()>> {
        Box::pin(LiveThreadRegistry::prepare_thread_external_goal_mutation(
            self, thread_id,
        ))
    }

    fn apply_thread_external_goal_set(
        &self,
        thread_id: ThreadId,
        external_set: ExternalGoalSet,
    ) -> BoxFuture<'_, protocol::error::Result<()>> {
        Box::pin(LiveThreadRegistry::apply_thread_external_goal_set(
            self,
            thread_id,
            external_set,
        ))
    }

    fn apply_thread_external_goal_clear(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, protocol::error::Result<()>> {
        Box::pin(LiveThreadRegistry::apply_thread_external_goal_clear(
            self, thread_id,
        ))
    }
}

#[derive(Clone)]
pub(crate) struct ThreadGoalRequestProcessor {
    thread_runtime: Arc<dyn ThreadGoalRuntime>,
    outgoing: Arc<OutgoingMessageSender>,
    config: Arc<Config>,
    thread_state_manager: ThreadStateManager,
    state_db: Option<StateDbHandle>,
}

impl ThreadGoalRequestProcessor {
    pub(crate) fn new<R>(
        thread_runtime: Arc<R>,
        outgoing: Arc<OutgoingMessageSender>,
        config: Arc<Config>,
        thread_state_manager: ThreadStateManager,
        state_db: Option<StateDbHandle>,
    ) -> Self
    where
        R: ThreadGoalRuntime + 'static,
    {
        let thread_runtime: Arc<dyn ThreadGoalRuntime> = thread_runtime;
        Self {
            thread_runtime,
            outgoing,
            config,
            thread_state_manager,
            state_db,
        }
    }

    pub(crate) async fn thread_goal_set(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalSetParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_goal_set_inner(request_id, params)
            .await
            .map(|()| None)
    }

    pub(crate) async fn thread_goal_get(
        &self,
        params: ThreadGoalGetParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_goal_get_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn thread_goal_clear(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalClearParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_goal_clear_inner(request_id, params)
            .await
            .map(|()| None)
    }

    pub(crate) async fn emit_resume_goal_snapshot_and_continue(
        &self,
        thread_id: ThreadId,
        thread: &dyn AppServerLiveThreadHandle,
    ) {
        if !self.config.features.enabled(Feature::Goals) {
            return;
        }
        self.emit_thread_goal_snapshot(thread_id).await;
        // App-server owns resume response and snapshot ordering, so wait until
        // those are sent before letting core start goal continuation.
        if let Err(err) = thread.continue_active_goal_if_idle().await {
            tracing::warn!("failed to continue active goal after resume: {err}");
        }
    }

    pub(crate) async fn pending_resume_goal_state(&self) -> (bool, Option<StateDbHandle>) {
        let emit_thread_goal_update = self.config.features.enabled(Feature::Goals);
        let thread_goal_state_db = if emit_thread_goal_update {
            self.state_db.clone()
        } else {
            None
        };
        (emit_thread_goal_update, thread_goal_state_db)
    }

    async fn thread_goal_set_inner(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalSetParams,
    ) -> Result<(), JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }

        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let state_db = self.state_db_for_materialized_thread(thread_id).await?;
        let live_thread_info = self.thread_runtime.live_thread_info(thread_id).await.ok();
        let rollout_path = match live_thread_info.as_ref() {
            Some(info) => info.rollout_path.clone().ok_or_else(|| {
                invalid_request(format!(
                    "ephemeral thread does not support goals: {thread_id}"
                ))
            })?,
            None => rollout::find_thread_path_by_id_str(
                &self.config.codex_home,
                &thread_id.to_string(),
                self.state_db.as_deref(),
            )
            .await
            .map_err(|err| {
                internal_error(format!("failed to locate thread id {thread_id}: {err}"))
            })?
            .ok_or_else(|| invalid_request(format!("thread not found: {thread_id}")))?,
        };
        reconcile_rollout(
            Some(state_db.as_ref()),
            rollout_path.as_path(),
            self.config.model_provider_id.as_str(),
            /*builder*/ None,
            &[],
            /*archived_only*/ None,
            /*new_thread_memory_mode*/ None,
        )
        .await;

        let listener_command_tx = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let thread_state = thread_state.lock().await;
            thread_state.listener_command_tx()
        };
        let status = params
            .status
            .map(|status| state_goal_status_from_protocol(status.to_core()));
        let objective = params.objective.as_deref().map(str::trim);

        if let Some(objective) = objective {
            validate_thread_goal_objective(objective).map_err(invalid_request)?;
        }
        if objective.is_some() || params.token_budget.is_some() {
            validate_thread_goal_budget(params.token_budget.flatten())
                .map_err(|err| invalid_request(err.to_string()))?;
        }

        if live_thread_info.is_some()
            && let Err(err) = self
                .thread_runtime
                .prepare_thread_external_goal_mutation(thread_id)
                .await
        {
            tracing::warn!("failed to prepare external goal mutation: {err}");
        }

        let (goal, previous_status) = (if let Some(objective) = objective {
            let existing_goal = state_db
                .get_thread_goal(thread_id)
                .await
                .map_err(|err| invalid_request(err.to_string()))?;
            if let Some(goal) = existing_goal.as_ref() {
                let previous_status = ExternalGoalPreviousStatus::from(goal);
                state_db
                    .update_thread_goal(
                        thread_id,
                        state::ThreadGoalUpdate {
                            objective: Some(objective.to_string()),
                            status,
                            token_budget: params.token_budget,
                            expected_goal_id: Some(goal.goal_id.clone()),
                        },
                    )
                    .await
                    .and_then(|goal| {
                        goal.ok_or_else(|| {
                            anyhow::anyhow!(
                                "cannot update goal for thread {thread_id}: no goal exists"
                            )
                        })
                    })
                    .map(|goal| (goal, previous_status))
            } else {
                let previous_status = ExternalGoalPreviousStatus::NewGoal;
                state_db
                    .replace_thread_goal(
                        thread_id,
                        objective,
                        status.unwrap_or(state::ThreadGoalStatus::Active),
                        params.token_budget.flatten(),
                    )
                    .await
                    .map(|goal| (goal, previous_status))
            }
        } else {
            let existing_goal = state_db
                .get_thread_goal(thread_id)
                .await
                .map_err(|err| invalid_request(err.to_string()))?;
            let Some(existing_goal) = existing_goal else {
                return Err(invalid_request(format!(
                    "cannot update goal for thread {thread_id}: no goal exists"
                )));
            };
            let previous_status = ExternalGoalPreviousStatus::from(&existing_goal);
            state_db
                .update_thread_goal(
                    thread_id,
                    state::ThreadGoalUpdate {
                        objective: None,
                        status,
                        token_budget: params.token_budget,
                        expected_goal_id: None,
                    },
                )
                .await
                .and_then(|goal| {
                    goal.ok_or_else(|| {
                        anyhow::anyhow!("cannot update goal for thread {thread_id}: no goal exists")
                    })
                })
                .map(|goal| (goal, previous_status))
        })
        .map_err(|err| invalid_request(err.to_string()))?;
        let external_goal_set = ExternalGoalSet {
            goal: goal.clone(),
            previous_status,
        };
        let goal = api_thread_goal_from_state(goal);
        self.outgoing
            .send_response(
                request_id.clone(),
                ThreadGoalSetResponse { goal: goal.clone() },
            )
            .await;
        self.emit_thread_goal_updated_ordered(thread_id, goal, listener_command_tx)
            .await;
        if live_thread_info.is_some()
            && let Err(err) = self
                .thread_runtime
                .apply_thread_external_goal_set(thread_id, external_goal_set)
                .await
        {
            tracing::warn!("failed to apply external goal set runtime effects: {err}");
        }
        Ok(())
    }

    async fn thread_goal_get_inner(
        &self,
        params: ThreadGoalGetParams,
    ) -> Result<ThreadGoalGetResponse, JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }

        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let state_db = self.state_db_for_materialized_thread(thread_id).await?;
        let goal = state_db
            .get_thread_goal(thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to read thread goal: {err}")))?
            .map(api_thread_goal_from_state);
        Ok(ThreadGoalGetResponse { goal })
    }

    async fn thread_goal_clear_inner(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalClearParams,
    ) -> Result<(), JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }

        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let state_db = self.state_db_for_materialized_thread(thread_id).await?;
        let live_thread_info = self.thread_runtime.live_thread_info(thread_id).await.ok();
        let rollout_path = match live_thread_info.as_ref() {
            Some(info) => info.rollout_path.clone().ok_or_else(|| {
                invalid_request(format!(
                    "ephemeral thread does not support goals: {thread_id}"
                ))
            })?,
            None => rollout::find_thread_path_by_id_str(
                &self.config.codex_home,
                &thread_id.to_string(),
                self.state_db.as_deref(),
            )
            .await
            .map_err(|err| {
                internal_error(format!("failed to locate thread id {thread_id}: {err}"))
            })?
            .ok_or_else(|| invalid_request(format!("thread not found: {thread_id}")))?,
        };
        reconcile_rollout(
            Some(state_db.as_ref()),
            rollout_path.as_path(),
            self.config.model_provider_id.as_str(),
            /*builder*/ None,
            &[],
            /*archived_only*/ None,
            /*new_thread_memory_mode*/ None,
        )
        .await;

        if live_thread_info.is_some()
            && let Err(err) = self
                .thread_runtime
                .prepare_thread_external_goal_mutation(thread_id)
                .await
        {
            tracing::warn!("failed to prepare external goal clear mutation: {err}");
        }

        let listener_command_tx = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let thread_state = thread_state.lock().await;
            thread_state.listener_command_tx()
        };
        let cleared = state_db
            .delete_thread_goal(thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to clear thread goal: {err}")))?;

        if cleared
            && live_thread_info.is_some()
            && let Err(err) = self
                .thread_runtime
                .apply_thread_external_goal_clear(thread_id)
                .await
        {
            tracing::warn!("failed to apply external goal clear runtime effects: {err}");
        }

        self.outgoing
            .send_response(request_id, ThreadGoalClearResponse { cleared })
            .await;
        if cleared {
            self.emit_thread_goal_cleared_ordered(thread_id, listener_command_tx)
                .await;
        }
        Ok(())
    }

    async fn state_db_for_materialized_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<StateDbHandle, JSONRPCErrorError> {
        if let Ok(live_info) = self.thread_runtime.live_thread_info(thread_id).await {
            if live_info.rollout_path.is_none() {
                return Err(invalid_request(format!(
                    "ephemeral thread does not support goals: {thread_id}"
                )));
            }
        } else {
            rollout::find_thread_path_by_id_str(
                &self.config.codex_home,
                &thread_id.to_string(),
                self.state_db.as_deref(),
            )
            .await
            .map_err(|err| {
                internal_error(format!("failed to locate thread id {thread_id}: {err}"))
            })?
            .ok_or_else(|| invalid_request(format!("thread not found: {thread_id}")))?;
        }

        self.state_db
            .clone()
            .ok_or_else(|| internal_error("sqlite state db unavailable for thread goals"))
    }

    async fn emit_thread_goal_snapshot(&self, thread_id: ThreadId) {
        let state_db = match self.state_db_for_materialized_thread(thread_id).await {
            Ok(state_db) => state_db,
            Err(err) => {
                warn!(
                    "failed to open state db before emitting thread goal resume snapshot for {thread_id}: {}",
                    err.message
                );
                return;
            }
        };
        let listener_command_tx = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let thread_state = thread_state.lock().await;
            thread_state.listener_command_tx()
        };
        if let Some(listener_command_tx) = listener_command_tx {
            let command = crate::thread_state::ThreadListenerCommand::EmitThreadGoalSnapshot {
                state_db: state_db.clone(),
            };
            if listener_command_tx.send(command).is_ok() {
                return;
            }
            warn!(
                "failed to enqueue thread goal snapshot for {thread_id}: listener command channel is closed"
            );
        }
        send_thread_goal_snapshot_notification(&self.outgoing, thread_id, &state_db).await;
    }

    async fn emit_thread_goal_updated_ordered(
        &self,
        thread_id: ThreadId,
        goal: ThreadGoal,
        listener_command_tx: Option<tokio::sync::mpsc::UnboundedSender<ThreadListenerCommand>>,
    ) {
        if let Some(listener_command_tx) = listener_command_tx {
            let command = crate::thread_state::ThreadListenerCommand::EmitThreadGoalUpdated {
                goal: goal.clone(),
            };
            if listener_command_tx.send(command).is_ok() {
                return;
            }
            warn!(
                "failed to enqueue thread goal update for {thread_id}: listener command channel is closed"
            );
        }
        self.outgoing
            .send_server_notification(ServerNotification::ThreadGoalUpdated(
                ThreadGoalUpdatedNotification {
                    thread_id: thread_id.to_string(),
                    turn_id: None,
                    goal,
                },
            ))
            .await;
    }

    async fn emit_thread_goal_cleared_ordered(
        &self,
        thread_id: ThreadId,
        listener_command_tx: Option<tokio::sync::mpsc::UnboundedSender<ThreadListenerCommand>>,
    ) {
        if let Some(listener_command_tx) = listener_command_tx {
            let command = crate::thread_state::ThreadListenerCommand::EmitThreadGoalCleared;
            if listener_command_tx.send(command).is_ok() {
                return;
            }
            warn!(
                "failed to enqueue thread goal clear for {thread_id}: listener command channel is closed"
            );
        }
        self.outgoing
            .send_server_notification(ServerNotification::ThreadGoalCleared(
                ThreadGoalClearedNotification {
                    thread_id: thread_id.to_string(),
                },
            ))
            .await;
    }
}

pub(super) fn api_thread_goal_from_state(goal: state::ThreadGoal) -> ThreadGoal {
    protocol_goal_from_state(goal).into()
}

fn parse_thread_id_for_request(thread_id: &str) -> Result<ThreadId, JSONRPCErrorError> {
    ThreadId::from_string(thread_id)
        .map_err(|err| invalid_request(format!("invalid thread id: {err}")))
}
