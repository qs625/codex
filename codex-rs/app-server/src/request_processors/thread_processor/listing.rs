use super::*;

struct ThreadListFilters {
    model_providers: Option<Vec<String>>,
    source_kinds: Option<Vec<ThreadSourceKind>>,
    archived: bool,
    cwd_filters: Option<Vec<PathBuf>>,
    search_term: Option<String>,
    use_state_db_only: bool,
}

struct ThreadListQuery {
    requested_page_size: usize,
    cursor: Option<String>,
    sort_key: StoreThreadSortKey,
    sort_direction: SortDirection,
    model_providers: Option<Vec<String>>,
    allowed_sources: Vec<protocol::protocol::SessionSource>,
    source_kind_filter: Option<Vec<ThreadSourceKind>>,
    archived: bool,
    cwd_filters: Option<Vec<PathBuf>>,
    search_term: Option<String>,
    use_state_db_only: bool,
}

impl ThreadListQuery {
    fn new(
        requested_page_size: usize,
        cursor: Option<String>,
        sort_key: StoreThreadSortKey,
        sort_direction: SortDirection,
        filters: ThreadListFilters,
        default_model_providers: Vec<String>,
    ) -> Self {
        let ThreadListFilters {
            model_providers,
            source_kinds,
            archived,
            cwd_filters,
            search_term,
            use_state_db_only,
        } = filters;
        let model_providers = match model_providers {
            Some(providers) if providers.is_empty() => None,
            Some(providers) => Some(providers),
            None => Some(default_model_providers),
        };
        let (allowed_sources, source_kind_filter) = compute_source_filters(source_kinds);

        Self {
            requested_page_size,
            cursor,
            sort_key,
            sort_direction,
            model_providers,
            allowed_sources,
            source_kind_filter,
            archived,
            cwd_filters,
            search_term,
            use_state_db_only,
        }
    }

    fn store_sort_direction(&self) -> StoreSortDirection {
        match self.sort_direction {
            SortDirection::Asc => StoreSortDirection::Asc,
            SortDirection::Desc => StoreSortDirection::Desc,
        }
    }

    fn store_params(&self, page_size: usize, cursor: Option<String>) -> StoreListThreadsParams {
        StoreListThreadsParams {
            page_size,
            cursor,
            sort_key: self.sort_key,
            sort_direction: self.store_sort_direction(),
            allowed_sources: self.allowed_sources.clone(),
            model_providers: self.model_providers.clone(),
            cwd_filters: self.cwd_filters.clone(),
            archived: self.archived,
            search_term: self.search_term.clone(),
            use_state_db_only: self.use_state_db_only,
        }
    }

    fn accepts_post_store_thread(&self, thread: &StoredThread) -> bool {
        let source = with_thread_spawn_agent_metadata(
            thread.source.clone(),
            thread.agent_nickname.clone(),
            thread.agent_role.clone(),
            thread.agent_path.clone(),
        );
        self.source_kind_filter
            .as_ref()
            .is_none_or(|filter| source_kind_matches(&source, filter))
            && self.cwd_filters.as_ref().is_none_or(|expected_cwds| {
                expected_cwds.iter().any(|expected_cwd| {
                    path_utils::paths_match_after_normalization(&thread.cwd, expected_cwd)
                })
            })
    }
}

