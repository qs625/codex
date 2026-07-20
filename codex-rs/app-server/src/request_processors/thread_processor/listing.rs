use super::*;

impl ThreadRequestProcessor {
    pub(super) async fn thread_list_response_inner(
        &self,
        params: ThreadListParams,
    ) -> Result<ThreadListResponse, JSONRPCErrorError> {
        self.restore_persisted_active_threads_on_startup().await;
        let ThreadListParams {
            cursor,
            limit,
            sort_key,
            sort_direction,
            model_providers,
            source_kinds,
            archived,
            cwd,
            use_state_db_only,
            search_term,
        } = params;
        let cwd_filters = normalize_thread_list_cwd_filters(cwd)?;

        let requested_page_size = limit
            .map(|value| value as usize)
            .unwrap_or(THREAD_LIST_DEFAULT_LIMIT)
            .clamp(1, THREAD_LIST_MAX_LIMIT);
        let store_sort_key = match sort_key.unwrap_or(ThreadSortKey::CreatedAt) {
            ThreadSortKey::CreatedAt => StoreThreadSortKey::CreatedAt,
            ThreadSortKey::UpdatedAt => StoreThreadSortKey::UpdatedAt,
        };
        let sort_direction = sort_direction.unwrap_or(SortDirection::Desc);
        let (stored_threads, next_cursor) = self
            .list_threads_common(
                requested_page_size,
                cursor,
                store_sort_key,
                sort_direction,
                ThreadListFilters {
                    model_providers,
                    source_kinds,
                    archived: archived.unwrap_or(false),
                    cwd_filters,
                    search_term,
                    use_state_db_only,
                },
            )
            .await?;
        let backwards_cursor = stored_threads.first().and_then(|thread| {
            thread_backwards_cursor_for_sort_key(thread, store_sort_key, sort_direction)
        });
        let mut threads = Vec::with_capacity(stored_threads.len());
        let mut status_ids = Vec::with_capacity(stored_threads.len());
        let fallback_provider = self.config.model_provider_id.clone();

        for stored_thread in stored_threads {
            let (thread, _) = thread_from_stored_thread(
                stored_thread,
                fallback_provider.as_str(),
                &self.config.cwd,
            );
            status_ids.push(thread.id.clone());
            threads.push(thread);
        }

        let statuses = self
            .thread_watch_manager
            .loaded_statuses_for_threads(status_ids)
            .await;

        let data: Vec<_> = threads
            .into_iter()
            .map(|mut thread| {
                if let Some(status) = statuses.get(&thread.id) {
                    thread.lifecycle_status = status.clone();
                }
                thread
            })
            .collect();
        Ok(ThreadListResponse {
            data,
            next_cursor,
            backwards_cursor,
        })
    }

    pub(super) async fn thread_loaded_list_response_inner(
        &self,
        params: ThreadLoadedListParams,
    ) -> Result<ThreadLoadedListResponse, JSONRPCErrorError> {
        self.restore_persisted_active_threads_on_startup().await;
        let ThreadLoadedListParams { cursor, limit } = params;
        let mut data: Vec<String> = self
            .live_threads
            .list_thread_ids()
            .await
            .into_iter()
            .map(|thread_id| thread_id.to_string())
            .collect();

        if data.is_empty() {
            return Ok(ThreadLoadedListResponse {
                data,
                next_cursor: None,
            });
        }

        data.sort();
        let total = data.len();
        let start = match cursor {
            Some(cursor) => {
                let cursor = match ThreadId::from_string(&cursor) {
                    Ok(id) => id.to_string(),
                    Err(_) => return Err(invalid_request(format!("invalid cursor: {cursor}"))),
                };
                match data.binary_search(&cursor) {
                    Ok(idx) => idx + 1,
                    Err(idx) => idx,
                }
            }
            None => 0,
        };

        let effective_limit = limit.unwrap_or(total as u32).max(1) as usize;
        let end = start.saturating_add(effective_limit).min(total);
        let page = data[start..end].to_vec();
        let next_cursor = page.last().filter(|_| end < total).cloned();

        Ok(ThreadLoadedListResponse {
            data: page,
            next_cursor,
        })
    }

