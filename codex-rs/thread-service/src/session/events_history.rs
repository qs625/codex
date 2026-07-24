use super::*;
use std::sync::Weak;

fn developer_instructions_contains_section(
    developer_instructions: Option<&str>,
    section: &str,
) -> bool {
    let section = section.trim();
    !section.is_empty()
        && developer_instructions
            .map(str::trim)
            .is_some_and(|developer_instructions| developer_instructions.contains(section))
}

fn append_developer_instructions_section(
    developer_instructions: &mut Option<String>,
    section: String,
) {
    let section = section.trim();
    if section.is_empty() {
        return;
    }
    match developer_instructions {
        Some(existing) if !existing.trim().is_empty() => {
            existing.push_str("\n\n");
            existing.push_str(section);
        }
        _ => {
            *developer_instructions = Some(section.to_string());
        }
    }
}

impl Session {
    const EXTERNAL_AGENT_TOOL_SPECS_CONTEXT_MAX_CHARS: usize = 48_000;
    const EXTERNAL_AGENT_TOOL_NAMES: &'static [&'static str] = &[
        "spawn_external_agent",
        "followup_external_task",
        "list_external_agents",
        "close_external_agent",
    ];

    fn self_arc_for_initial_context(&self) -> Option<Arc<Self>> {
        self.self_weak
            .get()
            .and_then(Weak::upgrade)
    }

    async fn external_agent_tool_specs_for_initial_context(
        &self,
        turn_context: &TurnContext,
    ) -> Vec<tool_service_api::ToolSpec> {
        let Some(sess) = self.self_arc_for_initial_context() else {
            warn!("skipping external agent tool specs for initial context without session Arc");
            return Vec::new();
        };
        let Some(turn_context) = turn_context.self_weak.get().and_then(Weak::upgrade) else {
            warn!("skipping external agent tool specs for initial context without turn context Arc");
            return Vec::new();
        };
        let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
            Arc::clone(&sess) as Arc<dyn thread_service_api::ThreadSessionCapability>;
        let tool_inputs = match crate::session::turn::built_tools(
            Arc::clone(&sess),
            Arc::clone(&turn_context),
            Arc::downgrade(&session_capability),
            &[],
            &std::collections::HashSet::new(),
            Some(turn_context.turn_skills.outcome.as_ref()),
            &CancellationToken::new(),
        )
        .await
        {
            Ok(tool_inputs) => tool_inputs,
            Err(err) => {
                warn!("failed to build external agent tool specs for initial context: {err}");
                return Vec::new();
            }
        };
        crate::session::turn::model_visible_tool_specs(&sess, &turn_context, &tool_inputs)
            .into_iter()
            .filter(Self::is_external_agent_tool_spec)
            .collect()
    }

    fn is_external_agent_tool_spec(spec: &tool_service_api::ToolSpec) -> bool {
        match spec {
            tool_service_api::ToolSpec::Function(tool) => {
                Self::EXTERNAL_AGENT_TOOL_NAMES.contains(&tool.name.as_str())
            }
            _ => false,
        }
    }

    fn truncate_external_agent_tool_specs_json(json: &mut String) {
        if json.len() <= Self::EXTERNAL_AGENT_TOOL_SPECS_CONTEXT_MAX_CHARS {
            return;
        }
        let truncate_at = json
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx <= Self::EXTERNAL_AGENT_TOOL_SPECS_CONTEXT_MAX_CHARS)
            .last()
            .unwrap_or(0);
        json.truncate(truncate_at);
        json.push_str("\n... truncated ...");
    }

    pub(crate) fn external_agent_tool_specs_context_section(
        specs: &[tool_service_api::ToolSpec],
    ) -> Option<String> {
        let specs = specs
            .iter()
            .filter(|spec| Self::is_external_agent_tool_spec(spec))
            .collect::<Vec<_>>();
        if specs.is_empty() {
            return None;
        }
        let mut specs_json = Vec::with_capacity(specs.len());
        for spec in specs {
            let Ok(value) = serde_json::to_value(spec) else {
                warn!("failed to serialize external agent tool spec for initial context");
                continue;
            };
            specs_json.push(value);
        }
        if specs_json.is_empty() {
            return None;
        }
        let mut json =
            serde_json::to_string_pretty(&specs_json).unwrap_or_else(|_| "[]".to_string());
        Self::truncate_external_agent_tool_specs_json(&mut json);
        Some(format!(
            "<external_agent_tools>\n这些 external agent tools 属于独立的外部 CLI agent 协作总线。内置/native tools 已经通过模型 API tool config 暴露，不会在这里重复注入。协调外部 CLI agents 时，请按下面的 tool specs、schemas 和参数说明使用这些 external tools。\n```json\n{json}\n```\n</external_agent_tools>"
        ))
    }

    pub(crate) async fn build_initial_context_for_external_agent_tools(
        &self,
        turn_context: &TurnContext,
    ) -> Vec<ResponseItem> {
        let external_agent_tool_specs = self
            .external_agent_tool_specs_for_initial_context(turn_context)
            .await;
        self.build_initial_context_with_external_agent_tool_specs(
            turn_context,
            &external_agent_tool_specs,
        )
        .await
    }

    fn init_context_workflow_registry(
        &self,
        turn_context: &TurnContext,
    ) -> codex_workflow_api::WorkflowRegistry {
        let trusted_discovery = turn_context.discovery_context();
        let mut registry = codex_workflow_api::load_workflow_registry_from_roots(
            trusted_discovery.home_root.clone(),
            trusted_discovery.project_roots.clone(),
        );
        let mut _temp_roots = Vec::new();
        let mut disabled_project_roots = Vec::new();
        for layer in turn_context.config.config_layer_stack.get_layers(
            config_service::ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ true,
        ) {
            let codex_config_types::ConfigLayerSource::Project { dot_codex_folder } = &layer.name
            else {
                continue;
            };
            if layer.disabled_reason.is_none() {
                continue;
            }
            let Some(project_root) = dot_codex_folder.parent() else {
                continue;
            };
            let workflow_root = dot_codex_folder.join("workflows");
            if let Some((temp_root, copied_root)) =
                copy_safe_disabled_workflow_root_for_display(workflow_root.as_path(), &project_root)
            {
                _temp_roots.push(temp_root);
                disabled_project_roots.push(copied_root);
            }
        }

        if disabled_project_roots.is_empty() {
            return registry;
        }

        let disabled_registry = codex_workflow_api::load_workflow_registry_from_roots(
            trusted_discovery.home_root,
            disabled_project_roots,
        );
        let trusted_ids = registry
            .workflows
            .iter()
            .map(|workflow| workflow.id.clone())
            .collect::<std::collections::HashSet<_>>();
        registry.workflows.extend(
            disabled_registry
                .workflows
                .into_iter()
                .filter(|workflow| !trusted_ids.contains(&workflow.id)),
        );
        registry.diagnostics.extend(disabled_registry.diagnostics);
        registry
    }

    async fn current_agent_role_developer_instructions(
        &self,
        turn_context: &TurnContext,
        session_source: &SessionSource,
    ) -> Option<String> {
        let role_name = self
            .services
            .agent_control
            .get_agent_metadata(self.conversation_id)
            .and_then(|metadata| metadata.agent_role)
            .or_else(|| session_source.get_agent_role())?;
        let role =
            codex_agent_roles::resolve_role_config(&turn_context.config.agent_roles, &role_name)?;
        let role_file = role
            .source_path
            .as_deref()
            .or(role.config_file.as_deref())?;
        let is_built_in = !turn_context.config.agent_roles.contains_key(&role_name);
        let role_contents = if is_built_in {
            codex_agent_roles::built_in_config_file_contents(role_file)?.to_string()
        } else {
            tokio::fs::read_to_string(role_file).await.ok()?
        };
        let role_base_dir = if is_built_in {
            turn_context.config.codex_home.as_path()
        } else {
            role_file.parent()?
        };
        let parsed = codex_agent_roles::parse_agent_role_file_contents(
            &role_contents,
            role_file,
            role_base_dir,
            Some(&role_name),
        )
        .ok()?;
        parsed
            .config
            .as_table()
            .and_then(|table| table.get("developer_instructions"))
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
    }

    pub(crate) async fn reference_context_item_for_turn(
        &self,
        turn_context: &TurnContext,
    ) -> TurnContextItem {
        let mut item = turn_context.to_turn_context_item();
        let session_source = {
            let state = self.state.lock().await;
            state.session_configuration.session_source.clone()
        };
        if let Some(agent_role_instructions) = self
            .current_agent_role_developer_instructions(turn_context, &session_source)
            .await
            && !developer_instructions_contains_section(
                item.developer_instructions.as_deref(),
                &agent_role_instructions,
            )
        {
            append_developer_instructions_section(
                &mut item.developer_instructions,
                agent_role_instructions,
            );
        }
        item
    }

    pub(crate) async fn send_event(&self, turn_context: &TurnContext, msg: EventMsg) {
        let legacy_source = msg.clone();
        self.services
            .rollout_thread_trace
            .record_codex_turn_event(&turn_context.sub_id, &legacy_source);
        self.services
            .rollout_thread_trace
            .record_tool_call_event(turn_context.sub_id.clone(), &legacy_source);
        let event = Event {
            id: turn_context.sub_id.clone(),
            msg,
        };
        self.send_event_raw(event).await;
        self.maybe_mirror_event_text_to_realtime(&legacy_source)
            .await;
        self.maybe_clear_realtime_handoff_for_event(&legacy_source)
            .await;

        let show_raw_agent_reasoning = self.show_raw_agent_reasoning();
        for legacy in legacy_source.as_legacy_events(show_raw_agent_reasoning) {
            let legacy_event = Event {
                id: turn_context.sub_id.clone(),
                msg: legacy,
            };
            self.send_event_raw(legacy_event).await;
        }
    }

    /// Forwards finished spawned MultiAgentV2 children to their direct parent once inactive.
    pub(crate) async fn maybe_notify_parent_of_final_status(&self, turn_context: &TurnContext) {
        self.maybe_notify_parent_of_final_status_for_source(
            turn_context.sub_id.as_str(),
            &turn_context.session_source,
        )
        .await;
    }

    pub(crate) async fn maybe_notify_parent_of_final_status_for_current_source(&self) {
        let session_source = {
            let state = self.state.lock().await;
            state.session_configuration.session_source.clone()
        };
        let sub_id = self.next_internal_sub_id();
        self.maybe_notify_parent_of_final_status_for_source(&sub_id, &session_source)
            .await;
    }

    async fn maybe_notify_parent_of_final_status_for_source(
        &self,
        sub_id: &str,
        session_source: &SessionSource,
    ) {
        let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id,
            agent_path: Some(child_agent_path),
            ..
        }) = session_source
        else {
            return;
        };

        let status = self.agent_status.borrow().clone();
        if !is_final(&status) {
            return;
        }
        if self
            .services
            .agent_control
            .get_agent_metadata(self.conversation_id)
            .is_some_and(|metadata| metadata.agent_mode == AgentMode::Management)
        {
            return;
        }
        match Box::pin(self.thread_post_turn_state()).await {
            ThreadPostTurnState::ThreadCompletion
            | ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitEventSubscription) => {}
            ThreadPostTurnState::ThreadActive
            | ThreadPostTurnState::ThreadIdle(
                ThreadIdleReason::WaitChild | ThreadIdleReason::WaitCommand,
            )
            | ThreadPostTurnState::GoContextContinuation { .. } => return,
        }

        let _ = Box::pin(self.forward_child_completion_to_parent(
            sub_id,
            *parent_thread_id,
            child_agent_path,
            status,
        ))
        .await;
    }

    /// Sends the standard completion envelope from a spawned MultiAgentV2 child to its parent.
    async fn forward_child_completion_to_parent(
        &self,
        turn_id: &str,
        parent_thread_id: ThreadId,
        child_agent_path: &protocol::AgentPath,
        status: AgentStatus,
    ) -> bool {
        let Some(parent_agent_path) = child_agent_path
            .as_str()
            .rsplit_once('/')
            .and_then(|(parent, _)| protocol::AgentPath::try_from(parent).ok())
        else {
            return false;
        };

        let message = format_subagent_notification_message(child_agent_path.as_str(), &status);
        // `communication` owns the message. Keep a second copy only when the
        // recorder will actually need it after parent delivery succeeds.
        let trace_message = self
            .services
            .rollout_thread_trace
            .is_enabled()
            .then(|| message.clone());
        let communication = InterAgentCommunication::new(
            child_agent_path.clone(),
            parent_agent_path,
            Vec::new(),
            message,
            protocol::protocol::InterAgentOperation::ChildCompletion,
        )
        .with_trigger_turn(true)
        .with_thread_ids(self.conversation_id, parent_thread_id)
        .with_status(status.clone());
        if let Err(err) = self
            .services
            .agent_control
            .send_inter_agent_communication(parent_thread_id, communication)
            .await
        {
            debug!("failed to notify parent thread {parent_thread_id}: {err}");
            return false;
        }
        if let Some(message) = trace_message {
            self.services
                .rollout_thread_trace
                .record_agent_result_interaction(
                    turn_id,
                    parent_thread_id,
                    &AgentResultTracePayload {
                        child_agent_path: child_agent_path.as_str(),
                        message: &message,
                        status: &status,
                    },
                );
        }
        true
    }

    pub(crate) async fn has_active_direct_child(&self) -> bool {
        Box::pin(
            self.services
                .agent_control
                .direct_agent_children_are_active(self.conversation_id),
        )
        .await
    }

    pub(crate) async fn has_wait_command(&self) -> bool {
        self.services
            .command_service_state
            .has_running_process_for_thread(self.conversation_id)
            .await
    }

    pub(crate) fn has_active_event_subscription(&self) -> bool {
        self.services
            .active_event_subscriptions
            .active_count(self.conversation_id)
            > 0
    }

    async fn maybe_mirror_event_text_to_realtime(&self, msg: &EventMsg) {
        let Some(text) = realtime_text_for_event(msg) else {
            return;
        };
        if self.conversation.running_state().await.is_none()
            || self.conversation.active_handoff_id().await.is_none()
        {
            return;
        }
        if let Err(err) = self.conversation.handoff_out(text).await {
            debug!("failed to mirror event text to realtime conversation: {err}");
        }
    }

    async fn maybe_clear_realtime_handoff_for_event(&self, msg: &EventMsg) {
        if !matches!(msg, EventMsg::TurnComplete(_)) {
            return;
        }
        if let Err(err) = self.conversation.handoff_complete().await {
            debug!("failed to finalize realtime handoff output: {err}");
        }
        self.conversation.clear_active_handoff().await;
    }

    pub(crate) async fn send_event_raw(&self, event: Event) {
        // Persist the event into rollout storage (the store filters as needed).
        let rollout_items = vec![RolloutItem::EventMsg(event.msg.clone())];
        self.persist_rollout_items(&rollout_items).await;
        self.services
            .rollout_thread_trace
            .record_protocol_event(&event.msg);
        self.deliver_event_raw(event).await;
    }

    pub(crate) async fn deliver_event_raw(&self, event: Event) {
        // Record the last known agent status.
        if let Some(status) = agent_status_from_event(&event.msg) {
            self.agent_status.send_replace(status);
        }
        if let Err(e) = self.tx_event.send(event).await {
            debug!("dropping event because channel is closed: {e}");
        }
    }

    pub async fn emit_turn_item_started(&self, turn_context: &TurnContext, item: &TurnItem) {
        self.send_event(
            turn_context,
            EventMsg::ItemStarted(ItemStartedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item: item.clone(),
                started_at_ms: now_unix_timestamp_ms(),
            }),
        )
        .await;
    }

    pub async fn emit_turn_item_completed(&self, turn_context: &TurnContext, item: TurnItem) {
        record_turn_ttfm_metric(turn_context, &item).await;
        self.send_event(
            turn_context,
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item,
                completed_at_ms: now_unix_timestamp_ms(),
            }),
        )
        .await;
    }

    pub(crate) async fn emit_model_item_started_display_event(
        &self,
        turn_context: &TurnContext,
        item: &ResponseItem,
    ) {
        let now = now_unix_timestamp_ms();
        let event = match started_display_event_from_model_item(
            self.conversation_id,
            turn_context.sub_id.clone(),
            item,
            now,
        ) {
            Some(event) => event,
            None => EventMsg::ResponseItemStarted(ResponseItemStartedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item: item.clone(),
                started_at_ms: now,
            }),
        };
        self.send_event(turn_context, event).await;
    }

    /// Adds an execpolicy amendment to both the in-memory and on-disk policies so future
    /// commands can use the newly approved prefix.
    pub(crate) async fn persist_execpolicy_amendment(
        &self,
        amendment: &ExecPolicyAmendment,
    ) -> Result<(), ExecPolicyUpdateError> {
        let codex_home = self
            .state
            .lock()
            .await
            .session_configuration
            .codex_home()
            .clone();

        self.services
            .exec_policy
            .append_amendment_and_update(&codex_home, amendment)
            .await?;

        Ok(())
    }

    pub(crate) async fn turn_context_for_sub_id(&self, sub_id: &str) -> Option<Arc<TurnContext>> {
        let active = self.active_turn.lock().await;
        active
            .as_ref()
            .and_then(|turn| turn.tasks.get(sub_id))
            .map(|task| Arc::clone(&task.turn_context))
    }

    pub(crate) async fn active_turn_context_and_cancellation_token(
        &self,
    ) -> Option<(Arc<TurnContext>, CancellationToken)> {
        let active = self.active_turn.lock().await;
        let (_, task) = active.as_ref()?.tasks.first()?;
        Some((
            Arc::clone(&task.turn_context),
            task.cancellation_token.child_token(),
        ))
    }

    pub(crate) async fn record_execpolicy_amendment_message(
        &self,
        sub_id: &str,
        amendment: &ExecPolicyAmendment,
    ) {
        let Some(prefixes) = format_allow_prefixes(vec![amendment.command.clone()]) else {
            warn!("execpolicy amendment for {sub_id} had no command prefix");
            return;
        };
        let fragment = ApprovedCommandPrefixSaved::new(prefixes);
        let text = fragment.render();
        let message: ResponseItem = ContextualUserFragment::into(fragment);

        if let Some(turn_context) = self.turn_context_for_sub_id(sub_id).await {
            self.record_conversation_items(&turn_context, std::slice::from_ref(&message))
                .await;
            return;
        }

        if self
            .inject_hook_inspectable_items(vec![ResponseInputItem::Message {
                role: "developer".to_string(),
                content: vec![ContentItem::InputText { text }],
                phase: None,
            }])
            .await
            .is_err()
        {
            warn!("no active turn found to record execpolicy amendment message for {sub_id}");
        }
    }

    pub(crate) async fn persist_network_policy_amendment(
        &self,
        amendment: &NetworkPolicyAmendment,
        network_approval_context: &NetworkApprovalContext,
    ) -> anyhow::Result<()> {
        let _refresh_guard = self
            .managed_network_proxy_refresh_lock
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("managed network proxy refresh semaphore closed"))?;
        let host = validate_network_policy_amendment_host(amendment, network_approval_context)
            .map_err(anyhow::Error::msg)?;
        let codex_home = self
            .state
            .lock()
            .await
            .session_configuration
            .codex_home()
            .clone();
        let execpolicy_amendment =
            execpolicy_network_rule_amendment(amendment, network_approval_context, &host);

        if let Some(started_network_proxy) = self.services.network_proxy.as_ref() {
            let proxy = started_network_proxy.proxy();
            match amendment.action {
                NetworkPolicyRuleAction::Allow => proxy
                    .add_allowed_domain(&host)
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to update runtime allowlist: {err}"))?,
                NetworkPolicyRuleAction::Deny => proxy
                    .add_denied_domain(&host)
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to update runtime denylist: {err}"))?,
            }
        }

        self.services
            .exec_policy
            .append_network_rule_and_update(
                &codex_home,
                &host,
                execpolicy_amendment.protocol,
                execpolicy_amendment.decision,
                Some(execpolicy_amendment.justification),
            )
            .await
            .map_err(|err| {
                anyhow::anyhow!("failed to persist network policy amendment to execpolicy: {err}")
            })?;

        Ok(())
    }

    pub(crate) async fn record_network_policy_amendment_message(
        &self,
        sub_id: &str,
        amendment: &NetworkPolicyAmendment,
    ) {
        let fragment = NetworkRuleSaved::new(amendment);
        let text = fragment.render();
        let message: ResponseItem = ContextualUserFragment::into(fragment);

        if let Some(turn_context) = self.turn_context_for_sub_id(sub_id).await {
            self.record_conversation_items(&turn_context, std::slice::from_ref(&message))
                .await;
            return;
        }

        if self
            .inject_hook_inspectable_items(vec![ResponseInputItem::Message {
                role: "developer".to_string(),
                content: vec![ContentItem::InputText { text }],
                phase: None,
            }])
            .await
            .is_err()
        {
            warn!("no active turn found to record network policy amendment message for {sub_id}");
        }
    }

    /// Emit an exec approval request event and await the user's decision.
    ///
    /// The request is keyed by `call_id` + `approval_id` so matching responses
    /// are delivered to the correct in-flight turn. If the pending approval is
    /// cleared before a response arrives, treat it as an abort so interrupted
    /// turns do not continue on a synthetic denial.
    ///
    /// Note that if `available_decisions` is `None`, then the other fields will
    /// be used to derive the available decisions via
    /// [ExecApprovalRequestEvent::default_available_decisions].
    #[allow(clippy::too_many_arguments)]
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn request_command_approval(
        &self,
        turn_context: &TurnContext,
        call_id: String,
        approval_id: Option<String>,
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        reason: Option<String>,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        available_decisions: Option<Vec<ReviewDecision>>,
    ) -> ReviewDecision {
        //  command-level approvals use `call_id`.
        // `approval_id` is only present for subcommand callbacks (execve intercept)
        let effective_approval_id = approval_id.clone().unwrap_or_else(|| call_id.clone());
        // Add the tx_approve callback to the map before sending the request.
        let (tx_approve, rx_approve) = oneshot::channel();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_approval(effective_approval_id.clone(), tx_approve)
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            warn!("Overwriting existing pending approval for call_id: {effective_approval_id}");
        }

        let parsed_cmd = parse_command(&command);
        let proposed_network_policy_amendments = network_approval_context.as_ref().map(|context| {
            vec![
                NetworkPolicyAmendment {
                    host: context.host.clone(),
                    action: NetworkPolicyRuleAction::Allow,
                },
                NetworkPolicyAmendment {
                    host: context.host.clone(),
                    action: NetworkPolicyRuleAction::Deny,
                },
            ]
        });
        let available_decisions = available_decisions.unwrap_or_else(|| {
            ExecApprovalRequestEvent::default_available_decisions(
                network_approval_context.as_ref(),
                proposed_execpolicy_amendment.as_ref(),
                proposed_network_policy_amendments.as_deref(),
                additional_permissions.as_ref(),
            )
        });
        let event = EventMsg::ExecApprovalRequest(ExecApprovalRequestEvent {
            call_id,
            approval_id,
            turn_id: turn_context.sub_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            command,
            cwd,
            reason,
            network_approval_context,
            proposed_execpolicy_amendment,
            proposed_network_policy_amendments,
            additional_permissions,
            available_decisions: Some(available_decisions),
            parsed_cmd,
        });
        self.send_event(turn_context, event).await;
        rx_approve.await.unwrap_or(ReviewDecision::Abort)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn request_patch_approval(
        &self,
        turn_context: &TurnContext,
        call_id: String,
        changes: HashMap<PathBuf, FileChange>,
        reason: Option<String>,
        grant_root: Option<PathBuf>,
    ) -> oneshot::Receiver<ReviewDecision> {
        // Add the tx_approve callback to the map before sending the request.
        let (tx_approve, rx_approve) = oneshot::channel();
        let approval_id = call_id.clone();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_approval(approval_id.clone(), tx_approve)
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            warn!("Overwriting existing pending approval for call_id: {approval_id}");
        }

        let event = EventMsg::ApplyPatchApprovalRequest(ApplyPatchApprovalRequestEvent {
            call_id,
            turn_id: turn_context.sub_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            changes,
            reason,
            grant_root,
        });
        self.send_event(turn_context, event).await;
        rx_approve
    }

    pub async fn request_permissions(
        self: &Arc<Self>,
        turn_context: &Arc<TurnContext>,
        call_id: String,
        args: RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> Option<RequestPermissionsResponse> {
        self.request_permissions_for_cwd(
            turn_context,
            call_id,
            args,
            #[allow(deprecated)]
            turn_context.cwd.clone(),
            cancellation_token,
        )
        .await
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub(crate) async fn request_permissions_for_cwd(
        self: &Arc<Self>,
        turn_context: &Arc<TurnContext>,
        call_id: String,
        args: RequestPermissionsArgs,
        cwd: AbsolutePathBuf,
        cancellation_token: CancellationToken,
    ) -> Option<RequestPermissionsResponse> {
        match turn_context.as_ref().approval_policy.value() {
            AskForApproval::Never => {
                return Some(RequestPermissionsResponse {
                    permissions: RequestPermissionProfile::default(),
                    scope: PermissionGrantScope::Turn,
                    strict_auto_review: false,
                });
            }
            AskForApproval::Granular(granular_config)
                if !granular_config.allows_request_permissions() =>
            {
                return Some(RequestPermissionsResponse {
                    permissions: RequestPermissionProfile::default(),
                    scope: PermissionGrantScope::Turn,
                    strict_auto_review: false,
                });
            }
            AskForApproval::OnFailure
            | AskForApproval::OnRequest
            | AskForApproval::UnlessTrusted
            | AskForApproval::Granular(_) => {}
        }

        let requested_permissions = args.permissions;

        if routes_approval_to_guardian(
            &turn_context.approval_policy.value(),
            turn_context.config.approvals_reviewer,
        ) {
            let originating_turn_state = {
                let active = self.active_turn.lock().await;
                active.as_ref().map(|active| Arc::clone(&active.turn_state))
            };
            let request = codex_guardian::GuardianApprovalRequest::RequestPermissions {
                id: call_id,
                turn_id: turn_context.sub_id.clone(),
                reason: args.reason,
                permissions: requested_permissions.clone(),
            };
            let decision = self
                .services
                .approval_service
                .review_guardian_request(GuardianReviewDispatch {
                    session: Arc::clone(self)
                        as Arc<dyn codex_approval_service_api::ApprovalSessionCapability>,
                    turn: Arc::clone(turn_context)
                        as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                    review_id: uuid::Uuid::new_v4().to_string(),
                    request,
                    retry_reason: None,
                    approval_request_source:
                        codex_analytics_api::GuardianApprovalRequestSource::MainTurn,
                    cancellation_token: Some(cancellation_token.clone()),
                })
                .await
                .decision;
            let response = match decision {
                ReviewDecision::Approved | ReviewDecision::ApprovedExecpolicyAmendment { .. } => {
                    RequestPermissionsResponse {
                        permissions: requested_permissions.clone(),
                        scope: PermissionGrantScope::Turn,
                        strict_auto_review: false,
                    }
                }
                ReviewDecision::ApprovedForSession => RequestPermissionsResponse {
                    permissions: requested_permissions.clone(),
                    scope: PermissionGrantScope::Session,
                    strict_auto_review: false,
                },
                ReviewDecision::NetworkPolicyAmendment {
                    network_policy_amendment,
                } => match network_policy_amendment.action {
                    NetworkPolicyRuleAction::Allow => RequestPermissionsResponse {
                        permissions: requested_permissions.clone(),
                        scope: PermissionGrantScope::Turn,
                        strict_auto_review: false,
                    },
                    NetworkPolicyRuleAction::Deny => RequestPermissionsResponse {
                        permissions: RequestPermissionProfile::default(),
                        scope: PermissionGrantScope::Turn,
                        strict_auto_review: false,
                    },
                },
                ReviewDecision::Abort | ReviewDecision::Denied | ReviewDecision::TimedOut => {
                    RequestPermissionsResponse {
                        permissions: RequestPermissionProfile::default(),
                        scope: PermissionGrantScope::Turn,
                        strict_auto_review: false,
                    }
                }
            };
            let response = normalize_request_permissions_response(
                requested_permissions,
                response,
                cwd.as_path(),
            );
            self.record_granted_request_permissions_for_turn(
                &response,
                originating_turn_state.as_ref(),
            )
            .await;
            return Some(response);
        }

        let (tx_response, rx_response) = oneshot::channel();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_request_permissions(
                        call_id.clone(),
                        PendingRequestPermissions {
                            tx_response,
                            requested_permissions: requested_permissions.clone(),
                            cwd: cwd.clone(),
                        },
                    )
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            warn!("Overwriting existing pending request_permissions for call_id: {call_id}");
        }

        let event = EventMsg::RequestPermissions(RequestPermissionsEvent {
            call_id: call_id.clone(),
            turn_id: turn_context.sub_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            reason: args.reason,
            permissions: requested_permissions,
            cwd: Some(cwd),
        });
        self.send_event(turn_context.as_ref(), event).await;
        tokio::select! {
            biased;
            _ = cancellation_token.cancelled() => {
                let mut active = self.active_turn.lock().await;
                if let Some(at) = active.as_mut() {
                    let mut ts = at.turn_state.lock().await;
                    let _ = ts.remove_pending_request_permissions(&call_id);
                }
                None
            }
            response = rx_response => response.ok(),
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn request_user_input(
        &self,
        turn_context: &TurnContext,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> Option<RequestUserInputResponse> {
        let sub_id = turn_context.sub_id.clone();
        let (tx_response, rx_response) = oneshot::channel();
        let event_id = sub_id.clone();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_user_input(sub_id, tx_response)
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            warn!("Overwriting existing pending user input for sub_id: {event_id}");
        }

        let event = EventMsg::RequestUserInput(RequestUserInputEvent {
            call_id,
            turn_id: turn_context.sub_id.clone(),
            questions: args.questions,
        });
        turn_context
            .turn_metadata_state
            .mark_user_input_requested_during_turn();
        self.send_event(turn_context, event).await;
        rx_response.await.ok()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn notify_user_input_response(
        &self,
        sub_id: &str,
        response: RequestUserInputResponse,
    ) {
        let entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.remove_pending_user_input(sub_id)
                }
                None => None,
            }
        };
        match entry {
            Some(tx_response) => {
                tx_response.send(response).ok();
            }
            None => {
                warn!("No pending user input found for sub_id: {sub_id}");
            }
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn notify_request_permissions_response(
        &self,
        call_id: &str,
        response: RequestPermissionsResponse,
    ) {
        let (entry, originating_turn_state) = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    let entry = ts.remove_pending_request_permissions(call_id);
                    let originating_turn_state = entry.as_ref().map(|_| Arc::clone(&at.turn_state));
                    (entry, originating_turn_state)
                }
                None => (None, None),
            }
        };
        match entry {
            Some(entry) => {
                let response = normalize_request_permissions_response(
                    entry.requested_permissions,
                    response,
                    entry.cwd.as_path(),
                );
                self.record_granted_request_permissions_for_turn(
                    &response,
                    originating_turn_state.as_ref(),
                )
                .await;
                entry.tx_response.send(response).ok();
            }
            None => {
                warn!("No pending request_permissions found for call_id: {call_id}");
            }
        }
    }

    pub(crate) async fn record_granted_request_permissions_for_turn(
        &self,
        response: &RequestPermissionsResponse,
        originating_turn_state: Option<&Arc<Mutex<crate::state::TurnState>>>,
    ) {
        if response.permissions.is_empty() {
            return;
        }
        match response.scope {
            PermissionGrantScope::Turn => {
                if let Some(turn_state) = originating_turn_state {
                    let mut ts = turn_state.lock().await;
                    let permissions: AdditionalPermissionProfile =
                        response.permissions.clone().into();
                    ts.record_granted_permissions(permissions);
                    if response.strict_auto_review {
                        ts.enable_strict_auto_review();
                    }
                }
            }
            PermissionGrantScope::Session => {
                let mut state = self.state.lock().await;
                state.record_granted_permissions(response.permissions.clone().into());
            }
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn reads must stay consistent with the matching turn state"
    )]
    pub(crate) async fn granted_turn_permissions(&self) -> Option<AdditionalPermissionProfile> {
        let active = self.active_turn.lock().await;
        let active = active.as_ref()?;
        let ts = active.turn_state.lock().await;
        ts.granted_permissions()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn reads must stay consistent with the matching turn state"
    )]
    pub(crate) async fn strict_auto_review_enabled_for_turn(&self) -> bool {
        let active = self.active_turn.lock().await;
        let Some(active) = active.as_ref() else {
            return false;
        };
        let ts = active.turn_state.lock().await;
        ts.strict_auto_review_enabled()
    }

    pub(crate) async fn granted_session_permissions(&self) -> Option<AdditionalPermissionProfile> {
        let state = self.state.lock().await;
        state.granted_permissions()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn notify_dynamic_tool_response(&self, call_id: &str, response: DynamicToolResponse) {
        let entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.remove_pending_dynamic_tool(call_id)
                }
                None => None,
            }
        };
        match entry {
            Some(tx_response) => {
                tx_response.send(response).ok();
            }
            None => {
                warn!("No pending dynamic tool call found for call_id: {call_id}");
            }
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and dynamic tool response registration must remain atomic"
    )]
    pub(crate) async fn register_pending_dynamic_tool_response(
        &self,
        call_id: String,
        tx_response: oneshot::Sender<DynamicToolResponse>,
    ) -> bool {
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_dynamic_tool(call_id, tx_response)
                }
                None => None,
            }
        };
        prev_entry.is_some()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn notify_approval(&self, approval_id: &str, decision: ReviewDecision) {
        let entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.remove_pending_approval(approval_id)
                }
                None => None,
            }
        };
        match entry {
            Some(tx_approve) => {
                tx_approve.send(decision).ok();
            }
            None => {
                warn!("No pending approval found for call_id: {approval_id}");
            }
        }
    }

    /// Records input items: always append to conversation history and
    /// persist these response items to rollout.
    pub(crate) async fn record_conversation_items(
        &self,
        turn_context: &TurnContext,
        items: &[ResponseItem],
    ) {
        self.record_into_history(items, turn_context).await;
        self.persist_rollout_response_items(items).await;
        self.send_thread_context_usage_event(turn_context).await;
    }

    pub(crate) async fn record_model_items_and_emit_display_events(
        &self,
        turn_context: &TurnContext,
        items: &[ResponseItem],
    ) {
        let items: Vec<ResponseItem> = items
            .iter()
            .cloned()
            .map(|mut item| {
                if !is_structured_display_response_item(&item) {
                    return item;
                }

                let id = match &mut item {
                    ResponseItem::CommandWait { id, .. }
                    | ResponseItem::CommandWriteStdin { id, .. }
                    | ResponseItem::CommandExecutionNotification { id, .. }
                    | ResponseItem::WorkflowRunProgress { id, .. }
                    | ResponseItem::EventCommandEvent { id, .. }
                    | ResponseItem::EventDrivenTool { id, .. }
                    | ResponseItem::ThreadGoalUpdate { id, .. }
                    | ResponseItem::InterAgentCommunication { id, .. } => id,
                    _ => return item,
                };
                if id.is_none() {
                    *id = Some(format!("response-item-{}", Uuid::new_v4()));
                }
                item
            })
            .collect();
        self.record_conversation_items(turn_context, &items).await;
        self.emit_model_observed_display_events(turn_context, &items)
            .await;
        self.emit_completed_model_item_display_events(turn_context, &items)
            .await;
    }

    /// Append ResponseItems to the in-memory conversation history only.
    pub(crate) async fn record_into_history(
        &self,
        items: &[ResponseItem],
        turn_context: &TurnContext,
    ) {
        let mut state = self.state.lock().await;
        state.record_items(items.iter(), turn_context.truncation_policy);
    }

    pub(crate) async fn maybe_warn_on_server_model_mismatch(
        self: &Arc<Self>,
        turn_context: &Arc<TurnContext>,
        server_model: String,
    ) -> bool {
        let requested_model = turn_context.model_info.slug.clone();
        let server_model_normalized = server_model.to_ascii_lowercase();
        let requested_model_normalized = requested_model.to_ascii_lowercase();
        if server_model_normalized == requested_model_normalized {
            info!("server reported model {server_model} (matches requested model)");
            return false;
        }

        warn!("server reported model {server_model} while requested model was {requested_model}");

        let warning_message = format!(
            "Your account was flagged for potentially high-risk cyber activity and this request was routed to gpt-5.2 as a fallback. To regain access to gpt-5.3-codex, apply for trusted access: {CYBER_VERIFY_URL} or learn more: {CYBER_SAFETY_URL}"
        );

        self.send_event(
            turn_context,
            EventMsg::ModelReroute(ModelRerouteEvent {
                from_model: requested_model.clone(),
                to_model: server_model.clone(),
                reason: ModelRerouteReason::HighRiskCyberActivity,
            }),
        )
        .await;

        self.send_event(
            turn_context,
            EventMsg::Warning(WarningEvent {
                message: warning_message.clone(),
            }),
        )
        .await;
        true
    }

    pub(crate) async fn emit_model_verification(
        self: &Arc<Self>,
        turn_context: &Arc<TurnContext>,
        verifications: Vec<ModelVerification>,
    ) {
        self.send_event(
            turn_context,
            EventMsg::ModelVerification(ModelVerificationEvent { verifications }),
        )
        .await;
    }

    #[allow(dead_code)]
    pub(crate) async fn replace_history(
        &self,
        items: Vec<ResponseItem>,
        reference_context_item: Option<TurnContextItem>,
    ) {
        let mut state = self.state.lock().await;
        state.replace_history(items, reference_context_item);
    }

    pub(crate) async fn replace_compacted_history(
        &self,
        items: Vec<ResponseItem>,
        reference_context_item: Option<TurnContextItem>,
        compacted_item: CompactedItem,
    ) {
        {
            let mut state = self.state.lock().await;
            state.replace_history_with_compact_window_start(
                items.clone(),
                reference_context_item.clone(),
                Some(items.len()),
            );
        }

        self.persist_rollout_items(&[RolloutItem::Compacted(compacted_item)])
            .await;
        if let Some(turn_context_item) = reference_context_item {
            self.persist_rollout_items(&[RolloutItem::TurnContext(turn_context_item)])
                .await;
        }
        self.services.model_client_api.advance_window_generation();
    }

    async fn persist_rollout_response_items(&self, items: &[ResponseItem]) {
        let rollout_items: Vec<RolloutItem> = items
            .iter()
            .cloned()
            .map(RolloutItem::ResponseItem)
            .collect();
        self.persist_rollout_items(&rollout_items).await;
    }

    pub fn enabled(&self, feature: Feature) -> bool {
        self.features.enabled(feature)
    }

    pub(crate) async fn collaboration_mode(&self) -> CollaborationMode {
        let state = self.state.lock().await;
        state.session_configuration.collaboration_mode.clone()
    }

    async fn emit_completed_model_item_display_events(
        &self,
        turn_context: &TurnContext,
        items: &[ResponseItem],
    ) {
        for item in items {
            if !is_structured_display_response_item(item) {
                continue;
            }

            self.emit_completed_model_item_display_event(turn_context, item)
                .await;
        }
    }

    pub(crate) async fn queue_model_observed_display_event(
        &self,
        item_id: String,
        event: EventMsg,
    ) {
        self.model_observed_display_events
            .lock()
            .await
            .entry(item_id)
            .or_default()
            .push(event);
    }

    async fn emit_model_observed_display_events(
        &self,
        turn_context: &TurnContext,
        items: &[ResponseItem],
    ) {
        let mut events = Vec::new();
        {
            let mut pending = self.model_observed_display_events.lock().await;
            for item in items {
                if let Some(item_id) = response_item_id(item)
                    && let Some(mut item_events) = pending.remove(item_id)
                {
                    events.append(&mut item_events);
                }
            }
        }
        for event in events {
            self.send_event(turn_context, event).await;
        }
    }

    async fn emit_completed_model_item_display_event(
        &self,
        turn_context: &TurnContext,
        item: &ResponseItem,
    ) {
        let now = now_unix_timestamp_ms();
        let event = match completed_display_event_from_model_item(
            self.conversation_id,
            turn_context.sub_id.clone(),
            item,
            now,
        ) {
            Some(event) => event,
            None => EventMsg::ResponseItemCompleted(ResponseItemCompletedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item: item.clone(),
                completed_at_ms: now,
            }),
        };
        self.send_event(turn_context, event).await;
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "MCP app context rendering reads through the session-owned manager guard"
    )]
    pub(crate) async fn build_initial_context(
        &self,
        turn_context: &TurnContext,
    ) -> Vec<ResponseItem> {
        self.build_initial_context_with_external_agent_tool_specs(turn_context, &[])
            .await
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "MCP app context rendering reads through the session-owned manager guard"
    )]
    async fn build_initial_context_with_external_agent_tool_specs(
        &self,
        turn_context: &TurnContext,
        external_agent_tool_specs: &[tool_service_api::ToolSpec],
    ) -> Vec<ResponseItem> {
        let mut developer_sections = Vec::<String>::with_capacity(8);
        let mut contextual_user_sections = Vec::<String>::with_capacity(2);
        let mut separate_developer_sections = Vec::<String>::new();
        let (
            reference_context_item,
            previous_turn_settings,
            collaboration_mode,
            base_instructions,
            session_source,
            root_agent_metadata,
        ) = {
            let state = self.state.lock().await;
            (
                state.reference_context_item(),
                state.previous_turn_settings(),
                state.session_configuration.collaboration_mode.clone(),
                state.session_configuration.base_instructions.clone(),
                state.session_configuration.session_source.clone(),
                state.session_configuration.root_agent_metadata.clone(),
            )
        };
        if let Some(model_switch_message) =
            codex_context_manager::build_model_instructions_update_item(
                previous_turn_settings_view(previous_turn_settings.as_ref()),
                &turn_context.model_info,
                turn_context.personality,
            )
        {
            developer_sections.push(model_switch_message);
        }
        if turn_context.config.include_permissions_instructions {
            developer_sections.push(
                PermissionsInstructions::from_permission_profile(
                    &turn_context.permission_profile,
                    turn_context.approval_policy.value(),
                    turn_context.config.approvals_reviewer,
                    self.services.exec_policy.current().as_ref(),
                    #[allow(deprecated)]
                    &turn_context.cwd,
                    turn_context
                        .features
                        .enabled(Feature::ExecPermissionApprovals),
                    turn_context
                        .features
                        .enabled(Feature::RequestPermissionsTool),
                )
                .render(),
            );
        }
        let separate_guardian_developer_message = is_guardian_reviewer_source(&session_source);
        // Keep the guardian policy prompt out of the aggregated developer bundle so it
        // stays isolated as its own top-level developer message for guardian subagents.
        if !separate_guardian_developer_message
            && let Some(developer_instructions) = turn_context.developer_instructions.as_deref()
            && !developer_instructions.is_empty()
        {
            developer_sections.push(developer_instructions.to_string());
        }
        if !separate_guardian_developer_message
            && let Some(agent_role_instructions) = self
                .current_agent_role_developer_instructions(turn_context, &session_source)
                .await
            && !developer_instructions_contains_section(
                turn_context.developer_instructions.as_deref(),
                &agent_role_instructions,
            )
        {
            developer_sections.push(agent_role_instructions);
        }
        // Add developer instructions for memories.
        if turn_context.features.enabled(Feature::MemoryTool)
            && turn_context.config.memories.use_memories
            && let Some(memory_prompt) = self
                .services
                .memory_tool_developer_instructions_provider
                .build_memory_tool_developer_instructions(&turn_context.config.codex_home)
                .await
        {
            developer_sections.push(memory_prompt);
        }
        // Add developer instructions from collaboration_mode if they exist and are non-empty
        if turn_context.config.include_collaboration_mode_instructions
            && let Some(collab_instructions) =
                CollaborationModeInstructions::from_collaboration_mode(&collaboration_mode)
        {
            developer_sections.push(collab_instructions.render());
        }
        if let Some(realtime_update) = codex_context_manager::build_initial_realtime_item(
            reference_context_item.as_ref(),
            previous_turn_settings_view(previous_turn_settings.as_ref()),
            turn_context.realtime_active,
            turn_context
                .config
                .experimental_realtime_start_instructions
                .as_deref(),
        ) {
            developer_sections.push(realtime_update);
        }
        if self.features.enabled(Feature::Personality)
            && let Some(personality) = turn_context.personality
        {
            let model_info = turn_context.model_info.clone();
            let has_baked_personality = model_info.supports_personality()
                && base_instructions == model_info.get_model_instructions(Some(personality));
            if !has_baked_personality
                && let Some(personality_message) =
                    codex_context_manager::personality_message_for(&model_info, personality)
            {
                developer_sections
                    .push(PersonalitySpecInstructions::new(personality_message).render());
            }
        }
        if turn_context.config.include_apps_instructions && turn_context.apps_enabled() {
            let mcp_connection_manager = self.services.mcp_connection_manager.read().await;
            let all_mcp_tools = mcp_connection_manager.list_all_tools().await;
            let accessible_and_enabled_connectors = self
                .services
                .mcp_service
                .list_accessible_and_enabled_connectors(&all_mcp_tools, &turn_context.config);
            if let Some(apps_instructions) =
                AppsInstructions::from_connectors(&accessible_and_enabled_connectors)
            {
                developer_sections.push(apps_instructions.render());
            }
        }
        if turn_context.config.include_skill_instructions {
            let available_skills = build_available_skills(
                &turn_context.turn_skills.outcome,
                default_skill_metadata_budget(turn_context.model_info.context_window),
                SkillRenderSideEffects::ThreadStart {
                    session_telemetry: self.services.session_telemetry.as_ref(),
                },
            );
            if let Some(available_skills) = available_skills {
                let warning_message = available_skills.warning_message.clone();
                let skills_instructions = AvailableSkillsInstructions::from(available_skills);
                if let Some(warning_message) = warning_message {
                    self.send_event_raw(Event {
                        id: String::new(),
                        msg: EventMsg::Warning(WarningEvent {
                            message: warning_message,
                        }),
                    })
                    .await;
                }
                developer_sections.push(skills_instructions.render());
            }
        }
        let workflow_registry = self.init_context_workflow_registry(turn_context);
        if let Some(workflow_instructions) =
            AvailableWorkflowsInstructions::from_registry(&workflow_registry)
        {
            developer_sections.push(workflow_instructions.render());
        }
        if let Some(agent_instructions) =
            AvailableAgentsInstructions::from_agent_roles(&turn_context.config.agent_roles)
        {
            developer_sections.push(agent_instructions.render());
        }
        if let Some(tool_specs_section) =
            Self::external_agent_tool_specs_context_section(external_agent_tool_specs)
        {
            developer_sections.push(tool_specs_section);
        }
        let plugin_capability_summaries = self
            .services
            .plugins_manager
            .capability_summaries_for_config(&turn_context.config.plugins_config_input())
            .await;
        if let Some(plugin_instructions) =
            AvailablePluginsInstructions::from_plugins(&plugin_capability_summaries)
        {
            developer_sections.push(plugin_instructions.render());
        }
        let context_contributors = self.services.extensions.context_contributors().to_vec();
        for contributor in context_contributors {
            for fragment in contributor
                .contribute(
                    &self.services.session_extension_data,
                    &self.services.thread_extension_data,
                )
                .await
            {
                match fragment.slot() {
                    PromptSlot::DeveloperPolicy | PromptSlot::DeveloperCapabilities => {
                        developer_sections.push(fragment.text().to_string());
                    }
                    PromptSlot::ContextualUser => {
                        contextual_user_sections.push(fragment.text().to_string());
                    }
                    PromptSlot::SeparateDeveloper => {
                        separate_developer_sections.push(fragment.text().to_string());
                    }
                }
            }
        }
        if let Some(user_instructions) = turn_context.user_instructions.as_deref() {
            contextual_user_sections.push(
                UserInstructions {
                    text: user_instructions.to_string(),
                    #[allow(deprecated)]
                    directory: turn_context.cwd.to_string_lossy().into_owned(),
                }
                .render(),
            );
        }
        if turn_context.config.include_environment_context {
            let shell = self.user_shell();
            contextual_user_sections.push(
                crate::context::environment_context_from_turn_context(turn_context, shell.as_ref())
                    .render(),
            );
            contextual_user_sections.push(
                MultiagentContext::new(
                    codex_agent_runtime::current_agent_path_for_session(
                        &session_source,
                        self.services
                            .agent_control
                            .get_agent_metadata(self.conversation_id)
                            .as_ref()
                            .or(root_agent_metadata.as_ref()),
                    ),
                    self.services
                        .agent_control
                        .direct_subagent_paths(self.conversation_id)
                        .await,
                )
                .render(),
            );
            let running_commands = self
                .services
                .command_service_state
                .running_processes_for_thread(self.conversation_id)
                .await;
            let active_subscriptions =
                codex_file_subscription::active_subscriptions_from_thread_store(
                    &self.services.thread_extension_data,
                )
                .await
                .unwrap_or_default();
            let runtime_activity = crate::context::RuntimeActivityContext {
                running_commands,
                active_subscriptions,
            };
            if !runtime_activity.is_empty() {
                contextual_user_sections.push(runtime_activity.render());
            }
        }

        let multi_agent_v2_usage_hint_text =
            multi_agents::usage_hint_text(turn_context, &session_source);

        let mut items = Vec::with_capacity(4);
        if let Some(developer_message) =
            codex_context_manager::build_developer_update_item(developer_sections)
        {
            items.push(developer_message);
        }
        for section in separate_developer_sections {
            if let Some(developer_message) =
                codex_context_manager::build_developer_update_item(vec![section])
            {
                items.push(developer_message);
            }
        }
        if let Some(usage_hint_text) = multi_agent_v2_usage_hint_text
            && let Some(usage_hint_message) =
                codex_context_manager::build_developer_update_item(vec![
                    usage_hint_text.to_string(),
                ])
        {
            items.push(usage_hint_message);
        }
        if let Some(contextual_user_message) =
            codex_context_manager::build_contextual_user_message(contextual_user_sections)
        {
            items.push(contextual_user_message);
        }
        // Emit the guardian policy prompt as a separate developer item so the guardian
        // subagent sees a distinct, easy-to-audit instruction block.
        if separate_guardian_developer_message
            && let Some(developer_instructions) = turn_context.developer_instructions.as_deref()
            && !developer_instructions.is_empty()
            && let Some(guardian_developer_message) =
                codex_context_manager::build_developer_update_item(vec![
                    developer_instructions.to_string(),
                ])
        {
            items.push(guardian_developer_message);
        }
        items
    }

    pub(crate) async fn persist_rollout_items(&self, items: &[RolloutItem]) {
        if let Some(live_thread) = self.live_thread()
            && let Err(e) = live_thread.append_items(items).await
        {
            error!("failed to record rollout items: {e:#}");
        }
    }

    pub(crate) async fn clone_history(&self) -> ContextManager {
        let state = self.state.lock().await;
        state.clone_history()
    }

    pub(crate) async fn remove_oldest_history_item(&self) {
        let mut state = self.state.lock().await;
        state.history.remove_first_item();
    }

    pub(crate) async fn compact_window_items(&self) -> Vec<ResponseItem> {
        let state = self.state.lock().await;
        state.compact_window_items()
    }

    pub(crate) async fn reference_context_item(&self) -> Option<TurnContextItem> {
        let state = self.state.lock().await;
        state.reference_context_item()
    }

    /// Persist the latest turn context snapshot for the first real user turn and for
    /// steady-state turns that emit model-visible context updates.
    ///
    /// When the reference snapshot is missing, this injects full initial context. Otherwise, it
    /// emits only settings diff items.
    ///
    /// If full context is injected and a model switch occurred, this prepends the
    /// `<model_switch>` developer message so model-specific instructions are not lost.
    ///
    /// This is the normal runtime path that establishes a new `reference_context_item`.
    /// Mid-turn compaction is the other path that can re-establish that baseline when it
    /// reinjects full initial context into replacement history. Other non-regular tasks
    /// intentionally do not update the baseline.
    pub(crate) async fn record_context_updates_and_set_reference_context_item(
        &self,
        turn_context: &TurnContext,
    ) {
        let reference_context_item = {
            let state = self.state.lock().await;
            state.reference_context_item()
        };
        let should_inject_full_context = reference_context_item.is_none();
        let context_items = if should_inject_full_context {
            self.build_initial_context_for_external_agent_tools(turn_context)
                .await
        } else {
            // Steady-state path: append only context diffs to minimize token overhead.
            self.build_settings_update_items(reference_context_item.as_ref(), turn_context)
                .await
        };
        let turn_context_item = self.reference_context_item_for_turn(turn_context).await;
        if !context_items.is_empty() {
            self.record_conversation_items(turn_context, &context_items)
                .await;
            if should_inject_full_context
                && let Some(item) = injected_context_item_from_response_items(&context_items)
            {
                self.emit_turn_item_completed(turn_context, item).await;
            }
        }
        // Persist one `TurnContextItem` per real user turn so resume/lazy replay can recover the
        // latest durable baseline even when this turn emitted no model-visible context diffs.
        self.persist_rollout_items(&[RolloutItem::TurnContext(turn_context_item.clone())])
            .await;

        // Advance the in-memory diff baseline even when this turn emitted no model-visible
        // context items. This keeps later runtime diffing aligned with the current turn state.
        let mut state = self.state.lock().await;
        state.set_reference_context_item(Some(turn_context_item));
    }

    pub(crate) async fn record_token_usage_info(
        &self,
        turn_context: &TurnContext,
        token_usage: Option<&TokenUsage>,
    ) {
        if let Some(token_usage) = token_usage {
            let token_info = {
                let mut state = self.state.lock().await;
                state
                    .update_token_info_from_usage(token_usage, turn_context.model_context_window());
                state.token_info()
            };
            if let Some(token_info) = token_info.as_ref() {
                for contributor in self.services.extensions.token_usage_contributors() {
                    contributor.on_token_usage(
                        &self.services.session_extension_data,
                        &self.services.thread_extension_data,
                        turn_context.extension_data.as_ref(),
                        token_info,
                    );
                }
            }
        }
    }

    pub(crate) async fn recompute_token_usage(&self, turn_context: &TurnContext) {
        let history = self.clone_history().await;
        let base_instructions = self.get_base_instructions().await;
        let Some(estimated_total_tokens) =
            history.estimate_token_count_with_base_instructions(&base_instructions)
        else {
            return;
        };
        {
            let mut state = self.state.lock().await;
            let mut info = state.token_info().unwrap_or(TokenUsageInfo {
                total_token_usage: TokenUsage::default(),
                last_token_usage: TokenUsage::default(),
                model_context_window: None,
            });

            info.last_token_usage = TokenUsage {
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                reasoning_output_tokens: 0,
                total_tokens: estimated_total_tokens.max(0),
            };

            if let Some(model_context_window) = turn_context.model_context_window() {
                info.model_context_window = Some(model_context_window);
            }

            state.set_token_info(Some(info));
        }
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn update_rate_limits(
        &self,
        turn_context: &TurnContext,
        new_rate_limits: RateLimitSnapshot,
    ) {
        self.record_rate_limits_info(new_rate_limits).await;
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn record_rate_limits_info(&self, new_rate_limits: RateLimitSnapshot) {
        {
            let mut state = self.state.lock().await;
            state.set_rate_limits(new_rate_limits);
        }
    }

    pub(crate) async fn mcp_dependency_prompted(&self) -> HashSet<String> {
        let state = self.state.lock().await;
        state.mcp_dependency_prompted()
    }

    pub(crate) async fn record_mcp_dependency_prompted<I>(&self, names: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut state = self.state.lock().await;
        state.record_mcp_dependency_prompted(names);
    }

    pub async fn dependency_env(&self) -> HashMap<String, String> {
        let state = self.state.lock().await;
        state.dependency_env()
    }

    pub async fn set_dependency_env(&self, values: HashMap<String, String>) {
        let mut state = self.state.lock().await;
        state.set_dependency_env(values);
    }

    pub(crate) async fn set_server_reasoning_included(&self, included: bool) {
        let mut state = self.state.lock().await;
        state.set_server_reasoning_included(included);
    }

    pub(crate) async fn send_token_count_event(&self, turn_context: &TurnContext) {
        let (info, rate_limits) = {
            let state = self.state.lock().await;
            state.token_info_and_rate_limits()
        };
        let event = EventMsg::TokenCount(TokenCountEvent { info, rate_limits });
        self.send_event(turn_context, event).await;
        self.send_thread_context_usage_event(turn_context).await;
    }

    pub(crate) async fn send_thread_context_usage_event(&self, turn_context: &TurnContext) {
        let usage = {
            let state = self.state.lock().await;
            build_thread_context_usage(
                &state.history,
                turn_context,
                &state.thread_skills(),
                state.compact_replacement_history_len(),
            )
        };
        self.send_event(
            turn_context,
            EventMsg::ThreadContextUsageUpdated(ThreadContextUsageUpdatedEvent { usage }),
        )
        .await;
    }

    pub(crate) async fn set_total_tokens_full(&self, turn_context: &TurnContext) {
        if let Some(context_window) = turn_context.model_context_window() {
            let mut state = self.state.lock().await;
            state.set_token_usage_full(context_window);
        }
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn record_response_item_and_emit_turn_item(
        &self,
        turn_context: &TurnContext,
        response_item: ResponseItem,
    ) {
        // Add to conversation history and persist response item to rollout.
        self.record_conversation_items(turn_context, std::slice::from_ref(&response_item))
            .await;

        // Derive a turn item and emit lifecycle events if applicable.
        if let Some(item) = parse_turn_item(&response_item) {
            self.emit_turn_item_started(turn_context, &item).await;
            self.emit_turn_item_completed(turn_context, item).await;
        }
    }

    pub(crate) async fn record_user_prompt_and_emit_turn_item(
        &self,
        turn_context: &TurnContext,
        input: &[UserInput],
        response_item: ResponseItem,
    ) {
        // Persist the user message to history, but emit the turn item from `UserInput` so
        // UI-only `text_elements` are preserved. `ResponseItem::Message` does not carry
        // those spans, and `record_response_item_and_emit_turn_item` would drop them.
        self.record_conversation_items(turn_context, std::slice::from_ref(&response_item))
            .await;
        let turn_item = TurnItem::UserMessage(UserMessageItem::new(input));
        self.emit_turn_item_started(turn_context, &turn_item).await;
        self.emit_turn_item_completed(turn_context, turn_item).await;
        self.ensure_rollout_materialized().await;
    }

    pub(crate) async fn notify_stream_error(
        &self,
        turn_context: &TurnContext,
        message: impl Into<String>,
        codex_error: CodexErr,
    ) {
        let additional_details = codex_error.to_string();
        let codex_error_info = CodexErrorInfo::ResponseStreamDisconnected {
            http_status_code: codex_error.http_status_code_value(),
        };
        let event = EventMsg::StreamError(StreamErrorEvent {
            message: message.into(),
            codex_error_info: Some(codex_error_info),
            additional_details: Some(additional_details),
        });
        self.send_event(turn_context, event).await;
    }
}

fn response_item_id(item: &ResponseItem) -> Option<&str> {
    match item {
        ResponseItem::CommandWait { id, .. }
        | ResponseItem::CommandWriteStdin { id, .. }
        | ResponseItem::CommandExecutionNotification { id, .. }
        | ResponseItem::WorkflowRunProgress { id, .. }
        | ResponseItem::EventCommandEvent { id, .. }
        | ResponseItem::EventDrivenTool { id, .. }
        | ResponseItem::ThreadGoalUpdate { id, .. }
        | ResponseItem::InterAgentCommunication { id, .. } => id.as_deref(),
        _ => None,
    }
}

fn copy_safe_disabled_workflow_root_for_display(
    workflow_root: &std::path::Path,
    project_root: &std::path::Path,
) -> Option<(tempfile::TempDir, std::path::PathBuf)> {
    let canonical_project_root = std::fs::canonicalize(project_root).ok()?;
    let workflow_root_meta = std::fs::symlink_metadata(workflow_root).ok()?;
    if !workflow_root_meta.is_dir() || workflow_root_meta.file_type().is_symlink() {
        return None;
    }
    canonical_path_within_root(workflow_root, &canonical_project_root)?;

    let temp_root = tempfile::tempdir().ok()?;
    let copied_root = temp_root.path().join("workflows");
    std::fs::create_dir_all(&copied_root).ok()?;

    for entry in std::fs::read_dir(workflow_root).ok()? {
        let entry = entry.ok()?;
        let source_path = entry.path();
        let file_type = entry.file_type().ok()?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }

        if canonical_path_within_root(&source_path, &canonical_project_root).is_none() {
            continue;
        }

        let destination = copied_root.join(entry.file_name());
        if copy_directory_without_symlinks(&source_path, &destination, &canonical_project_root)
            .is_err()
        {
            let _ = std::fs::remove_dir_all(&destination);
        }
    }

    Some((temp_root, copied_root))
}

fn copy_directory_without_symlinks(
    source: &std::path::Path,
    destination: &std::path::Path,
    canonical_project_root: &std::path::Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&source_path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(std::io::Error::other("symlink not allowed"));
        }
        if file_type.is_dir() {
            if canonical_path_within_root(&source_path, canonical_project_root).is_none() {
                return Err(std::io::Error::other("directory escapes project root"));
            }
            copy_directory_without_symlinks(
                &source_path,
                &destination_path,
                canonical_project_root,
            )?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        if canonical_path_within_root(&source_path, canonical_project_root).is_none() {
            return Err(std::io::Error::other("file escapes project root"));
        }
        std::fs::copy(&source_path, &destination_path)?;
    }
    Ok(())
}

fn canonical_path_within_root(
    path: &std::path::Path,
    canonical_project_root: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let canonical_path = std::fs::canonicalize(path).ok()?;
    canonical_path
        .starts_with(canonical_project_root)
        .then_some(canonical_path)
}
