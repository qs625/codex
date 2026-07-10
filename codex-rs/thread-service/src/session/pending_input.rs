use super::session::ThreadWaitEventSnapshot;
use super::session::ThreadWaitSource;
use super::*;
use tokio::time::Instant;

impl Session {
    pub(crate) async fn sync_mailbox_pending_buffer(&self) {
        self.mailbox_rx.lock().await.drain_incoming_to_pending();
    }

    pub(crate) async fn has_thread_pending_work_locked(&self) -> bool {
        if !self.idle_pending_input.lock().await.is_empty() {
            return true;
        }
        self.mailbox_rx.lock().await.has_pending_trigger_turn()
    }

    pub(crate) async fn has_pending_turn_input_locked(&self) -> bool {
        if !self.idle_pending_input.lock().await.is_empty() {
            return true;
        }
        self.mailbox_rx.lock().await.has_pending()
    }

    /// Inject additional user input into the currently active turn.
    ///
    /// Returns the active turn id when accepted.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn steer_input(
        &self,
        input: Vec<UserInput>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<String, SteerInputError> {
        let mut active = self.active_turn.lock().await;
        let Some(active_turn) = active.as_mut() else {
            return validate_steer_input(input, expected_turn_id, None)
                .map(|validated| validated.active_turn_id);
        };

        let Some((active_turn_id, active_task)) = active_turn.tasks.first() else {
            return validate_steer_input(input, expected_turn_id, None)
                .map(|validated| validated.active_turn_id);
        };

        let active_turn_id = active_turn_id.clone();
        let active_task_kind = active_task.kind;
        let active_turn_context = Arc::clone(&active_task.turn_context);
        let validated = validate_steer_input(
            input,
            expected_turn_id,
            Some(ActiveSteerTurn {
                turn_id: &active_turn_id,
                task_kind: steerable_task_kind(active_task_kind),
            }),
        )?;

        if let Some(responsesapi_client_metadata) = responsesapi_client_metadata {
            active_turn_context
                .turn_metadata_state
                .set_responsesapi_client_metadata(responsesapi_client_metadata);
        }

        let mut turn_state = active_turn.turn_state.lock().await;
        turn_state.push_pending_input(PendingInputItem::from(
            codex_model_input::response_input_item_from_user_input(validated.input),
        ));
        turn_state.accept_async_input_for_current_turn();
        self.note_thread_wait_event(ThreadWaitSource::UserInput);
        Ok(validated.active_turn_id)
    }

    pub(crate) async fn queue_user_input_for_next_turn_if_finishing(
        &self,
        input: Vec<UserInput>,
    ) -> Result<(), Vec<UserInput>> {
        let _scheduler = self.scheduler.lock().await;
        let active = self.active_turn.lock().await;
        let Some(active_turn) = active.as_ref() else {
            return Err(input);
        };
        if !active_turn.tasks.is_empty() {
            return Err(input);
        }
        if active_turn
            .turn_state
            .lock()
            .await
            .accepts_async_input_for_current_turn()
        {
            return Err(input);
        }
        self.idle_pending_input
            .lock()
            .await
            .push(PendingInputItem::from(
                codex_model_input::response_input_item_from_user_input(input),
            ));
        self.note_thread_wait_event(ThreadWaitSource::UserInput);
        Ok(())
    }