    pub(super) async fn thread_read_response_inner(
        &self,
        params: ThreadReadParams,
    ) -> Result<ThreadReadResponse, JSONRPCErrorError> {
        let ThreadReadParams {
            thread_id,
            include_turns,
        } = params;

        let thread_uuid = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        let thread = self
            .read_thread_view(thread_uuid, include_turns)
            .await
            .map_err(thread_read_view_error)?;
        Ok(ThreadReadResponse { thread })
    }

    /// Builds the API view for `thread/read` from persisted metadata plus optional live state.
    pub(super) async fn read_thread_view(
        &self,
        thread_id: ThreadId,
        include_turns: bool,
    ) -> Result<Thread, ThreadReadViewError> {
        let live_snapshot = self.live_threads.live_thread_snapshot(thread_id).await.ok();
        let (mut thread, has_live_in_progress_turn) = if include_turns {
            if let Some(live_snapshot) = live_snapshot.as_ref() {
                // Loaded thread with turns: keep the persisted turn projection available
                // so richer init-context items survive live-history reconstruction.
                let persisted_thread = self
                    .load_persisted_thread_for_read(thread_id, /*include_turns*/ true)
                    .await?;
                self.load_live_thread_view(
                    thread_id,
                    include_turns,
                    live_snapshot,
                    persisted_thread,
                )
                .await?
            } else if let Some(thread) = self
                .load_persisted_thread_for_read(thread_id, include_turns)
                .await?
            {
                // Unloaded thread with turns: load metadata and history together
                // from the ThreadStore.
                (thread, false)
            } else {
                return Err(ThreadReadViewError::InvalidRequest(format!(
                    "thread not loaded: {thread_id}"
                )));
            }
        } else if let Some(thread) = self
            .load_persisted_thread_for_read(thread_id, include_turns)
            .await?
        {
            // Persisted metadata-only read: preserve stored fields, but still
            // consult live state when the thread is loaded so status reflects
            // an in-progress turn before watch status catches up.
            let has_live_in_progress_turn = if live_snapshot.is_some() {
                self.active_in_progress_turn_snapshot(thread_id)
                    .await
                    .is_some()
            } else {
                false
            };
            (thread, has_live_in_progress_turn)
        } else if let Some(live_snapshot) = live_snapshot.as_ref() {
            // Loaded metadata-only read before persistence is materialized: build
            // the response from the live thread snapshot.
            self.load_live_thread_view(
                thread_id,
                include_turns,
                live_snapshot,
                /*persisted_thread*/ None,
            )
            .await?
        } else {
            return Err(ThreadReadViewError::InvalidRequest(format!(
                "thread not loaded: {thread_id}"
            )));
        };

        let thread_status = self
            .thread_watch_manager
            .loaded_status_for_thread(&thread.id)
            .await;

        set_thread_status_and_interrupt_stale_turns(
            &mut thread,
            thread_status,
            has_live_in_progress_turn,
        );
        Ok(thread)
    }

    pub(super) async fn active_in_progress_turn_snapshot(&self, thread_id: ThreadId) -> Option<Turn> {
        let thread_state = self.thread_state_manager.thread_state(thread_id).await;
        let state = thread_state.lock().await;
        state.active_in_progress_turn_snapshot()
    }