impl ThreadRequestProcessor {
    pub(super) async fn thread_list_response_inner(
        &self,
        params: ThreadListParams,
    ) -> Result<ThreadListResponse, JSONRPCErrorError> {
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
        let data = self
            .project_listed_threads(stored_threads, use_state_db_only)
            .await;
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
        let ThreadLoadedListParams { cursor, limit } = params;
        let mut data: Vec<String> = self
            .live_thread_inspection
            .list_live_thread_ids()
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
        connection_id: ConnectionId,
    ) -> Result<ThreadReadResponse, JSONRPCErrorError> {
        let ThreadReadParams {
            thread_id,
            include_turns,
        } = params;

        let thread_uuid = ThreadId::from_string(&thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;

        let mut auto_resume_skipped_for_unknown_agent_role = false;
        if let Err(err) = self
            .ensure_persisted_native_thread_loaded(thread_uuid, /*parent_trace*/ None)
            .await
        {
            if is_unknown_agent_type_resume_error(&err) {
                auto_resume_skipped_for_unknown_agent_role = true;
                tracing::warn!(
                    thread_id = %thread_uuid,
                    error = %err.message,
                    "thread/read could not auto-resume stored agent role; returning persisted read-only view"
                );
            } else {
                return Err(err);
            }
        }
        if self
            .live_thread_inspection
            .is_live_thread_loaded(thread_uuid)
            .await
        {
            self.ensure_conversation_listener(thread_uuid, connection_id)
                .await?;
        }

        let mut thread = self
            .read_thread_view(thread_uuid, include_turns)
            .await
            .map_err(thread_read_view_error)?;
        if auto_resume_skipped_for_unknown_agent_role && thread.active_command_items.is_none() {
            thread.active_command_items = Some(Vec::new());
        }
        Ok(ThreadReadResponse { thread })
    }

    async fn list_threads_common(
        &self,
        requested_page_size: usize,
        cursor: Option<String>,
        sort_key: StoreThreadSortKey,
        sort_direction: SortDirection,
        filters: ThreadListFilters,
    ) -> Result<(Vec<StoredThread>, Option<String>), JSONRPCErrorError> {
        let query = ThreadListQuery::new(
            requested_page_size,
            cursor,
            sort_key,
            sort_direction,
            filters,
            self.default_thread_list_model_providers(),
        );
        self.fetch_thread_list_pages(query).await
    }

    async fn project_listed_threads(
        &self,
        stored_threads: Vec<StoredThread>,
        use_state_db_only: bool,
    ) -> Vec<Thread> {
        let mut threads = Vec::with_capacity(stored_threads.len());
        let fallback_provider = self.config.model_provider_id.clone();

        for stored_thread in stored_threads {
            let thread_id = stored_thread.thread_id;
            let (mut thread, history) = thread_from_stored_thread(
                stored_thread,
                fallback_provider.as_str(),
                &self.config.cwd,
            );
            if !use_state_db_only
                && history.is_none()
                && let Ok(history_items) =
                    read_thread_history_items(self.thread_store.as_ref(), thread_id).await
            {
                apply_persisted_thread_lifecycle_status(&mut thread, &history_items);
                apply_thread_stats_from_rollout_items(&mut thread, &history_items);
            }
            threads.push((thread_id, thread));
        }

        let mut projected_threads = Vec::with_capacity(threads.len());
        for (thread_id, mut thread) in threads {
            let has_live_in_progress_turn = if self
                .live_thread_inspection
                .is_live_thread_loaded(thread_id)
                .await
            {
                self.active_in_progress_turn_snapshot(thread_id).await.is_some()
            } else {
                false
            };
            set_thread_status_and_interrupt_stale_turns(
                &mut thread,
                ThreadLifecycleStatus::NotLoaded,
                has_live_in_progress_turn,
            );
            projected_threads.push(thread);
        }
        projected_threads
    }

    async fn fetch_thread_list_pages(
        &self,
        query: ThreadListQuery,
    ) -> Result<(Vec<StoredThread>, Option<String>), JSONRPCErrorError> {
        let mut cursor_obj = query.cursor.clone();
        let mut last_cursor = cursor_obj.clone();
        let mut remaining = query.requested_page_size;
        let mut items = Vec::with_capacity(query.requested_page_size);
        let mut next_cursor: Option<String> = None;

        while remaining > 0 {
            let page_size = remaining.min(THREAD_LIST_MAX_LIMIT);
            let page = self
                .thread_store
                .list_threads(query.store_params(page_size, cursor_obj.clone()))
                .await
                .map_err(thread_store_list_error)?;

            for item in page
                .items
                .into_iter()
                .filter(|thread| query.accepts_post_store_thread(thread))
                .take(remaining)
            {
                items.push(item);
            }
            remaining = query.requested_page_size.saturating_sub(items.len());

            next_cursor = page.next_cursor;
            if remaining == 0 {
                break;
            }

            let Some(cursor_val) = next_cursor.clone() else {
                break;
            };
            // Break if our pagination would reuse the same cursor again; this avoids
            // an infinite loop when filtering drops everything on the page.
            if last_cursor.as_ref() == Some(&cursor_val) {
                next_cursor = None;
                break;
            }
            last_cursor = Some(cursor_val.clone());
            cursor_obj = Some(cursor_val);
        }

        Ok((items, next_cursor))
    }

    /// Builds the API view for `thread/read` from persisted metadata plus optional live state.
    pub(super) async fn read_thread_view(
        &self,
        thread_id: ThreadId,
        include_turns: bool,
    ) -> Result<Thread, ThreadReadViewError> {
        let live_snapshot = self
            .live_thread_inspection
            .live_thread_snapshot(thread_id)
            .await
            .ok();
        let (mut thread, has_live_in_progress_turn) = if include_turns {
            if let Some(live_snapshot) = live_snapshot.as_ref() {
                // Loaded thread with turns: keep the persisted turn projection available
                // so richer init-context items survive live-history reconstruction.
                let persisted_thread = match self
                    .load_persisted_thread_for_read(thread_id, /*include_turns*/ true)
                    .await
                {
                    Ok(thread) => thread,
                    Err(ThreadReadViewError::InvalidRequest(message))
                        if Self::is_include_turns_unavailable_before_first_user_message(
                            &message,
                        ) =>
                    {
                        None
                    }
                    Err(err) => return Err(err),
                };
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

        set_thread_status_and_interrupt_stale_turns(
            &mut thread,
            ThreadLifecycleStatus::NotLoaded,
            has_live_in_progress_turn,
        );
        Ok(thread)
    }

    fn is_include_turns_unavailable_before_first_user_message(message: &str) -> bool {
        message.contains("includeTurns is unavailable before first user message")
    }

    pub(super) async fn active_in_progress_turn_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> Option<Turn> {
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
                    apply_runtime_activity_items_from_persisted_turns(&mut thread);
                    prune_turns_to_latest_compaction_boundary(&mut thread.turns);
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
        let LiveThreadReadProjectionBase {
            mut thread,
            persisted_turns,
        } = live_thread_read_projection_base(fallback_thread, persisted_thread);
        let active_turn = self.active_in_progress_turn_snapshot(thread_id).await;
        let has_live_in_progress_turn = match self
            .apply_thread_read_store_fields(
                thread_id,
                &mut thread,
                include_turns,
                active_turn.as_ref(),
            )
            .await
        {
            Ok(has_live_in_progress_turn) => has_live_in_progress_turn,
            Err(ThreadReadViewError::InvalidRequest(message))
                if include_turns
                    && Self::is_include_turns_unavailable_before_first_user_message(&message) =>
            {
                false
            }
            Err(err) => return Err(err),
        };
        if include_turns {
            restore_persisted_display_turns(&mut thread, &persisted_turns);
            apply_runtime_activity_items_from_persisted_turns(&mut thread);
            self.apply_persisted_subscription_snapshot_items(thread_id, &mut thread)
                .await?;
            prune_turns_to_latest_compaction_boundary(&mut thread.turns);
        }
        apply_live_active_command_items_from_active_turn(&mut thread, active_turn.as_ref());
        Ok((thread, has_live_in_progress_turn))
    }

    async fn apply_persisted_subscription_snapshot_items(
        &self,
        thread_id: ThreadId,
        thread: &mut Thread,
    ) -> Result<(), ThreadReadViewError> {
        if thread.active_subscription_items.is_some() {
            return Ok(());
        }

        match self
            .thread_store
            .read_thread_subscriptions(thread_id, /*include_archived*/ true)
            .await
        {
            Ok(Some(subscriptions)) => {
                thread.active_subscription_items = Some(active_subscription_items_from_snapshot(
                    subscriptions.as_slice(),
                ));
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(ThreadStoreError::ThreadNotFound {
                thread_id: missing_thread_id,
            }) if missing_thread_id == thread_id => Ok(()),
            Err(ThreadStoreError::InvalidRequest { message })
                if message == format!("no rollout found for thread id {thread_id}") =>
            {
                Ok(())
            }
            Err(ThreadStoreError::InvalidRequest { message }) => {
                Err(ThreadReadViewError::InvalidRequest(message))
            }
            Err(ThreadStoreError::Unsupported { operation }) => {
                Err(ThreadReadViewError::Unsupported(operation))
            }
            Err(err) => Err(ThreadReadViewError::Internal(format!(
                "failed to read thread subscriptions for {thread_id}: {err}"
            ))),
        }
    }

    pub(super) async fn apply_thread_read_store_fields(
        &self,
        thread_id: ThreadId,
        thread: &mut Thread,
        include_turns: bool,
        active_turn: Option<&Turn>,
    ) -> Result<bool, ThreadReadViewError> {
        self.attach_thread_name(thread_id, thread).await;
        let history = self
            .live_thread_history
            .live_thread_history(thread_id, /*include_archived*/ true)
            .await
            .map_err(|err| thread_read_history_load_error(thread_id, err))?;
        apply_thread_stats_from_rollout_items(thread, history.items.as_slice());
        if let Some(token_usage) = self
            .live_thread_usage
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
                    .live_thread_usage
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

        let has_live_in_progress_turn = active_turn.is_some();
        if include_turns {
            populate_thread_turns_from_history(thread, &history.items, active_turn);
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
            .thread_lifecycle_runtime
            .live_thread_agent_status(thread_uuid)
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
        turns.retain(|turn| !is_active_subscriptions_turn(turn) && !is_active_commands_turn(turn));
        project_thread_turn_items_view(&mut turns, items_view);
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
            .live_thread_inspection
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

        self.live_thread_history
            .live_thread_history(thread_id, /*include_archived*/ true)
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
            if !self
                .live_thread_inspection
                .is_live_thread_loaded(thread_id)
                .await
            {
                // Reconcile stale app-server bookkeeping when the thread has already been
                // removed from the core manager.
                self.finalize_thread_teardown(thread_id).await;
            }
        }
    }

    pub(crate) fn subscribe_running_assistant_turn_count(&self) -> watch::Receiver<usize> {
        self.thread_watch_manager.subscribe_running_turn_count()
    }

    /// Best-effort: ensure initialized connections are subscribed to this thread.
    pub(crate) async fn try_attach_thread_listener(
        &self,
        thread_id: ThreadId,
        connection_ids: Vec<ConnectionId>,
    ) {
        if let Ok(live_snapshot) = self
            .live_thread_inspection
            .live_thread_snapshot(thread_id)
            .await
        {
            let loaded_thread = build_thread_from_live_snapshot(thread_id, &live_snapshot);
            self.thread_watch_manager
                .upsert_thread_silently(loaded_thread)
                .await;
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

fn is_unknown_agent_type_resume_error(err: &JSONRPCErrorError) -> bool {
    err.code == crate::error_code::INVALID_REQUEST_ERROR_CODE
        && err.message.starts_with("unknown agent_type ")
}