    /// Returns the input if there was no task running to inject into.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn inject_hook_inspectable_items(
        &self,
        input: Vec<ResponseInputItem>,
    ) -> Result<(), Vec<ResponseInputItem>> {
        let mut active = self.active_turn.lock().await;
        match active.as_mut() {
            Some(at) => {
                let mut ts = at.turn_state.lock().await;
                if at.tasks.is_empty() || !ts.accepts_async_input_for_current_turn() {
                    return Err(input);
                }
                for item in input {
                    ts.push_pending_input(PendingInputItem::from(item));
                }
                self.note_thread_wait_event(ThreadWaitSource::AsyncInput);
                Ok(())
            }
            None => Err(input),
        }
    }

    pub(crate) async fn record_memory_citation_for_turn(&self, sub_id: &str) {
        let turn_state = self.turn_state_for_sub_id(sub_id).await;
        let Some(turn_state) = turn_state else {
            return;
        };
        turn_state.lock().await.has_memory_citation = true;
    }

    pub(crate) async fn defer_async_input_to_next_turn(&self, sub_id: &str) {
        let turn_state = self.turn_state_for_sub_id(sub_id).await;
        let Some(turn_state) = turn_state else {
            return;
        };
        turn_state.lock().await.defer_async_input_to_next_turn();
    }

    pub(crate) async fn accept_async_input_for_current_turn(&self, sub_id: &str) {
        let turn_state = self.turn_state_for_sub_id(sub_id).await;
        let Some(turn_state) = turn_state else {
            return;
        };
        turn_state.lock().await.accept_async_input_for_current_turn();
    }

    async fn turn_state_for_sub_id(
        &self,
        sub_id: &str,
    ) -> Option<Arc<tokio::sync::Mutex<crate::state::TurnState>>> {
        let active = self.active_turn.lock().await;
        active.as_ref().and_then(|active_turn| {
            active_turn
                .tasks
                .contains_key(sub_id)
                .then(|| Arc::clone(&active_turn.turn_state))
        })
    }

    pub(crate) fn subscribe_thread_wait_events(&self) -> watch::Receiver<ThreadWaitEventSnapshot> {
        self.thread_wait_events.subscribe()
    }

    pub(crate) fn note_thread_wait_event(&self, source: ThreadWaitSource) {
        let current = *self.thread_wait_events.borrow();
        self.thread_wait_events
            .send_replace(ThreadWaitEventSnapshot {
                seq: current.seq + 1,
                source: Some(source),
            });
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "pending input routing and active turn checks must remain atomic"
    )]
    async fn route_thread_pending_input(&self, input: PendingInputItem) -> bool {
        let _scheduler = self.scheduler.lock().await;
        let should_start_turn = input.trigger_turn();
        let mut active = self.active_turn.lock().await;
        if let Some(active_turn) = active.as_mut() {
            let mut turn_state = active_turn.turn_state.lock().await;
            if turn_state.accepts_async_input_for_current_turn() {
                turn_state.push_pending_input(input);
            } else {
                self.mailbox.send(input);
                drop(turn_state);
                drop(active);
                self.sync_mailbox_pending_buffer().await;
            }
            return false;
        }

        self.mailbox.send(input);
        drop(active);
        self.sync_mailbox_pending_buffer().await;
        should_start_turn
    }

    pub(crate) async fn thread_wait_current_window(
        &self,
        initial_timeout_ms: i64,
        hard_cap_timeout_ms: i64,
    ) -> Duration {
        let initial_window = duration_from_config_ms(initial_timeout_ms);
        let max_window = duration_from_config_ms(hard_cap_timeout_ms);
        self.thread_wait_backoff
            .lock()
            .await
            .current_window(initial_window, max_window)
    }

    pub(crate) async fn advance_thread_wait_backoff(
        &self,
        initial_timeout_ms: i64,
        hard_cap_timeout_ms: i64,
    ) {
        let initial_window = duration_from_config_ms(initial_timeout_ms);
        let max_window = duration_from_config_ms(hard_cap_timeout_ms);
        self.thread_wait_backoff
            .lock()
            .await
            .advance_after_timeout(initial_window, max_window);
    }

    pub(crate) async fn reset_thread_wait_backoff(&self) {
        self.thread_wait_backoff.lock().await.reset_after_event();
    }

    pub(crate) async fn enqueue_mailbox_communication(
        &self,
        communication: InterAgentCommunication,
    ) -> bool {
        let source = thread_wait_source_for_communication(&communication);
        let should_start_turn = self
            .route_thread_pending_input(PendingInputItem::from(communication))
            .await;
        self.note_thread_wait_event(source);
        should_start_turn
    }

    pub(crate) async fn enqueue_async_input(&self, input: PendingInputItem) -> bool {
        let source = thread_wait_source_for_pending_input_item(&input);
        let should_start_turn = self.route_thread_pending_input(input).await;
        self.note_thread_wait_event(source);
        should_start_turn
    }

    pub(crate) async fn has_trigger_turn_mailbox_items(&self) -> bool {
        let _scheduler = self.scheduler.lock().await;
        self.sync_mailbox_pending_buffer().await;
        self.mailbox_rx.lock().await.has_pending_trigger_turn()
    }

    pub(crate) async fn has_pending_mailbox_items(&self) -> bool {
        let _scheduler = self.scheduler.lock().await;
        self.sync_mailbox_pending_buffer().await;
        self.mailbox_rx.lock().await.has_pending()
    }

    pub(crate) async fn has_thread_pending_work(&self) -> bool {
        let _scheduler = self.scheduler.lock().await;
        self.sync_mailbox_pending_buffer().await;
        self.has_thread_pending_work_locked().await
    }

    pub(crate) async fn has_thread_pending_work_excluding_active_turn(&self) -> bool {
        let _scheduler = self.scheduler.lock().await;
        self.sync_mailbox_pending_buffer().await;
        self.has_thread_pending_work_locked().await
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state reads must remain atomic"
    )]
    pub(crate) async fn find_pending_input<F, R>(&self, mut f: F) -> Option<R>
    where
        F: FnMut(&PendingInputItem) -> Option<R>,
    {
        let accepts_async_input_for_current_turn = {
            let active = self.active_turn.lock().await;
            if let Some(at) = active.as_ref() {
                let ts = at.turn_state.lock().await;
                if let Some(found) = ts.pending_input().iter().find_map(&mut f) {
                    return Some(found);
                }
                ts.accepts_async_input_for_current_turn()
            } else {
                true
            }
        };
        {
            let idle_pending_input = self.idle_pending_input.lock().await;
            if let Some(found) = idle_pending_input.iter().find_map(&mut f) {
                return Some(found);
            }
        }
        if !accepts_async_input_for_current_turn {
            return None;
        }
        let _scheduler = self.scheduler.lock().await;
        self.sync_mailbox_pending_buffer().await;
        let mut mailbox_rx = self.mailbox_rx.lock().await;
        mailbox_rx.pending().find_map(f)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and pending input updates must remain atomic"
    )]
    pub(crate) async fn clear_child_completion_pending_input(
        &self,
        child_thread_id: ThreadId,
    ) -> usize {
        let _scheduler = self.scheduler.lock().await;
        let mut removed = Vec::new();
        {
            let mut active = self.active_turn.lock().await;
            if let Some(at) = active.as_mut() {
                let mut ts = at.turn_state.lock().await;
                removed.extend(ts.extract_pending_input_matching(|item| {
                    matches_child_completion(item, child_thread_id)
                }));
            }
        }
        {
            let mut idle_pending_input = self.idle_pending_input.lock().await;
            let mut kept = Vec::with_capacity(idle_pending_input.len());
            for item in idle_pending_input.drain(..) {
                if matches_child_completion(&item, child_thread_id) {
                    removed.push(item);
                } else {
                    kept.push(item);
                }
            }
            *idle_pending_input = kept;
        }
        {
            let mut mailbox_rx = self.mailbox_rx.lock().await;
            mailbox_rx.drain_incoming_to_pending();
            removed.extend(
                mailbox_rx.extract_matching(|item| matches_child_completion(item, child_thread_id)),
            );
        }
        if removed.is_empty() {
            return 0;
        }
        self.mark_direct_child_completions_received_from_pending_input(removed.iter())
            .await;
        removed.len()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn prepend_pending_input(&self, input: Vec<PendingInputItem>) -> Result<(), ()> {
        let mut active = self.active_turn.lock().await;
        match active.as_mut() {
            Some(at) => {
                let mut ts = at.turn_state.lock().await;
                ts.prepend_pending_input(input);
                Ok(())
            }
            None => Err(()),
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn get_pending_input(&self) -> Vec<PendingInputItem> {
        let _scheduler = self.scheduler.lock().await;
        let (pending_input, accepts_async_input_for_current_turn) = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    (
                        ts.take_pending_input(),
                        ts.accepts_async_input_for_current_turn(),
                    )
                }
                None => (Vec::new(), true),
            }
        };
        if !accepts_async_input_for_current_turn {
            return pending_input;
        }
        let mailbox_items = {
            let mut mailbox_rx = self.mailbox_rx.lock().await;
            mailbox_rx.drain_incoming_to_pending();
            mailbox_rx.drain()
        };
        if pending_input.is_empty() {
            mailbox_items
        } else if mailbox_items.is_empty() {
            pending_input
        } else {
            let mut pending_input = pending_input;
            pending_input.extend(mailbox_items);
            pending_input
        }
    }

    /// Queue response items to be injected into the next active turn created for this session.
    pub(crate) async fn queue_response_items_for_next_turn(&self, items: Vec<PendingInputItem>) {
        if items.is_empty() {
            return;
        }

        {
            let _scheduler = self.scheduler.lock().await;
            self.idle_pending_input.lock().await.extend(items);
        }
        self.note_thread_wait_event(ThreadWaitSource::QueuedInput);
    }

    pub(crate) async fn take_queued_response_items_for_next_turn(&self) -> Vec<PendingInputItem> {
        std::mem::take(&mut *self.idle_pending_input.lock().await)
    }

    pub(crate) async fn has_queued_response_items_for_next_turn(&self) -> bool {
        !self.idle_pending_input.lock().await.is_empty()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state reads must remain atomic"
    )]
    pub async fn has_pending_input(&self) -> bool {
        let (has_turn_pending_input, accepts_async_input_for_current_turn) = {
            let active = self.active_turn.lock().await;
            match active.as_ref() {
                Some(at) => {
                    let ts = at.turn_state.lock().await;
                    (
                        ts.has_pending_input(),
                        ts.accepts_async_input_for_current_turn(),
                    )
                }
                None => (false, true),
            }
        };
        if has_turn_pending_input {
            return true;
        }
        if !accepts_async_input_for_current_turn {
            return false;
        }
        self.has_pending_mailbox_items().await
    }

    pub async fn interrupt_task(self: &Arc<Self>) {
        info!("interrupt received: abort current task, if any");
        let had_active_turn = self.active_turn.lock().await.is_some();
        // Even without an active task, interrupt handling pauses any active goal.
        self.abort_all_tasks(TurnAbortReason::Interrupted).await;
        if !had_active_turn {
            self.services
                .mcp_service
                .cancel_startup(self.as_ref())
                .await;
        }
    }

    pub(crate) async fn pending_thread_input_source_hint(&self) -> Option<String> {
        self.find_pending_input(|item| Some(thread_wait_source_hint_for_pending_input(item)))
            .await
            .flatten()
    }

    pub(crate) async fn poll_event(
        &self,
        request: thread_service_api::ThreadPollEventRequest,
    ) -> Result<thread_service_api::ThreadPollEventResult, tool_service_api::FunctionCallError>
    {
        let initial_timeout_ms = request.initial_timeout_ms.ok_or_else(|| {
            tool_service_api::FunctionCallError::Fatal(
                "poll_event requires initial_timeout_ms to be resolved by the thread runtime"
                    .to_string(),
            )
        })?;
        let hard_cap_timeout_ms = request.hard_cap_timeout_ms.ok_or_else(|| {
            tool_service_api::FunctionCallError::Fatal(
                "poll_event requires hard_cap_timeout_ms to be resolved by the thread runtime"
                    .to_string(),
            )
        })?;
        let started = Instant::now();
        let current_timeout = self
            .thread_wait_current_window(initial_timeout_ms, hard_cap_timeout_ms)
            .await;
        let current_timeout_ms = current_timeout.as_millis() as i64;
        let mut thread_wait_rx = self.subscribe_thread_wait_events();
        let wait_snapshot = *thread_wait_rx.borrow_and_update();
        if let Some(source_hint) = self.pending_thread_input_source_hint().await {
            self.reset_thread_wait_backoff().await;
            return Ok(thread_service_api::ThreadPollEventResult {
                timed_out: false,
                source_hint: Some(source_hint),
                waited_ms: 0,
                initial_timeout_ms,
                current_timeout_ms,
                hard_cap_timeout_ms,
            });
        }

        let source_hint = tokio::time::timeout(current_timeout, async move {
            loop {
                if thread_wait_rx.changed().await.is_err() {
                    return None;
                }
                let snapshot = *thread_wait_rx.borrow_and_update();
                if snapshot.seq > wait_snapshot.seq {
                    return snapshot.source.map(thread_wait_source_hint);
                }
            }
        })
        .await;

        match source_hint {
            Ok(source_hint) => {
                self.reset_thread_wait_backoff().await;
                Ok(thread_service_api::ThreadPollEventResult {
                    timed_out: false,
                    source_hint,
                    waited_ms: started.elapsed().as_millis() as i64,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                })
            }
            Err(_) => {
                self.advance_thread_wait_backoff(initial_timeout_ms, hard_cap_timeout_ms)
                    .await;
                Ok(thread_service_api::ThreadPollEventResult {
                    timed_out: true,
                    source_hint: None,
                    waited_ms: started.elapsed().as_millis() as i64,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                })
            }
        }
    }
}