    pub(super) async fn load_persisted_thread_for_read(
        &self,
        thread_id: ThreadId,
        include_turns: bool,
    ) -> Result<Option<Thread>, ThreadReadViewError> {
        let fallback_provider = self.config.model_provider_id.as_str();
        match self
            .thread_store
            .read_thread(StoreReadThreadParams {
                thread_id,
                include_archived: true,
                // `thread/read` restores usage snapshots from rollout events even when
                // callers only request metadata.
                include_history: true,
            })
            .await
        {
            Ok(stored_thread) => {
                let (mut thread, history) =
                    thread_from_stored_thread(stored_thread, fallback_provider, &self.config.cwd);
                if include_turns && let Some(history) = history {
                    thread.turns = build_api_turns_from_rollout_items(&history.items);
                    suppress_init_only_display_turns(&mut thread.turns);
                }
                Ok(Some(thread))
            }
            Err(ThreadStoreError::InvalidRequest { message })
                if message == format!("no rollout found for thread id {thread_id}") =>
            {
                Ok(None)
            }
            Err(ThreadStoreError::ThreadNotFound {
                thread_id: missing_thread_id,
            }) if missing_thread_id == thread_id => Ok(None),
            Err(ThreadStoreError::InvalidRequest { message }) => {
                Err(ThreadReadViewError::InvalidRequest(message))
            }
            Err(err) => Err(ThreadReadViewError::Internal(format!(
                "failed to read thread: {err}"
            ))),
        }
    }

    /// Builds a `thread/read` view from a loaded thread plus optional persisted metadata.
    pub(super) async fn load_live_thread_view(
        &self,
        thread_id: ThreadId,
        include_turns: bool,
        live_snapshot: &LiveThreadSnapshot,
        persisted_thread: Option<Thread>,
    ) -> Result<(Thread, bool), ThreadReadViewError> {
        let config_snapshot = &live_snapshot.config_snapshot;
        if include_turns && config_snapshot.ephemeral {
            return Err(ThreadReadViewError::InvalidRequest(
                "ephemeral threads do not support includeTurns".to_string(),
            ));
        }
        let fallback_thread = build_thread_from_live_snapshot(thread_id, live_snapshot);
        let persisted_turns = persisted_thread
            .as_ref()
            .map(|thread| thread.turns.clone())
            .unwrap_or_default();
        let mut thread = if let Some(mut thread) = persisted_thread {
            if thread.path.is_none() {
                thread.path = fallback_thread.path.clone();
            }
            thread.session_id.clone_from(&fallback_thread.session_id);
            thread.ephemeral = fallback_thread.ephemeral;
            thread
        } else {
            fallback_thread
        };
        let has_live_in_progress_turn = self
            .apply_thread_read_store_fields(thread_id, &mut thread, include_turns)
            .await?;
        if include_turns {
            restore_persisted_injected_context_turns(&mut thread, &persisted_turns);
        }
        Ok((thread, has_live_in_progress_turn))
    }

    pub(super) async fn apply_thread_read_store_fields(
        &self,
        thread_id: ThreadId,
        thread: &mut Thread,
        include_turns: bool,
    ) -> Result<bool, ThreadReadViewError> {
        self.attach_thread_name(thread_id, thread).await;
        let history = self
            .live_threads
            .thread_history(thread_id, /*include_archived*/ true)
            .await
            .map_err(|err| thread_read_history_load_error(thread_id, err))?;
        if let Some(token_usage) = self
            .live_threads
            .thread_token_usage_info(thread_id)
            .await
            .map_err(|err| {
                ThreadReadViewError::Internal(format!("failed to read token usage: {err}"))
            })?
            .map(Into::into)
        {
            thread.token_usage = Some(token_usage);
        }
        if thread.context_usage.is_none() {
            let context_usage = if let Some(usage) =
                super::context_usage_replay::latest_nonzero_thread_context_usage_from_rollout_items(
                    history.items.as_slice(),
                ) {
                usage
            } else {
                let usage = self
                    .live_threads
                    .thread_context_usage(thread_id)
                    .await
                    .map_err(|err| {
                        ThreadReadViewError::Internal(format!(
                            "failed to read context usage: {err}"
                        ))
                    })?;
                if usage.total_bytes > 0 {
                    usage
                } else {
                    super::context_usage_replay::legacy_thread_context_usage_from_rollout_items(
                        history.items.as_slice(),
                    )
                    .unwrap_or(usage)
                }
            };
            thread.context_usage = Some(context_usage.into());
        }

        let active_turn = self.active_in_progress_turn_snapshot(thread_id).await;
        let has_live_in_progress_turn = active_turn.is_some();
        if include_turns {
            populate_thread_turns_from_history(thread, &history.items, active_turn.as_ref());
        }

        Ok(has_live_in_progress_turn)
    }

    pub(super) async fn thread_turns_list_response_inner(
        &self,
        params: ThreadTurnsListParams,
    ) -> Result<ThreadTurnsListResponse, JSONRPCErrorError> {
        let ThreadTurnsListParams {
            thread_id,
            cursor,
            limit,
            sort_direction,
            items_view,
        } = params;
        let items_view = items_view.unwrap_or(TurnItemsView::Summary);

        let thread_uuid = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        let items = self
            .load_thread_turns_list_history(thread_uuid)
            .await
            .map_err(thread_read_view_error)?;
        // This API optimizes network transfer by letting clients page through a
        // thread's turns incrementally, but it still replays the entire rollout on
        // every request. Rollback and compaction events can change earlier turns, so
        // the server has to rebuild the full turn list until turn metadata is indexed
        // separately.
        let live_agent_status = self
            .live_threads
            .thread_agent_status(thread_uuid)
            .await
            .ok();
        let has_live_running_thread = matches!(live_agent_status, Some(AgentStatus::Running));
        let active_turn = if live_agent_status.is_some() {
            // Persisted history may not yet include the currently running turn. The
            // app-server listener has already projected live turn events into ThreadState,
            // so merge that in-memory snapshot before paginating.
            let thread_state = self.thread_state_manager.thread_state(thread_uuid).await;
            let state = thread_state.lock().await;
            state.active_in_progress_turn_snapshot()
        } else {
            None
        };
        let mut turns = reconstruct_thread_turns_for_turns_list(
            &items,
            self.thread_watch_manager
                .loaded_status_for_thread(&thread_uuid.to_string())
                .await,
            has_live_running_thread,
            active_turn,
        );
        suppress_init_only_display_turns(&mut turns);
        for turn in &mut turns {
            match items_view {
                TurnItemsView::NotLoaded => {
                    turn.items.clear();
                    turn.items_view = TurnItemsView::NotLoaded;
                }
                TurnItemsView::Summary => {
                    let first_user_message = turn
                        .items
                        .iter()
                        .find(|item| matches!(item, ThreadItem::UserMessage { .. }))
                        .cloned();
                    let final_agent_message = turn
                        .items
                        .iter()
                        .rev()
                        .find(|item| matches!(item, ThreadItem::AgentMessage { .. }))
                        .cloned();
                    let initial_injected_context = turn
                        .items
                        .iter()
                        .find(|item| matches!(item, ThreadItem::InjectedContext { .. }))
                        .cloned();
                    turn.items = match (
                        first_user_message,
                        final_agent_message,
                        initial_injected_context,
                    ) {
                        (Some(user_message), Some(agent_message), _)
                            if user_message.id() != agent_message.id() =>
                        {
                            vec![user_message, agent_message]
                        }
                        (Some(user_message), _, _) => vec![user_message],
                        (None, Some(agent_message), _) => vec![agent_message],
                        (None, None, Some(injected_context)) => vec![injected_context],
                        (None, None, None) => Vec::new(),
                    };
                    turn.items_view = TurnItemsView::Summary;
                }
                TurnItemsView::Full => {
                    turn.items_view = TurnItemsView::Full;
                }
            }
        }
        let page = paginate_thread_turns(
            turns,
            cursor.as_deref(),
            limit,
            sort_direction.unwrap_or(SortDirection::Desc),
        )?;
        Ok(ThreadTurnsListResponse {
            data: page.turns,
            next_cursor: page.next_cursor,
            backwards_cursor: page.backwards_cursor,
        })
    }