fn matches_child_completion(item: &PendingInputItem, child_thread_id: ThreadId) -> bool {
    match item {
        PendingInputItem::InterAgentCommunication(communication)
        | PendingInputItem::ResponseItem(ResponseItem::InterAgentCommunication {
            communication,
            ..
        })
        | PendingInputItem::HookInspectable(ResponseItem::InterAgentCommunication {
            communication,
            ..
        }) => {
            communication.operation == InterAgentOperation::ChildCompletion
                && communication.sender_thread_id == Some(child_thread_id)
        }
        PendingInputItem::ResponseItem(_) | PendingInputItem::HookInspectable(_) => false,
    }
}

fn thread_wait_source_hint(source: ThreadWaitSource) -> String {
    match source {
        ThreadWaitSource::UserInput => "user_input",
        ThreadWaitSource::InterAgent => "inter_agent",
        ThreadWaitSource::ChildCompletion => "child_completion",
        ThreadWaitSource::QueuedInput => "queued_input",
        ThreadWaitSource::AsyncInput => "async_input",
        ThreadWaitSource::CommandOutput => "command_output",
        ThreadWaitSource::CommandExit => "command_exit",
    }
    .to_string()
}

fn thread_wait_source_for_communication(
    communication: &InterAgentCommunication,
) -> ThreadWaitSource {
    match communication.operation {
        InterAgentOperation::ChildCompletion => ThreadWaitSource::ChildCompletion,
        _ => ThreadWaitSource::InterAgent,
    }
}