    pub(super) async fn load_thread_turns_list_history(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<RolloutItem>, ThreadReadViewError> {
        match read_thread_history_items(self.thread_store.as_ref(), thread_id).await {
            Ok(items) => return Ok(items),
            Err(ThreadStoreError::InvalidRequest { message })
                if message == format!("no rollout found for thread id {thread_id}") => {}
            Err(ThreadStoreError::ThreadNotFound {
                thread_id: missing_thread_id,
            }) if missing_thread_id == thread_id => {}
            Err(ThreadStoreError::InvalidRequest { message }) => {
                return Err(ThreadReadViewError::InvalidRequest(message));
            }
            Err(err) => {
                return Err(ThreadReadViewError::Internal(format!(
                    "failed to read thread: {err}"
                )));
            }
        }

        let live_snapshot = self
            .live_threads
            .live_thread_snapshot(thread_id)
            .await
            .map_err(|_| {
                ThreadReadViewError::InvalidRequest(format!("thread not loaded: {thread_id}"))
            })?;
        if live_snapshot.config_snapshot.ephemeral {
            return Err(ThreadReadViewError::InvalidRequest(
                "ephemeral threads do not support thread/turns/list".to_string(),
            ));
        }

        self.live_threads
            .thread_history(thread_id, /*include_archived*/ true)
            .await
            .map(|history| history.items)
            .map_err(|err| thread_turns_list_history_load_error(thread_id, err))
    }

    pub(crate) fn thread_created_receiver(&self) -> broadcast::Receiver<ThreadCreatedEvent> {
        self.thread_lifecycle_runtime.subscribe_thread_created()
    }

    pub(crate) async fn connection_initialized(
        &self,
        connection_id: ConnectionId,
        capabilities: ConnectionCapabilities,
    ) {
        self.thread_state_manager
            .connection_initialized(connection_id, capabilities)
            .await;
    }

    pub(crate) async fn connection_closed(&self, connection_id: ConnectionId) {
        let thread_ids = self
            .thread_state_manager
            .remove_connection(connection_id)
            .await;

        for thread_id in thread_ids {
            if !self.live_threads.is_thread_loaded(thread_id).await {
                // Reconcile stale app-server bookkeeping when the thread has already been
                // removed from the core manager.
                self.finalize_thread_teardown(thread_id).await;
            }
        }
    }

    pub(crate) fn subscribe_running_assistant_turn_count(&self) -> watch::Receiver<usize> {
        self.thread_watch_manager.subscribe_running_turn_count()
    }

    pub(super) async fn restore_persisted_active_threads_on_startup(&self) {
        self.startup_active_threads_restored
            .get_or_init(|| async {
                self.restore_persisted_active_threads_on_startup_inner()
                    .await;
            })
            .await;
    }

    pub(super) async fn restore_persisted_active_threads_on_startup_inner(&self) {
        let thread_ids = self
            .list_threads_with_persisted_subscriptions()
            .await
            .unwrap_or_else(|err| {
                warn!("failed to list threads with persisted subscriptions: {err:?}");
                Vec::new()
            });

        for thread_id in thread_ids {
            if self.live_threads.is_thread_loaded(thread_id).await {
                continue;
            }
            self.restore_persisted_active_thread(thread_id).await;
        }
    }

    pub(super) async fn list_threads_with_persisted_subscriptions(
        &self,
    ) -> Result<Vec<ThreadId>, JSONRPCErrorError> {
        let Some(_local_thread_store) = self
            .thread_store
            .as_any()
            .downcast_ref::<LocalThreadStore>()
        else {
            return Ok(Vec::new());
        };
        let mut cursor = None;
        let mut thread_ids = Vec::new();

        loop {
            let page = self
                .thread_store
                .list_threads(thread_store::ListThreadsParams {
                    page_size: THREAD_LIST_MAX_LIMIT,
                    cursor,
                    sort_key: thread_store::ThreadSortKey::UpdatedAt,
                    sort_direction: thread_store::SortDirection::Desc,
                    allowed_sources: Vec::new(),
                    model_providers: Some(Vec::new()),
                    cwd_filters: None,
                    archived: false,
                    search_term: None,
                    use_state_db_only: false,
                })
                .await
                .map_err(thread_store_list_error)?;

            for thread in page.items {
                if persisted_subscription_count_from_rollout(thread.rollout_path.as_deref()) > 0 {
                    thread_ids.push(thread.thread_id);
                }
            }

            if let Some(next_cursor) = page.next_cursor {
                cursor = Some(next_cursor);
            } else {
                break;
            }
        }

        Ok(thread_ids)
    }

    pub(super) async fn restore_persisted_active_thread(&self, thread_id: ThreadId) {
        let thread_id_string = thread_id.to_string();
        let (thread_history, stored_thread) = match self
            .resume_thread_from_rollout(&thread_id_string, /*path*/ None)
            .await
        {
            Ok(value) => value,
            Err(err) => {
                warn!("failed to load persisted active thread {thread_id}: {err:?}");
                return;
            }
        };

        let persisted_subscription_count = persisted_subscription_count(&stored_thread);
        if persisted_subscription_count == 0 {
            return;
        }

        let session_source = stored_thread_session_source_with_agent_metadata(&stored_thread);
        let agent_metadata = stored_thread_root_agent_metadata(&stored_thread);
        let stored_agent_path = stored_thread.agent_path.clone();
        let stored_agent_role = stored_thread.agent_role.clone();
        let history_cwd = thread_history.session_cwd();
        let mut request_overrides = None;
        let mut typesafe_overrides = self.build_thread_config_overrides(
            /*model*/ None, /*model_provider*/ None, /*service_tier*/ None,
            /*cwd*/ None, /*runtime_workspace_roots*/ None,
            /*approval_policy*/ None, /*approvals_reviewer*/ None, /*sandbox*/ None,
            /*permissions*/ None, /*base_instructions*/ None,
            /*developer_instructions*/ None, /*personality*/ None,
        );
        self.load_and_apply_persisted_resume_metadata(
            &thread_history,
            &mut request_overrides,
            &mut typesafe_overrides,
        )
        .await;

        let config = match self
            .config_manager
            .load_for_cwd(request_overrides, typesafe_overrides, history_cwd)
            .await
        {
            Ok(config) => config,
            Err(err) => {
                warn!("failed to load config while restoring thread {thread_id}: {err}");
                return;
            }
        };

        match self
            .thread_runtime
            .resume_thread_with_history_and_source(
                config,
                thread_history,
                session_source,
                agent_metadata,
                /*parent_trace*/ None,
            )
            .await
        {
            Ok(ThreadProcessorNewThread {
                thread_id,
                thread,
                session_configured,
                ..
            }) => {
                let config_snapshot = thread.config_snapshot().await;
                let mut loaded_thread = build_thread_from_snapshot(
                    thread_id,
                    thread.session_configured().session_id.to_string(),
                    &config_snapshot,
                    session_configured.rollout_path,
                );
                apply_stored_agent_metadata_to_loaded_thread(
                    &mut loaded_thread,
                    stored_agent_path,
                    stored_agent_role,
                );
                self.thread_watch_manager
                    .upsert_thread_silently(loaded_thread)
                    .await;
                let active_event_subscriptions =
                    self.thread_lifecycle_runtime.active_event_subscriptions();
                sync_active_event_subscriptions(
                    active_event_subscriptions.as_ref(),
                    &self.thread_watch_manager,
                    thread_id,
                    persisted_subscription_count,
                )
                .await;
            }
            Err(err) => {
                warn!("failed to restore persisted active thread {thread_id}: {err}");
            }
        }
    }

    /// Best-effort: ensure initialized connections are subscribed to this thread.
    pub(crate) async fn try_attach_thread_listener(
        &self,
        thread_id: ThreadId,
        connection_ids: Vec<ConnectionId>,
    ) {
        if let Ok(live_snapshot) = self.live_threads.live_thread_snapshot(thread_id).await {
            let loaded_thread = build_thread_from_live_snapshot(thread_id, &live_snapshot);
            self.thread_watch_manager.upsert_thread(loaded_thread).await;
        }

        for connection_id in connection_ids {
            log_listener_attach_result(
                self.ensure_conversation_listener(thread_id, connection_id)
                    .await,
                thread_id,
                connection_id,
                "thread",
            );
        }
    }
}

fn apply_stored_agent_metadata_to_loaded_thread(
    loaded_thread: &mut Thread,
    stored_agent_path: Option<String>,
    stored_agent_role: Option<String>,
) {
    if stored_agent_path.is_some() {
        loaded_thread.agent_path = stored_agent_path;
    }
    if stored_agent_role.is_some() {
        loaded_thread.agent_role = stored_agent_role;
    }
}

fn restore_persisted_injected_context_turns(thread: &mut Thread, persisted_turns: &[Turn]) {
    for (persisted_index, persisted_turn) in persisted_turns.iter().enumerate() {
        let persisted_injected_items: Vec<_> = persisted_turn
            .items
            .iter()
            .filter(|item| matches!(item, ThreadItem::InjectedContext { .. }))
            .cloned()
            .collect();
        if persisted_injected_items.is_empty() {
            continue;
        }

        if let Some(live_turn) = thread.turns.iter_mut().find(|turn| turn.id == persisted_turn.id) {
            restore_persisted_injected_context_items(live_turn, &persisted_injected_items);
            continue;
        }

        let mut injected_context_turn = persisted_turn.clone();
        injected_context_turn.items = persisted_injected_items;
        injected_context_turn.items_view = TurnItemsView::Full;
        thread
            .turns
            .insert(persisted_index.min(thread.turns.len()), injected_context_turn);
    }
}

fn restore_persisted_injected_context_items(
    live_turn: &mut Turn,
    persisted_injected_items: &[ThreadItem],
) {
    let mut injected_insert_index = live_turn
        .items
        .iter()
        .position(|item| !matches!(item, ThreadItem::InjectedContext { .. }))
        .unwrap_or(live_turn.items.len());

    for persisted_item in persisted_injected_items {
        let persisted_id = persisted_item.id().to_string();
        if let Some(existing_index) = live_turn
            .items
            .iter()
            .position(|item| item.id() == persisted_id)
        {
            live_turn.items[existing_index] = persisted_item.clone();
            injected_insert_index = injected_insert_index.max(existing_index + 1);
        } else {
            live_turn
                .items
                .insert(injected_insert_index, persisted_item.clone());
            injected_insert_index += 1;
        }
    }

    if live_turn
        .items
        .iter()
        .any(|item| matches!(item, ThreadItem::InjectedContext { .. }))
    {
        live_turn.items_view = TurnItemsView::Full;
    }
}

#[cfg(test)]
mod restore_persisted_injected_context_turns_tests {
    use super::*;
    use app_server_protocol::InjectedContextSection;
    use app_server_protocol::SessionSource;
    use codex_utils_absolute_path::test_support::PathBufExt;

    fn thread_with_turns(turns: Vec<Turn>) -> Thread {
        Thread {
            id: "thread-1".to_string(),
            session_id: "session-1".to_string(),
            forked_from_id: None,
            preview: String::new(),
            ephemeral: false,
            model_provider: "mock_provider".to_string(),
            created_at: 1,
            updated_at: 1,
            lifecycle_status: ThreadLifecycleStatus::completed(None),
            path: None,
            cwd: codex_utils_absolute_path::test_support::test_path_buf("/tmp").abs(),
            cli_version: "0.0.0".to_string(),
            source: SessionSource::Cli,
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            git_info: None,
            name: None,
            skills: Vec::new(),
            token_usage: None,
            context_usage: None,
            turns,
        }
    }

    fn injected_context_item(id: &str, text: &str) -> ThreadItem {
        ThreadItem::InjectedContext {
            id: id.to_string(),
            title: "Init Context".to_string(),
            preview: "Init Context".to_string(),
            sections: vec![InjectedContextSection {
                label: "Instructions".to_string(),
                text: text.to_string(),
            }],
        }
    }

    fn agent_message_item(id: &str, text: &str) -> ThreadItem {
        ThreadItem::AgentMessage {
            id: id.to_string(),
            text: text.to_string(),
            phase: None,
            memory_citation: None,
        }
    }