fn thread_wait_source_for_pending_input_item(item: &PendingInputItem) -> ThreadWaitSource {
    match item {
        PendingInputItem::InterAgentCommunication(communication) => {
            thread_wait_source_for_communication(communication)
        }
        PendingInputItem::ResponseItem(ResponseItem::Message { role, .. })
        | PendingInputItem::HookInspectable(ResponseItem::Message { role, .. })
            if role == "user" =>
        {
            ThreadWaitSource::UserInput
        }
        PendingInputItem::ResponseItem(ResponseItem::CommandExecutionNotification {
            kind: protocol::models::CommandExecutionNotificationKind::Output,
            ..
        })
        | PendingInputItem::HookInspectable(ResponseItem::CommandExecutionNotification {
            kind: protocol::models::CommandExecutionNotificationKind::Output,
            ..
        }) => ThreadWaitSource::CommandOutput,
        PendingInputItem::ResponseItem(ResponseItem::CommandExecutionNotification {
            kind: protocol::models::CommandExecutionNotificationKind::Exit,
            ..
        })
        | PendingInputItem::HookInspectable(ResponseItem::CommandExecutionNotification {
            kind: protocol::models::CommandExecutionNotificationKind::Exit,
            ..
        }) => ThreadWaitSource::CommandExit,
        PendingInputItem::HookInspectable(_) | PendingInputItem::ResponseItem(_) => {
            ThreadWaitSource::AsyncInput
        }
    }
}

fn thread_wait_source_hint_for_pending_input(item: &PendingInputItem) -> Option<String> {
    Some(thread_wait_source_hint(
        thread_wait_source_for_pending_input_item(item),
    ))
}