    fn turn(id: &str, items: Vec<ThreadItem>) -> Turn {
        Turn {
            id: id.to_string(),
            items,
            items_view: TurnItemsView::Full,
            status: TurnStatus::Completed,
            error: None,
            started_at: Some(1),
            completed_at: Some(2),
            duration_ms: Some(100),
        }
    }

    #[test]
    fn restore_persisted_injected_context_turns_replaces_live_sections_with_persisted_item() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![injected_context_item("ctx-1", "live init context")],
        )]);
        let persisted_turns = vec![turn(
            "turn-1",
            vec![injected_context_item("ctx-1", "persisted instruction_files text")],
        )];

        restore_persisted_injected_context_turns(&mut thread, &persisted_turns);

        assert_eq!(
            thread.turns[0].items,
            vec![injected_context_item(
                "ctx-1",
                "persisted instruction_files text"
            )]
        );
    }

    #[test]
    fn restore_persisted_injected_context_turns_inserts_missing_turn_without_dup_agent_items() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-2",
            vec![agent_message_item("msg-1", "final assistant output")],
        )]);
        let persisted_turns = vec![
            turn(
                "turn-1",
                vec![injected_context_item("ctx-1", "persisted instruction_files text")],
            ),
            turn(
                "turn-2",
                vec![
                    injected_context_item("ctx-2", "persisted compact init context"),
                    agent_message_item("msg-1", "older assistant output"),
                ],
            ),
        ];

        restore_persisted_injected_context_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 2);
        assert_eq!(
            thread.turns[0].items,
            vec![injected_context_item(
                "ctx-1",
                "persisted instruction_files text"
            )]
        );
        assert_eq!(
            thread.turns[1].items,
            vec![
                injected_context_item("ctx-2", "persisted compact init context"),
                agent_message_item("msg-1", "final assistant output"),
            ]
        );
    }

    #[test]
    fn stored_agent_metadata_preserves_snapshot_path_when_stored_path_is_missing() {
        let mut thread = thread_with_turns(Vec::new());
        thread.agent_path = Some("/root/legacy_child".to_string());
        thread.agent_role = Some("default".to_string());

        apply_stored_agent_metadata_to_loaded_thread(
            &mut thread,
            None,
            Some("feature-owner".to_string()),
        );

        assert_eq!(thread.agent_path.as_deref(), Some("/root/legacy_child"));
        assert_eq!(thread.agent_role.as_deref(), Some("feature-owner"));
    }

    #[test]
    fn stored_agent_metadata_overrides_snapshot_path_when_stored_path_exists() {
        let mut thread = thread_with_turns(Vec::new());
        thread.agent_path = Some("/root/from_source".to_string());

        apply_stored_agent_metadata_to_loaded_thread(
            &mut thread,
            Some("/root/from_metadata".to_string()),
            None,
        );

        assert_eq!(thread.agent_path.as_deref(), Some("/root/from_metadata"));
    }
}
