use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use codex_approval_service_api::is_guardian_reviewer_source;
use codex_features::Feature;

use crate::SharedTurnDiffTracker;
use crate::SkillInjections;
use crate::SkillLoadOutcome;
use crate::TurnResolvedConfigFactInput;
use crate::build_skill_injections;
use crate::build_turn_resolved_config_fact;
use crate::client_common::Prompt;
use crate::client_common::PromptBuildParams;
use crate::client_common::ResponseEvent;
use crate::client_common::build_prompt;
use crate::collect_env_var_dependencies;
use crate::collect_explicit_skill_mentions;
use crate::compact::InitialContextInjection;
use crate::compact::collect_user_messages;
use crate::compact::run_inline_auto_compact_task;
use crate::emit_thread_skills_update;
use crate::feedback_tags;
use crate::mentions::build_connector_slug_counts;
use crate::mentions::build_skill_name_counts;
use crate::mentions::collect_explicit_plugin_mentions;
use crate::parse_turn_item;
use crate::resolve_skill_dependencies_for_turn;
use crate::session::PreviousTurnSettings;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::stream_events_utils::HandleOutputCtx;
use crate::stream_events_utils::TurnItemContributorPolicy;
use crate::stream_events_utils::finalize_non_tool_response_item;
use crate::stream_events_utils::handle_non_tool_response_item;
use crate::stream_events_utils::handle_output_item_done;
use crate::stream_events_utils::mark_thread_memory_mode_polluted_if_external_context;
use crate::stream_events_utils::record_completed_response_item_with_finalized_facts;
use crate::turn_timing::record_turn_ttft_metric;
use crate::util::backoff;
use crate::util::error_or_panic;
use codex_analytics_api::AppInvocation;
use codex_analytics_api::CompactionPhase;
use codex_analytics_api::CompactionReason;
use codex_analytics_api::InvocationType;
use codex_analytics_api::SkillInvocation;
use codex_analytics_api::build_track_events_context;
use codex_async_utils::OrCancelExt;
use codex_context_manager::ContextualUserFragment;
use codex_context_manager::SkillInstructions;
use codex_core_skills_api::collect_explicit_app_ids_from_skill_items;
use codex_core_skills_api::collect_explicit_app_ids_from_messages;
use codex_core_skills_api::filter_connectors_for_user_messages;
use codex_git_info::get_git_repo_root;
use codex_hooks::PendingInputHookDisposition;
use codex_hooks::emit_hook_completed_events;
use codex_hooks::inspect_pending_input;
use codex_hooks::record_additional_contexts;
use codex_hooks::record_pending_input;
use codex_hooks::run_pending_session_start_hooks;
use codex_hooks::run_user_prompt_submit_hooks;
use codex_hooks_api::HookEvent;
use codex_hooks_api::HookEventAfterAgent;
use codex_hooks_api::HookPayload;
use codex_hooks_api::HookResult;
use codex_model_client::ModelClientSession;
use codex_protocol::config_types::ModeKind;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::items::TurnItem;
use codex_protocol::items::UserMessageItem;
use codex_protocol::items::build_hook_prompt_message;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentMessageContentDeltaEvent;
use codex_protocol::protocol::AgentReasoningSectionBreakEvent;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::CodexErrorInfo;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::PlanDeltaEvent;
use codex_protocol::protocol::ReasoningContentDeltaEvent;
use codex_protocol::protocol::ReasoningRawContentDeltaEvent;
use codex_protocol::protocol::TurnDiffEvent;
use codex_protocol::protocol::WarningEvent;
use codex_protocol::user_input::UserInput;
use plugin_service_api::PluginCapabilitySummary;
use thread_service_api::TurnDiffTracker;
use codex_tool_service_api::ExtensionToolBuildParams;
use codex_tool_service_api::ToolServiceParams;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolName;
use codex_turn_items::AssistantMessageStreamParsers;
use codex_turn_items::ParsedAssistantTextDelta;
use codex_turn_items::PlanModeStreamAction;
use codex_turn_items::PlanModeStreamState;
use codex_turn_items::raw_assistant_output_text_from_item;
use futures::future::BoxFuture;
use futures::prelude::*;
use futures::stream::FuturesOrdered;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;
use tracing::error;
use tracing::field;
use tracing::info;
use tracing::instrument;
use tracing::trace;
use tracing::trace_span;
use tracing::warn;
use crate::turn_plugin_injection::build_plugin_injections;

/// Takes a user message as input and runs a loop where, at each sampling request, the model
/// replies with either:
///
/// - requested function calls
/// - an assistant message
///
/// While it is possible for the model to return multiple of these items in a
/// single sampling request, in practice, we generally one item per sampling request:
///
/// - If the model requests a function call, we execute it and send the output
///   back to the model in the next sampling request.
/// - If the model sends only an assistant message, we record it in the
///   conversation history and consider the turn complete.
///
#[expect(
    clippy::await_holding_invalid_type,
    reason = "turn execution must keep active-turn state transitions atomic"
)]
pub(crate) async fn run_turn(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    turn_extension_data: Arc<codex_extension_api::ExtensionData>,
    input: Vec<UserInput>,
    prewarmed_client_session: Option<ModelClientSession>,
    cancellation_token: CancellationToken,
) -> Option<String> {
    if input.is_empty() && !sess.has_pending_input().await {
        return None;
    }

    let model_info = turn_context.model_info.clone();
    let auto_compact_limit = model_info.auto_compact_token_limit().unwrap_or(i64::MAX);
    let turn_provider = turn_context.provider.info();
    let prewarmed_client_session = prewarmed_client_session
        .filter(|_| sess.services.model_client.provider_info() == turn_provider);
    let mut client_session = prewarmed_client_session.unwrap_or_else(|| {
        sess.services.model_client.new_session_for_provider(
            sess.services.model_client.auth_manager(),
            turn_provider.clone(),
        )
    });
    // TODO(ccunningham): Pre-turn compaction runs before context updates and the
    // new user message are recorded. Estimate pending incoming items (context
    // diffs/full reinjection + user input) and trigger compaction preemptively
    // when they would push the thread over the compaction threshold.
    let pre_sampling_compact =
        match run_pre_sampling_compact(&sess, &turn_context, &mut client_session).await {
            Ok(pre_sampling_compact) => pre_sampling_compact,
            Err(_) => {
                error!("Failed to run pre-sampling compact");
                return None;
            }
        };
    if pre_sampling_compact.reset_client_session {
        client_session.reset_websocket_session();
    }

    let skills_outcome = Some(turn_context.turn_skills.outcome.as_ref());

    sess.record_context_updates_and_set_reference_context_item(turn_context.as_ref())
        .await;

    let plugin_capability_summaries = sess
        .services
        .plugins_manager
        .capability_summaries_for_config(&turn_context.config.plugins_config_input())
        .await;
    // Structured plugin:// mentions are resolved from the current session's
    // enabled plugins, then converted into turn-scoped guidance below.
    let mentioned_plugins = collect_explicit_plugin_mentions(&input, &plugin_capability_summaries);
    let mcp_tools = if turn_context.apps_enabled() || !mentioned_plugins.is_empty() {
        // Plugin mentions need raw MCP/app inventory even when app tools
        // are normally hidden so we can describe the plugin's currently
        // usable capabilities for this turn.
        match sess
            .services
            .mcp_connection_manager
            .read()
            .await
            .list_all_tools()
            .or_cancel(&cancellation_token)
            .await
        {
            Ok(mcp_tools) => mcp_tools,
            Err(_) if turn_context.apps_enabled() => return None,
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let available_connectors = if turn_context.apps_enabled() {
        sess.services
            .mcp_service
            .list_available_connectors(
                sess.services.plugins_manager.as_ref(),
                &mcp_tools,
                &turn_context.config,
            )
            .await
    } else {
        Vec::new()
    };
    let connector_slug_counts = build_connector_slug_counts(&available_connectors);
    let skill_name_counts_lower = skills_outcome
        .as_ref()
        .map_or_else(HashMap::new, |outcome| {
            build_skill_name_counts(&outcome.skills, &outcome.disabled_paths).1
        });
    let mentioned_skills = skills_outcome.as_ref().map_or_else(Vec::new, |outcome| {
        collect_explicit_skill_mentions(
            &input,
            &outcome.skills,
            &outcome.disabled_paths,
            &connector_slug_counts,
        )
    });
    let config = turn_context.config.clone();
    if config
        .features
        .enabled(Feature::SkillEnvVarDependencyPrompt)
    {
        let env_var_dependencies = collect_env_var_dependencies(&mentioned_skills);
        resolve_skill_dependencies_for_turn(&sess, &turn_context, &env_var_dependencies).await;
    }

    sess.services
        .mcp_service
        .maybe_prompt_and_install_mcp_dependencies(
            sess.as_ref(),
            turn_context.as_ref(),
            &turn_context.config,
            &cancellation_token,
            &mentioned_skills,
            Some(sess.mcp_elicitation_reviewer()),
        )
        .await;

    let session_telemetry = turn_context.session_telemetry.clone();
    let thread_id = sess.conversation_id.to_string();
    let tracking = build_track_events_context(
        turn_context.model_info.slug.clone(),
        thread_id,
        turn_context.sub_id.clone(),
    );
    let SkillInjections {
        items: skill_injections,
        warnings: skill_warnings,
        invocations: skill_invocations,
    } = build_skill_injections(
        &mentioned_skills,
        skills_outcome,
        Some(session_telemetry.as_ref()),
    )
    .await;
    let analytics_skill_invocations = skill_invocations
        .iter()
        .map(|invocation| SkillInvocation {
            skill_name: invocation.skill_name.clone(),
            skill_scope: invocation.skill_scope,
            skill_path: invocation.skill_path.clone(),
            plugin_id: invocation.plugin_id.clone(),
            invocation_type: match invocation.invocation_type {
                crate::injection::SkillInvocationType::Explicit => InvocationType::Explicit,
                crate::injection::SkillInvocationType::Implicit => InvocationType::Implicit,
            },
        })
        .collect::<Vec<_>>();
    sess.services
        .analytics_events_client
        .track_skill_invocations(tracking.clone(), analytics_skill_invocations.clone());
    emit_thread_skills_update(
        sess.as_ref(),
        turn_context.as_ref(),
        &analytics_skill_invocations,
    )
    .await;

    for message in skill_warnings {
        sess.send_event(&turn_context, EventMsg::Warning(WarningEvent { message }))
            .await;
    }

    let skill_items: Vec<ResponseItem> = skill_injections
        .iter()
        .map(|skill| ContextualUserFragment::into(SkillInstructions::from(skill)))
        .collect();

    let plugin_items =
        build_plugin_injections(&mentioned_plugins, &mcp_tools, &available_connectors);
    let mentioned_plugin_metadata = mentioned_plugins
        .iter()
        .filter_map(PluginCapabilitySummary::telemetry_metadata)
        .collect::<Vec<_>>();

    let user_messages = input
        .iter()
        .filter_map(|item| match item {
            UserInput::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<String>>();
    let mut explicitly_enabled_connectors = collect_explicit_app_ids_from_messages(
        &user_messages,
        &available_connectors,
        &skill_name_counts_lower,
    );
    explicitly_enabled_connectors.extend(collect_explicit_app_ids_from_skill_items(
        &skill_items,
        &available_connectors,
        &skill_name_counts_lower,
    ));
    let connector_names_by_id = available_connectors
        .iter()
        .map(|connector| (connector.id.as_str(), connector.name.as_str()))
        .collect::<HashMap<&str, &str>>();
    let mentioned_app_invocations = explicitly_enabled_connectors
        .iter()
        .map(|connector_id| AppInvocation {
            connector_id: Some(connector_id.clone()),
            app_name: connector_names_by_id
                .get(connector_id.as_str())
                .map(|name| (*name).to_string()),
            invocation_type: Some(InvocationType::Explicit),
        })
        .collect::<Vec<_>>();

    if run_pending_session_start_hooks(sess.as_ref(), turn_context.as_ref()).await {
        return None;
    }
    let additional_contexts = if input.is_empty() {
        Vec::new()
    } else {
        let initial_input_for_turn: ResponseInputItem =
            codex_model_input::response_input_item_from_user_input(input.clone());
        let response_item: ResponseItem = initial_input_for_turn.clone().into();
        let user_prompt_submit_outcome = run_user_prompt_submit_hooks(
            sess.as_ref(),
            turn_context.as_ref(),
            UserMessageItem::new(&input).message(),
        )
        .await;
        if user_prompt_submit_outcome.should_stop {
            record_additional_contexts(
                sess.as_ref(),
                turn_context.as_ref(),
                user_prompt_submit_outcome.additional_contexts,
            )
            .await;
            return None;
        }
        sess.record_user_prompt_and_emit_turn_item(turn_context.as_ref(), &input, response_item)
            .await;
        user_prompt_submit_outcome.additional_contexts
    };
    sess.services
        .analytics_events_client
        .track_app_mentioned(tracking.clone(), mentioned_app_invocations);
    for plugin in mentioned_plugin_metadata {
        sess.services
            .analytics_events_client
            .track_plugin_used(tracking.clone(), plugin);
    }
    sess.merge_connector_selection(explicitly_enabled_connectors.clone())
        .await;
    record_additional_contexts(sess.as_ref(), turn_context.as_ref(), additional_contexts).await;
    if !input.is_empty() {
        // Track the previous-turn baseline from the regular user-turn path only so
        // standalone tasks (compact/shell/review) cannot suppress future
        // model/realtime injections.
        sess.set_previous_turn_settings(Some(PreviousTurnSettings {
            model: turn_context.model_info.slug.clone(),
            realtime_active: Some(turn_context.realtime_active),
        }))
        .await;
    }
    if !skill_items.is_empty() {
        sess.record_conversation_items(&turn_context, &skill_items)
            .await;
    }
    if !plugin_items.is_empty() {
        sess.record_conversation_items(&turn_context, &plugin_items)
            .await;
    }

    track_turn_resolved_config_analytics(&sess, &turn_context, &input).await;

    let skills_outcome = Some(turn_context.turn_skills.outcome.as_ref());
    let mut last_agent_message: Option<String> = None;
    let mut stop_hook_active = false;
    // Although from the perspective of codex.rs, TurnDiffTracker has the lifecycle of a Task which contains
    // many turns, from the perspective of the user, it is a single turn.
    #[allow(deprecated)]
    let display_root = get_git_repo_root(turn_context.cwd.as_path())
        .unwrap_or_else(|| turn_context.cwd.clone().into_path_buf());
    let turn_diff_tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::with_display_root(
        display_root,
    )));

    // `ModelClientSession` is turn-scoped and caches WebSocket + sticky routing state, so we reuse
    // one instance across retries within this turn.
    // Pending input is drained into history before building the next model request.
    // However, we defer that drain until after sampling in two cases:
    // 1. At the start of a turn, so the fresh user prompt in `input` gets sampled first.
    // 2. After auto-compact, when model/tool continuation needs to resume before any steer.
    let mut can_drain_pending_input = input.is_empty();

    loop {
        if run_pending_session_start_hooks(sess.as_ref(), turn_context.as_ref()).await {
            break;
        }

        // Note that pending_input would be something like a message the user
        // submitted through the UI while the model was running. Though the UI
        // may support this, the model might not.
        let pending_input = if can_drain_pending_input {
            sess.get_pending_input().await
        } else {
            Vec::new()
        };

        let mut blocked_pending_input = false;
        let mut blocked_pending_input_contexts = Vec::new();
        let mut requeued_pending_input = false;
        let mut accepted_pending_input = Vec::new();
        if !pending_input.is_empty() {
            let mut pending_input_iter = pending_input.into_iter();
            while let Some(pending_input_item) = pending_input_iter.next() {
                match inspect_pending_input(
                    sess.as_ref(),
                    turn_context.as_ref(),
                    pending_input_item,
                )
                .await
                {
                    PendingInputHookDisposition::Accepted(pending_input) => {
                        accepted_pending_input.push(*pending_input);
                    }
                    PendingInputHookDisposition::Blocked {
                        additional_contexts,
                    } => {
                        let remaining_pending_input = pending_input_iter.collect::<Vec<_>>();
                        if !remaining_pending_input.is_empty() {
                            let _ = sess.prepend_pending_input(remaining_pending_input).await;
                            requeued_pending_input = true;
                        }
                        blocked_pending_input_contexts = additional_contexts;
                        blocked_pending_input = true;
                        break;
                    }
                }
            }
        }

        let has_accepted_pending_input = !accepted_pending_input.is_empty();
        for pending_input in accepted_pending_input {
            record_pending_input(sess.as_ref(), turn_context.as_ref(), pending_input).await;
        }
        record_additional_contexts(
            sess.as_ref(),
            turn_context.as_ref(),
            blocked_pending_input_contexts,
        )
        .await;

        if blocked_pending_input && !has_accepted_pending_input {
            if requeued_pending_input {
                continue;
            }
            break;
        }

        // Construct the input that we will send to the model.
        let sampling_request_input: Vec<ResponseItem> = {
            sess.clone_history()
                .await
                .for_prompt(&turn_context.model_info.input_modalities)
        };

        let sampling_request_input_messages = sampling_request_input
            .iter()
            .filter_map(|item| match parse_turn_item(item) {
                Some(TurnItem::UserMessage(user_message)) => Some(user_message),
                _ => None,
            })
            .map(|user_message| user_message.message())
            .collect::<Vec<String>>();
        let turn_metadata_header = turn_context.turn_metadata_state.current_header_value();
        match run_sampling_request(SamplingRequest {
            sess: Arc::clone(&sess),
            turn_context: Arc::clone(&turn_context),
            turn_store: Arc::clone(&turn_extension_data),
            turn_diff_tracker: Arc::clone(&turn_diff_tracker),
            client_session: &mut client_session,
            turn_metadata_header: turn_metadata_header.as_deref(),
            input: sampling_request_input,
            explicitly_enabled_connectors: &explicitly_enabled_connectors,
            skills_outcome,
            cancellation_token: cancellation_token.child_token(),
        })
        .await
        {
            Ok(sampling_request_output) => {
                let SamplingRequestResult {
                    needs_follow_up: model_needs_follow_up,
                    last_agent_message: sampling_request_last_agent_message,
                } = sampling_request_output;
                can_drain_pending_input = true;
                let has_pending_input = sess.has_pending_input().await;
                let needs_follow_up = model_needs_follow_up || has_pending_input;
                let total_usage_tokens = sess.get_total_token_usage().await;
                let token_limit_reached = total_usage_tokens >= auto_compact_limit;

                let estimated_token_count =
                    sess.get_estimated_token_count(turn_context.as_ref()).await;

                trace!(
                    turn_id = %turn_context.sub_id,
                    total_usage_tokens,
                    estimated_token_count = ?estimated_token_count,
                    auto_compact_limit,
                    token_limit_reached,
                    model_needs_follow_up,
                    has_pending_input,
                    needs_follow_up,
                    "post sampling token usage"
                );

                // as long as compaction works well in getting us way below the token limit, we shouldn't worry about being in an infinite loop.
                if token_limit_reached && needs_follow_up {
                    let reset_client_session = match run_auto_compact(
                        &sess,
                        &turn_context,
                        &mut client_session,
                        InitialContextInjection::BeforeLastUserMessage,
                        CompactionReason::ContextLimit,
                        CompactionPhase::MidTurn,
                    )
                    .await
                    {
                        Ok(reset_client_session) => reset_client_session,
                        Err(_) => return None,
                    };
                    if reset_client_session {
                        client_session.reset_websocket_session();
                    }
                    can_drain_pending_input = !model_needs_follow_up;
                    continue;
                }

                if !needs_follow_up {
                    last_agent_message = sampling_request_last_agent_message;
                    let stop_hook_permission_mode = match turn_context.approval_policy.value() {
                        AskForApproval::Never => "bypassPermissions",
                        AskForApproval::UnlessTrusted
                        | AskForApproval::OnFailure
                        | AskForApproval::OnRequest
                        | AskForApproval::Granular(_) => "default",
                    }
                    .to_string();
                    let stop_request = codex_hooks_api::StopRequest {
                        session_id: sess.session_id().into(),
                        turn_id: turn_context.sub_id.clone(),
                        #[allow(deprecated)]
                        cwd: turn_context.cwd.clone(),
                        transcript_path: sess.hook_transcript_path().await,
                        model: turn_context.model_info.slug.clone(),
                        permission_mode: stop_hook_permission_mode,
                        stop_hook_active,
                        last_assistant_message: last_agent_message.clone(),
                    };
                    let hooks = sess.hooks();
                    for run in hooks.preview_stop(&stop_request) {
                        sess.send_event(
                            &turn_context,
                            EventMsg::HookStarted(codex_protocol::protocol::HookStartedEvent {
                                turn_id: Some(turn_context.sub_id.clone()),
                                run,
                            }),
                        )
                        .await;
                    }
                    let stop_outcome = hooks.run_stop(stop_request).await;
                    emit_hook_completed_events(
                        sess.as_ref(),
                        turn_context.as_ref(),
                        stop_outcome.hook_events,
                    )
                    .await;
                    if stop_outcome.should_block {
                        if let Some(hook_prompt_message) =
                            build_hook_prompt_message(&stop_outcome.continuation_fragments)
                        {
                            sess.record_response_item_and_emit_turn_item(
                                &turn_context,
                                hook_prompt_message,
                            )
                            .await;
                            stop_hook_active = true;
                            continue;
                        } else {
                            sess.send_event(
                                &turn_context,
                                EventMsg::Warning(WarningEvent {
                                    message: "Stop hook requested continuation without a prompt; ignoring the block.".to_string(),
                                }),
                            )
                            .await;
                        }
                    }
                    if stop_outcome.should_stop {
                        break;
                    }
                    let hook_outcomes = sess
                        .hooks()
                        .dispatch(HookPayload {
                            session_id: sess.session_id().into(),
                            #[allow(deprecated)]
                            cwd: turn_context.cwd.clone(),
                            client: turn_context.app_server_client_name.clone(),
                            triggered_at: chrono::Utc::now(),
                            hook_event: HookEvent::AfterAgent {
                                event: HookEventAfterAgent {
                                    thread_id: sess.conversation_id,
                                    turn_id: turn_context.sub_id.clone(),
                                    input_messages: sampling_request_input_messages,
                                    last_assistant_message: last_agent_message.clone(),
                                },
                            },
                        })
                        .await;

                    let mut abort_message = None;
                    for hook_outcome in hook_outcomes {
                        let hook_name = hook_outcome.hook_name;
                        match hook_outcome.result {
                            HookResult::Success => {}
                            HookResult::FailedContinue(error) => {
                                warn!(
                                    turn_id = %turn_context.sub_id,
                                    hook_name = %hook_name,
                                    error = %error,
                                    "after_agent hook failed; continuing"
                                );
                            }
                            HookResult::FailedAbort(error) => {
                                let message = format!(
                                    "after_agent hook '{hook_name}' failed and aborted turn completion: {error}"
                                );
                                warn!(
                                    turn_id = %turn_context.sub_id,
                                    hook_name = %hook_name,
                                    error = %error,
                                    "after_agent hook failed; aborting operation"
                                );
                                if abort_message.is_none() {
                                    abort_message = Some(message);
                                }
                            }
                        }
                    }
                    if let Some(message) = abort_message {
                        sess.send_event(
                            &turn_context,
                            EventMsg::Error(ErrorEvent {
                                message,
                                codex_error_info: None,
                            }),
                        )
                        .await;
                        return None;
                    }
                    break;
                }
                continue;
            }
            Err(CodexErr::TurnAborted) => {
                // Aborted turn is reported via a different event.
                break;
            }
            Err(CodexErr::InvalidImageRequest()) => {
                {
                    let mut state = sess.state.lock().await;
                    error_or_panic(
                        "Invalid image detected; sanitizing tool output to prevent poisoning",
                    );
                    if state.history.replace_last_turn_images("Invalid image") {
                        continue;
                    }
                }

                let event = EventMsg::Error(ErrorEvent {
                    message: "Invalid image in your last message. Please remove it and try again."
                        .to_string(),
                    codex_error_info: Some(CodexErrorInfo::BadRequest),
                });
                sess.send_event(&turn_context, event).await;
                break;
            }
            Err(e) => {
                info!("Turn error: {e:#}");
                let event = EventMsg::Error(e.to_error_event(/*message_prefix*/ None));
                sess.send_event(&turn_context, event).await;
                // let the user continue the conversation
                break;
            }
        }
    }

    last_agent_message
}

async fn track_turn_resolved_config_analytics(
    sess: &Session,
    turn_context: &TurnContext,
    input: &[UserInput],
) {
    let thread_config = {
        let state = sess.state.lock().await;
        state.session_configuration.thread_config_snapshot()
    };
    let is_first_turn = {
        let mut state = sess.state.lock().await;
        state.take_next_turn_is_first()
    };
    sess.services
        .analytics_events_client
        .track_turn_resolved_config(build_turn_resolved_config_fact(
            TurnResolvedConfigFactInput {
                turn_id: turn_context.sub_id.clone(),
                thread_id: sess.conversation_id.to_string(),
                num_input_images: input
                    .iter()
                    .filter(|item| {
                        matches!(item, UserInput::Image { .. } | UserInput::LocalImage { .. })
                    })
                    .count(),
                ephemeral: thread_config.ephemeral,
                session_source: thread_config.session_source,
                model: turn_context.model_info.slug.clone(),
                model_provider: turn_context.config.model_provider_id.clone(),
                permission_profile: turn_context.permission_profile(),
                #[allow(deprecated)]
                permission_profile_cwd: turn_context.cwd.to_path_buf(),
                reasoning_effort: turn_context.reasoning_effort,
                reasoning_summary: turn_context.reasoning_summary,
                service_tier: turn_context.config.service_tier.clone(),
                approval_policy: turn_context.approval_policy.value(),
                approvals_reviewer: turn_context.config.approvals_reviewer,
                sandbox_network_access: turn_context.network_sandbox_policy().is_enabled(),
                collaboration_mode: turn_context.collaboration_mode.mode,
                personality: turn_context.personality,
                is_first_turn,
            },
        ));
}

struct PreSamplingCompactResult {
    reset_client_session: bool,
}

async fn run_pre_sampling_compact(
    sess: &Arc<Session>,
    turn_context: &Arc<TurnContext>,
    client_session: &mut ModelClientSession,
) -> CodexResult<PreSamplingCompactResult> {
    let total_usage_tokens_before_compaction = sess.get_total_token_usage().await;
    let mut pre_sampling_compacted = maybe_run_previous_model_inline_compact(
        sess,
        turn_context,
        client_session,
        total_usage_tokens_before_compaction,
    )
    .await?;
    let mut reset_client_session = pre_sampling_compacted;
    let total_usage_tokens = sess.get_total_token_usage().await;
    let auto_compact_limit = turn_context
        .model_info
        .auto_compact_token_limit()
        .unwrap_or(i64::MAX);
    // Compact if the total usage tokens are greater than the auto compact limit
    if total_usage_tokens >= auto_compact_limit {
        reset_client_session |= run_auto_compact(
            sess,
            turn_context,
            client_session,
            InitialContextInjection::DoNotInject,
            CompactionReason::ContextLimit,
            CompactionPhase::PreTurn,
        )
        .await?;
        pre_sampling_compacted = true;
    }
    Ok(PreSamplingCompactResult {
        reset_client_session: pre_sampling_compacted && reset_client_session,
    })
}

/// Runs pre-sampling compaction against the previous model when switching to a smaller
/// context-window model.
///
/// Returns `Ok(true)` when compaction ran successfully, `Ok(false)` when compaction was skipped
/// because the model/context-window preconditions were not met, and `Err(_)` only when compaction
/// was attempted and failed.
async fn maybe_run_previous_model_inline_compact(
    sess: &Arc<Session>,
    turn_context: &Arc<TurnContext>,
    client_session: &mut ModelClientSession,
    total_usage_tokens: i64,
) -> CodexResult<bool> {
    let Some(previous_turn_settings) = sess.previous_turn_settings().await else {
        return Ok(false);
    };
    let previous_model_turn_context = Arc::new(
        turn_context
            .with_model(previous_turn_settings.model, &sess.services.models_manager)
            .await,
    );

    let Some(old_context_window) = previous_model_turn_context.model_context_window() else {
        return Ok(false);
    };
    let Some(new_context_window) = turn_context.model_context_window() else {
        return Ok(false);
    };
    let new_auto_compact_limit = turn_context
        .model_info
        .auto_compact_token_limit()
        .unwrap_or(i64::MAX);
    let should_run = total_usage_tokens > new_auto_compact_limit
        && previous_model_turn_context.model_info.slug != turn_context.model_info.slug
        && old_context_window > new_context_window;
    if should_run {
        let _ = run_auto_compact(
            sess,
            &previous_model_turn_context,
            client_session,
            InitialContextInjection::DoNotInject,
            CompactionReason::ModelDownshift,
            CompactionPhase::PreTurn,
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

async fn run_auto_compact(
    sess: &Arc<Session>,
    turn_context: &Arc<TurnContext>,
    client_session: &mut ModelClientSession,
    initial_context_injection: InitialContextInjection,
    reason: CompactionReason,
    phase: CompactionPhase,
) -> CodexResult<bool> {
    run_inline_auto_compact_task(
        Arc::clone(sess),
        Arc::clone(turn_context),
        initial_context_injection,
        reason,
        phase,
    )
    .await?;
    client_session.reset_websocket_session();
    Ok(true)
}

#[allow(deprecated)]
#[instrument(level = "trace",
    skip_all,
    fields(
        turn_id = %request.turn_context.sub_id,
        model = %request.turn_context.model_info.slug,
        cwd = %request.turn_context.cwd.display()
    )
)]
async fn run_sampling_request(request: SamplingRequest<'_>) -> CodexResult<SamplingRequestResult> {
    let SamplingRequest {
        sess,
        turn_context,
        turn_store,
        turn_diff_tracker,
        client_session,
        turn_metadata_header,
        input,
        explicitly_enabled_connectors,
        skills_outcome,
        cancellation_token,
    } = request;
    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&sess) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let tool_inputs = Arc::new(
        built_tools(
            Arc::clone(&sess),
            Arc::clone(&turn_context),
            Arc::downgrade(&session_capability),
            &input,
            explicitly_enabled_connectors,
            skills_outcome,
            &cancellation_token,
        )
        .await?,
    );

    let base_instructions = sess.get_base_instructions().await;
    let _code_mode_worker = crate::code_mode_turn_bridge::start_turn_worker(
        &sess.services.code_mode_service,
        &sess,
        &turn_context,
        Arc::clone(&tool_inputs),
        Arc::clone(&turn_diff_tracker),
    );
    let mut retries = 0;
    let mut initial_input = Some(input);
    loop {
        let prompt_input = if let Some(input) = initial_input.take() {
            input
        } else {
            sess.clone_history()
                .await
                .for_prompt(&turn_context.model_info.input_modalities)
        };
        let prompt = build_prompt(PromptBuildParams {
            input: prompt_input,
            tools: sess
                .services
                .tool_service
                .model_visible_specs(tool_service_request(&sess, &turn_context, &tool_inputs)),
            parallel_tool_calls: turn_context.model_info.supports_parallel_tool_calls,
            base_instructions: base_instructions.clone(),
            personality: turn_context.personality,
            output_schema: turn_context.final_output_json_schema.clone(),
            output_schema_strict: !is_guardian_reviewer_source(&turn_context.session_source),
        });
        let err = match try_run_sampling_request(TrySamplingRequest {
            tool_inputs: Arc::clone(&tool_inputs),
            sess: Arc::clone(&sess),
            turn_context: Arc::clone(&turn_context),
            turn_store: Arc::clone(&turn_store),
            client_session,
            turn_metadata_header,
            turn_diff_tracker: Arc::clone(&turn_diff_tracker),
            prompt: &prompt,
            cancellation_token: cancellation_token.child_token(),
        })
        .await
        {
            Ok(output) => {
                return Ok(output);
            }
            Err(CodexErr::ContextWindowExceeded) => {
                sess.set_total_tokens_full(&turn_context).await;
                return Err(CodexErr::ContextWindowExceeded);
            }
            Err(CodexErr::UsageLimitReached(e)) => {
                let rate_limits = e.rate_limits.clone();
                if let Some(rate_limits) = rate_limits {
                    sess.update_rate_limits(&turn_context, *rate_limits).await;
                }
                return Err(CodexErr::UsageLimitReached(e));
            }
            Err(err) => err,
        };

        if !err.is_retryable() {
            return Err(err);
        }

        // Use the configured provider-specific stream retry budget.
        let max_retries = turn_context.provider.info().stream_max_retries();
        if retries >= max_retries
            && client_session.try_switch_fallback_transport(
                &turn_context.session_telemetry,
                &turn_context.model_info,
            )
        {
            sess.send_event(
                &turn_context,
                EventMsg::Warning(WarningEvent {
                    message: format!("Falling back from WebSockets to HTTPS transport. {err:#}"),
                }),
            )
            .await;
            retries = 0;
            continue;
        }
        if retries < max_retries {
            retries += 1;
            let delay = match &err {
                CodexErr::Stream(_, requested_delay) => {
                    requested_delay.unwrap_or_else(|| backoff(retries))
                }
                _ => backoff(retries),
            };
            warn!(
                "stream disconnected - retrying sampling request ({retries}/{max_retries} in {delay:?})...",
            );

            // In release builds, hide the first websocket retry notification to reduce noisy
            // transient reconnect messages. In debug builds, keep full visibility for diagnosis.
            let report_error = retries > 1
                || cfg!(debug_assertions)
                || !sess.services.model_client.responses_websocket_enabled();
            if report_error {
                // Surface retry information to any UI/front‑end so the
                // user understands what is happening instead of staring
                // at a seemingly frozen screen.
                sess.notify_stream_error(
                    &turn_context,
                    format!("Reconnecting... {retries}/{max_retries}"),
                    err,
                )
                .await;
            }
            tokio::time::sleep(delay).await;
        } else {
            return Err(err);
        }
    }
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "tool router construction reads through the session-owned manager guard"
)]
pub(crate) async fn built_tools(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    session_capability: std::sync::Weak<dyn thread_service_api::ThreadSessionCapability>,
    input: &[ResponseItem],
    explicitly_enabled_connectors: &HashSet<String>,
    skills_outcome: Option<&SkillLoadOutcome>,
    cancellation_token: &CancellationToken,
) -> CodexResult<TurnToolInputs> {
    let mcp_connection_manager = sess.services.mcp_connection_manager.read().await;
    let has_mcp_servers = mcp_connection_manager.has_servers();
    let all_mcp_tools = mcp_connection_manager
        .list_all_tools()
        .or_cancel(cancellation_token)
        .await
        .map_err(|_| CodexErr::TurnAborted)?;
    drop(mcp_connection_manager);
    let mut effective_explicitly_enabled_connectors = explicitly_enabled_connectors.clone();
    effective_explicitly_enabled_connectors.extend(sess.get_connector_selection().await);

    let apps_enabled = turn_context.apps_enabled();
    let accessible_connectors = apps_enabled.then(|| {
        sess.services
            .mcp_service
            .list_accessible_connectors(&all_mcp_tools, &turn_context.config)
    });
    let connectors = if apps_enabled {
        Some(
            sess.services
                .mcp_service
                .list_available_connectors(
                    sess.services.plugins_manager.as_ref(),
                    &all_mcp_tools,
                    &turn_context.config,
                )
                .await,
        )
    } else {
        None
    };
    let discoverable_tools = if apps_enabled && turn_context.tools_config.tool_suggest {
        if let Some(accessible_connectors) = accessible_connectors.as_ref() {
            match sess
                .services
                .mcp_service
                .list_discoverable_tools(
                    turn_context.as_ref(),
                    sess.services.plugins_manager.as_ref(),
                    accessible_connectors.as_slice(),
                    &turn_context.config,
                    turn_context.app_server_client_name.as_deref(),
                    turn_context.tools_config.tool_suggest,
                    apps_enabled,
                )
                .await
            {
                Ok(discoverable_tools) if discoverable_tools.is_empty() => None,
                Ok(discoverable_tools) => Some(discoverable_tools),
                Err(err) => {
                    warn!("failed to load discoverable tool suggestions: {err:#}");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let explicitly_enabled = if let Some(connectors) = connectors.as_ref() {
        let skill_name_counts_lower = skills_outcome.map_or_else(HashMap::new, |outcome| {
            build_skill_name_counts(&outcome.skills, &outcome.disabled_paths).1
        });

        let user_messages = collect_user_messages(input);
        filter_connectors_for_user_messages(
            connectors,
            &user_messages,
            &effective_explicitly_enabled_connectors,
            &skill_name_counts_lower,
        )
    } else {
        Vec::new()
    };
    let mcp_tool_exposure = sess.services.mcp_service.build_tool_exposure(
        &all_mcp_tools,
        connectors.as_deref(),
        explicitly_enabled.as_slice(),
        &turn_context.config,
        &turn_context.tools_config,
    );
    let mcp_tools = has_mcp_servers.then_some(mcp_tool_exposure.direct_tools);
    let deferred_mcp_tools = mcp_tool_exposure.deferred_tools;
    let default_agent_type_description =
        codex_agent_roles::spawn_tool_spec::build(&std::collections::BTreeMap::new());
    Ok(TurnToolInputs {
        session_capability,
        mcp_tools: mcp_tools.unwrap_or_default(),
        deferred_mcp_tools: deferred_mcp_tools.unwrap_or_default(),
        discoverable_tools: discoverable_tools.unwrap_or_default(),
        default_agent_type_description,
    })
}

pub(crate) struct TurnToolInputs {
    pub(crate) session_capability:
        std::sync::Weak<dyn thread_service_api::ThreadSessionCapability>,
    pub(crate) mcp_tools: Vec<codex_mcp_tool_types::ToolInfo>,
    pub(crate) deferred_mcp_tools: Vec<codex_mcp_tool_types::ToolInfo>,
    pub(crate) discoverable_tools: Vec<codex_tool_types::DiscoverableTool>,
    pub(crate) default_agent_type_description: String,
}

pub(crate) fn tool_service_request<'a>(
    sess: &'a Arc<Session>,
    turn_context: &'a Arc<TurnContext>,
    tool_inputs: &'a TurnToolInputs,
) -> codex_tool_service_api::ToolSpecRequest<'a> {
    codex_tool_service_api::ToolSpecRequest {
        config: &turn_context.tools_config,
        session_capability: tool_inputs.session_capability.clone(),
        session: Arc::clone(sess) as Arc<dyn thread_service_api::ThreadSessionCapability>,
        session_command_state: Arc::clone(&sess.services.command_service_state)
            as Arc<dyn codex_command_service_api::CommandServiceSessionState>,
        session_command_interaction: Arc::clone(sess)
            as Arc<dyn codex_command_service_api::SessionCommandInteractionCaller>,
        session_agent_jobs:
            Arc::clone(sess) as Arc<dyn thread_service_api::SessionAgentJobCaller>,
        turn: Arc::clone(turn_context) as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
        params: ToolServiceParams {
            mcp_tools: Some(tool_inputs.mcp_tools.as_slice()),
            deferred_mcp_tools: Some(tool_inputs.deferred_mcp_tools.as_slice()),
            discoverable_tools: Some(tool_inputs.discoverable_tools.as_slice()),
            extension_tools: Some(ExtensionToolBuildParams {
                tool_contributors: sess.services.extensions.tool_contributors(),
                session_store: &sess.services.session_extension_data,
                thread_store: &sess.services.thread_extension_data,
            }),
            dynamic_tools: turn_context.dynamic_tools.as_slice(),
            default_agent_type_description: &tool_inputs.default_agent_type_description,
        },
    }
}

pub(crate) async fn dispatch_tool_call(
    tool_service: Arc<crate::ToolServiceApi>,
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    tool_inputs: Arc<TurnToolInputs>,
    tracker: SharedTurnDiffTracker,
    call: codex_tool_types::ToolCall,
    source: codex_tool_types::ToolCallSource,
    cancellation_token: CancellationToken,
) -> Result<codex_tool_service_api::AnyToolResult, FunctionCallError> {
    tool_service
        .dispatch_tool(codex_tool_service_api::ToolDispatchRequest {
            tool: tool_service_request(&sess, &turn_context, &tool_inputs),
            cancellation_token,
            tracker,
            call,
            source,
        })
        .await
}

pub(crate) async fn handle_tool_call(
    tool_service: Arc<crate::ToolServiceApi>,
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    tool_inputs: Arc<TurnToolInputs>,
    tracker: SharedTurnDiffTracker,
    call: codex_tool_types::ToolCall,
    cancellation_token: CancellationToken,
) -> Result<ResponseItem, CodexErr> {
    let error_call = call.clone();
    match dispatch_tool_call(
        tool_service,
        sess,
        turn_context,
        tool_inputs,
        tracker,
        call,
        codex_tool_types::ToolCallSource::Direct,
        cancellation_token,
    )
    .await
    {
        Ok(response) => Ok(response.into_response().into()),
        Err(FunctionCallError::Fatal(message)) => Err(CodexErr::Fatal(message)),
        Err(other) => Ok(failure_response(error_call, other)),
    }
}

fn failure_response(call: codex_tool_types::ToolCall, err: FunctionCallError) -> ResponseItem {
    let message = err.to_string();
    match call.payload {
        codex_tool_types::ToolPayload::ToolSearch { .. } => ResponseItem::ToolSearchOutput {
            call_id: Some(call.call_id),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: Vec::new(),
        },
        codex_tool_types::ToolPayload::Custom { .. } => ResponseItem::CustomToolCallOutput {
            call_id: call.call_id,
            name: None,
            output: codex_protocol::models::FunctionCallOutputPayload {
                body: codex_protocol::models::FunctionCallOutputBody::Text(message),
                success: Some(false),
            },
        },
        _ => ResponseItem::FunctionCallOutput {
            call_id: call.call_id,
            output: codex_protocol::models::FunctionCallOutputPayload {
                body: codex_protocol::models::FunctionCallOutputBody::Text(message),
                success: Some(false),
            },
        },
    }
}

#[derive(Debug)]
struct SamplingRequestResult {
    needs_follow_up: bool,
    last_agent_message: Option<String>,
}

struct SamplingRequest<'a> {
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    turn_store: Arc<codex_extension_api::ExtensionData>,
    turn_diff_tracker: SharedTurnDiffTracker,
    client_session: &'a mut ModelClientSession,
    turn_metadata_header: Option<&'a str>,
    input: Vec<ResponseItem>,
    explicitly_enabled_connectors: &'a HashSet<String>,
    skills_outcome: Option<&'a SkillLoadOutcome>,
    cancellation_token: CancellationToken,
}

struct TrySamplingRequest<'a> {
    tool_inputs: Arc<TurnToolInputs>,
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    turn_store: Arc<codex_extension_api::ExtensionData>,
    client_session: &'a mut ModelClientSession,
    turn_metadata_header: Option<&'a str>,
    turn_diff_tracker: SharedTurnDiffTracker,
    prompt: &'a Prompt,
    cancellation_token: CancellationToken,
}

async fn emit_plan_mode_stream_actions(
    sess: &Session,
    turn_context: &TurnContext,
    actions: Vec<PlanModeStreamAction>,
) {
    for action in actions {
        match action {
            PlanModeStreamAction::TurnItemStarted(item) => {
                sess.emit_turn_item_started(turn_context, &item).await;
            }
            PlanModeStreamAction::TurnItemCompleted(item) => {
                sess.emit_turn_item_completed(turn_context, item).await;
            }
            PlanModeStreamAction::AgentMessageDelta { item_id, delta } => {
                let event = AgentMessageContentDeltaEvent {
                    thread_id: sess.conversation_id.to_string(),
                    turn_id: turn_context.sub_id.clone(),
                    item_id,
                    delta,
                };
                sess.send_event(turn_context, EventMsg::AgentMessageContentDelta(event))
                    .await;
            }
            PlanModeStreamAction::PlanDelta { item_id, delta } => {
                let event = PlanDeltaEvent {
                    thread_id: sess.conversation_id.to_string(),
                    turn_id: turn_context.sub_id.clone(),
                    item_id,
                    delta,
                };
                sess.send_event(turn_context, EventMsg::PlanDelta(event))
                    .await;
            }
        }
    }
}

async fn emit_streamed_assistant_text_delta(
    sess: &Session,
    turn_context: &TurnContext,
    plan_mode_state: Option<&mut PlanModeStreamState>,
    item_id: &str,
    parsed: ParsedAssistantTextDelta,
) {
    if parsed.is_empty() {
        return;
    }
    if !parsed.citations.is_empty() {
        // Citation extraction is intentionally local for now; we strip citations from display text
        // but do not yet surface them in protocol events.
        let _citations = parsed.citations;
    }
    if let Some(state) = plan_mode_state {
        if !parsed.plan_segments.is_empty() {
            let actions = state.handle_segments(item_id, parsed.plan_segments);
            emit_plan_mode_stream_actions(sess, turn_context, actions).await;
        }
        return;
    }
    if parsed.visible_text.is_empty() {
        return;
    }
    let event = AgentMessageContentDeltaEvent {
        thread_id: sess.conversation_id.to_string(),
        turn_id: turn_context.sub_id.clone(),
        item_id: item_id.to_string(),
        delta: parsed.visible_text,
    };
    sess.send_event(turn_context, EventMsg::AgentMessageContentDelta(event))
        .await;
}

/// Flush buffered assistant text parser state when an assistant message item ends.
async fn flush_assistant_text_segments_for_item(
    sess: &Session,
    turn_context: &TurnContext,
    plan_mode_state: Option<&mut PlanModeStreamState>,
    parsers: &mut AssistantMessageStreamParsers,
    item_id: &str,
) {
    let parsed = parsers.finish_item(item_id);
    emit_streamed_assistant_text_delta(sess, turn_context, plan_mode_state, item_id, parsed).await;
}

/// Flush any remaining buffered assistant text parser state at response completion.
async fn flush_assistant_text_segments_all(
    sess: &Session,
    turn_context: &TurnContext,
    mut plan_mode_state: Option<&mut PlanModeStreamState>,
    parsers: &mut AssistantMessageStreamParsers,
) {
    for (item_id, parsed) in parsers.drain_finished() {
        emit_streamed_assistant_text_delta(
            sess,
            turn_context,
            plan_mode_state.as_deref_mut(),
            &item_id,
            parsed,
        )
        .await;
    }
}

/// Handle a completed assistant response item in plan mode, returning true if handled.
async fn handle_assistant_item_done_in_plan_mode(
    sess: &Session,
    turn_context: &TurnContext,
    turn_store: &codex_extension_api::ExtensionData,
    item: &ResponseItem,
    state: &mut PlanModeStreamState,
    previously_active_item: Option<&TurnItem>,
    last_agent_message: &mut Option<String>,
) -> bool {
    if let ResponseItem::Message { role, .. } = item
        && role == "assistant"
    {
        let actions = state.complete_plan_from_message(item);
        emit_plan_mode_stream_actions(sess, turn_context, actions).await;

        let mut finalized_facts = None;
        if let Some(finalized_turn_item) = finalize_non_tool_response_item(
            sess,
            turn_context,
            TurnItemContributorPolicy::Run(turn_store),
            item,
            /*plan_mode*/ true,
        )
        .await
        {
            finalized_facts = Some(finalized_turn_item.facts.clone());
            let actions =
                state.complete_turn_item(finalized_turn_item.turn_item, previously_active_item);
            emit_plan_mode_stream_actions(sess, turn_context, actions).await;
        }
        let final_last_agent_message = finalized_facts
            .as_ref()
            .and_then(|facts| facts.last_agent_message.clone());

        record_completed_response_item_with_finalized_facts(
            sess,
            turn_context,
            item,
            finalized_facts.as_ref(),
        )
        .await;
        if let Some(agent_message) = final_last_agent_message {
            *last_agent_message = Some(agent_message);
        }
        return true;
    }
    false
}

async fn drain_in_flight(
    in_flight: &mut FuturesOrdered<BoxFuture<'static, CodexResult<ResponseItem>>>,
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
) -> CodexResult<()> {
    while let Some(res) = in_flight.next().await {
        match res {
            Ok(response_item) => {
                sess.record_conversation_items(&turn_context, std::slice::from_ref(&response_item))
                    .await;
                mark_thread_memory_mode_polluted_if_external_context(
                    sess.as_ref(),
                    turn_context.as_ref(),
                    &response_item,
                )
                .await;
            }
            Err(err) => {
                error_or_panic(format!("in-flight tool future failed during drain: {err}"));
            }
        }
    }
    Ok(())
}

#[instrument(level = "trace",
    skip_all,
    fields(
        turn_id = %request.turn_context.sub_id,
        model = %request.turn_context.model_info.slug
    )
)]
async fn try_run_sampling_request(
    request: TrySamplingRequest<'_>,
) -> CodexResult<SamplingRequestResult> {
    let TrySamplingRequest {
        tool_inputs,
        sess,
        turn_context,
        turn_store,
        client_session,
        turn_metadata_header,
        turn_diff_tracker,
        prompt,
        cancellation_token,
    } = request;
    feedback_tags!(
        model = turn_context.model_info.slug.clone(),
        approval_policy = turn_context.approval_policy.value(),
        sandbox_policy = &turn_context.sandbox_policy(),
        effort = turn_context.reasoning_effort,
        auth_mode = turn_context
            .auth_runtime
            .as_deref()
            .and_then(|auth_runtime| auth_runtime.telemetry_snapshot().auth_mode),
        features = sess.features.enabled_features(),
    );
    let inference_trace = sess.services.rollout_thread_trace.inference_trace_context(
        turn_context.sub_id.as_str(),
        turn_context.model_info.slug.as_str(),
        turn_context.provider.info().name.as_str(),
    );
    let mut stream = client_session
        .stream(
            prompt,
            &turn_context.model_info,
            &turn_context.session_telemetry,
            turn_context.reasoning_effort,
            turn_context.reasoning_summary,
            turn_context.config.service_tier.clone(),
            turn_metadata_header,
            &inference_trace,
        )
        .instrument(trace_span!("stream_request"))
        .or_cancel(&cancellation_token)
        .await
        .map_err(|_| CodexErr::TurnAborted)??;
    let mut in_flight: FuturesOrdered<BoxFuture<'static, CodexResult<ResponseItem>>> =
        FuturesOrdered::new();
    let mut needs_follow_up = false;
    let mut last_agent_message: Option<String> = None;
    let mut active_item: Option<TurnItem> = None;
    let mut active_tool_argument_diff_consumer: Option<(
        String,
        Box<dyn codex_tool_service_api::ErasedToolArgumentDiffConsumer>,
    )> = None;
    let mut should_emit_turn_diff = false;
    let mut should_emit_token_count = false;
    let reasoning_effort = turn_context.effective_reasoning_effort_for_tracing();
    let plan_mode = turn_context.collaboration_mode.mode == ModeKind::Plan;
    let mut assistant_message_stream_parsers = AssistantMessageStreamParsers::new(plan_mode);
    let mut plan_mode_state = plan_mode.then(|| PlanModeStreamState::new(&turn_context.sub_id));
    let defer_streamed_turn_items_for_contributors =
        !sess.services.extensions.turn_item_contributors().is_empty();
    let mut active_item_is_streaming_to_client = false;
    let receiving_span = trace_span!("receiving_stream");
    let mut completed_response_id: Option<String> = None;
    let outcome: CodexResult<SamplingRequestResult> = loop {
        let handle_responses = trace_span!(
            parent: &receiving_span,
            "handle_responses",
            otel.name = field::Empty,
            tool_name = field::Empty,
            from = field::Empty,
            codex.request.reasoning_effort = %reasoning_effort,
            gen_ai.usage.input_tokens = field::Empty,
            gen_ai.usage.cache_read.input_tokens = field::Empty,
            gen_ai.usage.output_tokens = field::Empty,
            codex.usage.reasoning_output_tokens = field::Empty,
            codex.usage.total_tokens = field::Empty,
        );

        let event = match stream
            .next()
            .instrument(trace_span!(parent: &handle_responses, "receiving"))
            .or_cancel(&cancellation_token)
            .await
        {
            Ok(event) => event,
            Err(codex_async_utils::CancelErr::Cancelled) => break Err(CodexErr::TurnAborted),
        };

        let event = match event {
            Some(Ok(event)) => event,
            Some(Err(err)) => break Err(err),
            None => {
                break Err(CodexErr::Stream(
                    "stream closed before response.completed".into(),
                    None,
                ));
            }
        };

        sess.services
            .session_telemetry
            .record_responses(&handle_responses, &event);
        record_turn_ttft_metric(&turn_context, &event).await;

        match event {
            ResponseEvent::Created => {}
            ResponseEvent::OutputItemDone(item) => {
                if let Some((_, mut consumer)) = active_tool_argument_diff_consumer.take()
                    && let Ok(Some(event)) = consumer.finish()
                {
                    sess.send_event(&turn_context, event).await;
                }
                let previously_active_item = active_item.take();
                let previously_streamed_item = if active_item_is_streaming_to_client {
                    previously_active_item
                } else {
                    None
                };
                active_item_is_streaming_to_client = false;
                if let Some(previous) = previously_streamed_item.as_ref()
                    && matches!(previous, TurnItem::AgentMessage(_))
                {
                    let item_id = previous.id();
                    flush_assistant_text_segments_for_item(
                        &sess,
                        &turn_context,
                        plan_mode_state.as_mut(),
                        &mut assistant_message_stream_parsers,
                        &item_id,
                    )
                    .await;
                }
                if let Some(state) = plan_mode_state.as_mut()
                    && handle_assistant_item_done_in_plan_mode(
                        &sess,
                        &turn_context,
                        turn_store.as_ref(),
                        &item,
                        state,
                        previously_streamed_item.as_ref(),
                        &mut last_agent_message,
                    )
                    .await
                {
                    continue;
                }

                let mut ctx = HandleOutputCtx {
                    sess: sess.clone(),
                    turn_context: turn_context.clone(),
                    turn_store: Arc::clone(&turn_store),
                    tool_inputs: Arc::clone(&tool_inputs),
                    turn_diff_tracker: Arc::clone(&turn_diff_tracker),
                    cancellation_token: cancellation_token.child_token(),
                };

                let preempt_for_mailbox_mail = match &item {
                    ResponseItem::Message { role, phase, .. } => {
                        role == "assistant" && matches!(phase, Some(MessagePhase::Commentary))
                    }
                    ResponseItem::Reasoning { .. } => true,
                    ResponseItem::LocalShellCall { .. }
                    | ResponseItem::FunctionCall { .. }
                    | ResponseItem::ToolSearchCall { .. }
                    | ResponseItem::FunctionCallOutput { .. }
                    | ResponseItem::CustomToolCall { .. }
                    | ResponseItem::CustomToolCallOutput { .. }
                    | ResponseItem::ToolSearchOutput { .. }
                    | ResponseItem::WebSearchCall { .. }
                    | ResponseItem::ImageGenerationCall { .. }
                    | ResponseItem::CommandWait { .. }
                    | ResponseItem::CommandWriteStdin { .. }
                    | ResponseItem::WorkflowRunProgress { .. }
                    | ResponseItem::CommandExecutionNotification { .. }
                    | ResponseItem::EventCommandEvent { .. }
                    | ResponseItem::EventDrivenTool { .. }
                    | ResponseItem::ThreadGoalUpdate { .. }
                    | ResponseItem::InterAgentCommunication { .. }
                    | ResponseItem::Compaction { .. }
                    | ResponseItem::ContextCompaction { .. }
                    | ResponseItem::Other => false,
                };

                let output_result =
                    match handle_output_item_done(&mut ctx, item, previously_streamed_item)
                        .instrument(handle_responses)
                        .await
                    {
                        Ok(output_result) => output_result,
                        Err(err) => break Err(err),
                    };
                if let Some(tool_future) = output_result.tool_future {
                    in_flight.push_back(tool_future);
                }
                if let Some(agent_message) = output_result.last_agent_message {
                    last_agent_message = Some(agent_message);
                }
                needs_follow_up |= output_result.needs_follow_up;
                // todo: remove before stabilizing multi-agent v2
                if preempt_for_mailbox_mail && sess.mailbox_rx.lock().await.has_pending() {
                    break Ok(SamplingRequestResult {
                        needs_follow_up: true,
                        last_agent_message,
                    });
                }
            }
            ResponseEvent::OutputItemAdded(item) => {
                if let ResponseItem::CustomToolCall { call_id, name, .. } = &item {
                    let tool_name = ToolName::plain(name.as_str());
                    active_tool_argument_diff_consumer = sess
                        .services
                        .tool_service
                        .create_diff_consumer(codex_tool_service_api::ToolDiffConsumerRequest {
                            tool: tool_service_request(&sess, &turn_context, &tool_inputs),
                            tool_name: &tool_name,
                        })
                        .map(|consumer| (call_id.clone(), consumer));
                } else if matches!(&item, ResponseItem::FunctionCall { .. }) {
                    active_tool_argument_diff_consumer = None;
                }
                if let Some(turn_item) = handle_non_tool_response_item(
                    sess.as_ref(),
                    turn_context.as_ref(),
                    TurnItemContributorPolicy::Skip,
                    &item,
                    plan_mode,
                )
                .await
                {
                    let mut turn_item = turn_item;
                    let stream_item_to_client = !defer_streamed_turn_items_for_contributors;
                    let mut seeded_parsed: Option<ParsedAssistantTextDelta> = None;
                    let mut seeded_item_id: Option<String> = None;
                    if stream_item_to_client
                        && matches!(turn_item, TurnItem::AgentMessage(_))
                        && let Some(raw_text) = raw_assistant_output_text_from_item(&item)
                    {
                        let item_id = turn_item.id();
                        let mut seeded =
                            assistant_message_stream_parsers.seed_item_text(&item_id, &raw_text);
                        if let TurnItem::AgentMessage(agent_message) = &mut turn_item {
                            agent_message.content =
                                vec![codex_protocol::items::AgentMessageContent::Text {
                                    text: if plan_mode {
                                        String::new()
                                    } else {
                                        std::mem::take(&mut seeded.visible_text)
                                    },
                                }];
                        }
                        seeded_parsed = plan_mode.then_some(seeded);
                        seeded_item_id = Some(item_id);
                    }
                    if stream_item_to_client {
                        if let Some(state) = plan_mode_state.as_mut()
                            && matches!(turn_item, TurnItem::AgentMessage(_))
                        {
                            let item_id = turn_item.id();
                            state.stage_agent_message_item(item_id, turn_item.clone());
                        } else {
                            sess.emit_turn_item_started(&turn_context, &turn_item).await;
                        }
                        if let (Some(state), Some(item_id), Some(parsed)) = (
                            plan_mode_state.as_mut(),
                            seeded_item_id.as_deref(),
                            seeded_parsed,
                        ) {
                            emit_streamed_assistant_text_delta(
                                &sess,
                                &turn_context,
                                Some(state),
                                item_id,
                                parsed,
                            )
                            .await;
                        }
                    }
                    active_item = Some(turn_item);
                    active_item_is_streaming_to_client = stream_item_to_client;
                }
            }
            ResponseEvent::ServerModel(server_model) => {
                if !turn_context
                    .server_model_warning_emitted
                    .load(Ordering::Relaxed)
                    && sess
                        .maybe_warn_on_server_model_mismatch(&turn_context, server_model)
                        .await
                {
                    turn_context
                        .server_model_warning_emitted
                        .store(true, Ordering::Relaxed);
                }
            }
            ResponseEvent::ModelVerifications(verifications) => {
                if !turn_context
                    .model_verification_emitted
                    .swap(true, Ordering::Relaxed)
                {
                    sess.emit_model_verification(&turn_context, verifications)
                        .await;
                }
            }
            ResponseEvent::ServerReasoningIncluded(included) => {
                sess.set_server_reasoning_included(included).await;
            }
            ResponseEvent::RateLimits(snapshot) => {
                // Update internal state with latest rate limits, but defer sending until
                // token usage is available to avoid duplicate TokenCount events.
                sess.record_rate_limits_info(snapshot).await;
                should_emit_token_count = true;
            }
            ResponseEvent::ModelsEtag(etag) => {
                // Update internal state with latest models etag
                sess.services.models_manager.refresh_if_new_etag(etag).await;
            }
            ResponseEvent::Completed {
                response_id,
                token_usage,
                end_turn,
            } => {
                flush_assistant_text_segments_all(
                    &sess,
                    &turn_context,
                    plan_mode_state.as_mut(),
                    &mut assistant_message_stream_parsers,
                )
                .await;
                sess.record_token_usage_info(&turn_context, token_usage.as_ref())
                    .await;
                should_emit_token_count = true;
                should_emit_turn_diff = true;
                if let Some(false) = end_turn {
                    needs_follow_up = true;
                }
                completed_response_id = Some(response_id);
                break Ok(SamplingRequestResult {
                    needs_follow_up,
                    last_agent_message,
                });
            }
            ResponseEvent::OutputTextDelta(delta) => {
                // In review child threads, suppress assistant text deltas; the
                // UI will show a selection popup from the final ReviewOutput.
                if let Some(active) = active_item.as_ref() {
                    if !active_item_is_streaming_to_client {
                        continue;
                    }
                    let item_id = active.id();
                    if matches!(active, TurnItem::AgentMessage(_)) {
                        let parsed = assistant_message_stream_parsers.parse_delta(&item_id, &delta);
                        emit_streamed_assistant_text_delta(
                            &sess,
                            &turn_context,
                            plan_mode_state.as_mut(),
                            &item_id,
                            parsed,
                        )
                        .await;
                    } else if matches!(
                        active,
                        TurnItem::EventDrivenTool(_) | TurnItem::CollabAgentMessage(_)
                    ) {
                        continue;
                    } else {
                        let event = AgentMessageContentDeltaEvent {
                            thread_id: sess.conversation_id.to_string(),
                            turn_id: turn_context.sub_id.clone(),
                            item_id,
                            delta,
                        };
                        sess.send_event(&turn_context, EventMsg::AgentMessageContentDelta(event))
                            .await;
                    }
                } else {
                    error_or_panic("OutputTextDelta without active item".to_string());
                }
            }
            ResponseEvent::ToolCallInputDelta {
                item_id: _,
                call_id,
                delta,
            } => {
                let Some((active_call_id, consumer)) = active_tool_argument_diff_consumer.as_mut()
                else {
                    continue;
                };
                let call_id = match call_id {
                    Some(call_id) if call_id.as_str() != active_call_id.as_str() => continue,
                    Some(call_id) => call_id,
                    None => active_call_id.clone(),
                };
                if let Some(event) = consumer.consume_diff(turn_context.as_ref(), call_id, &delta) {
                    sess.send_event(&turn_context, event).await;
                }
            }
            ResponseEvent::ReasoningSummaryDelta {
                delta,
                summary_index,
            } => {
                if let Some(active) = active_item.as_ref() {
                    if !active_item_is_streaming_to_client {
                        continue;
                    }
                    let event = ReasoningContentDeltaEvent {
                        thread_id: sess.conversation_id.to_string(),
                        turn_id: turn_context.sub_id.clone(),
                        item_id: active.id(),
                        delta,
                        summary_index,
                    };
                    sess.send_event(&turn_context, EventMsg::ReasoningContentDelta(event))
                        .await;
                } else {
                    error_or_panic("ReasoningSummaryDelta without active item".to_string());
                }
            }
            ResponseEvent::ReasoningSummaryPartAdded { summary_index } => {
                if let Some(active) = active_item.as_ref() {
                    if !active_item_is_streaming_to_client {
                        continue;
                    }
                    let event =
                        EventMsg::AgentReasoningSectionBreak(AgentReasoningSectionBreakEvent {
                            item_id: active.id(),
                            summary_index,
                        });
                    sess.send_event(&turn_context, event).await;
                } else {
                    error_or_panic("ReasoningSummaryPartAdded without active item".to_string());
                }
            }
            ResponseEvent::ReasoningContentDelta {
                delta,
                content_index,
            } => {
                if let Some(active) = active_item.as_ref() {
                    if !active_item_is_streaming_to_client {
                        continue;
                    }
                    let event = ReasoningRawContentDeltaEvent {
                        thread_id: sess.conversation_id.to_string(),
                        turn_id: turn_context.sub_id.clone(),
                        item_id: active.id(),
                        delta,
                        content_index,
                    };
                    sess.send_event(&turn_context, EventMsg::ReasoningRawContentDelta(event))
                        .await;
                } else {
                    error_or_panic("ReasoningRawContentDelta without active item".to_string());
                }
            }
        }
    };

    flush_assistant_text_segments_all(
        &sess,
        &turn_context,
        plan_mode_state.as_mut(),
        &mut assistant_message_stream_parsers,
    )
    .await;

    if sess
        .features
        .enabled(Feature::ResponsesWebsocketResponseProcessed)
        && outcome.is_ok()
        && let Some(response_id) = completed_response_id.as_deref()
    {
        client_session.send_response_processed(response_id).await;
    }

    drain_in_flight(&mut in_flight, sess.clone(), turn_context.clone()).await?;

    if should_emit_token_count {
        // A tool call such as request_user_input can intentionally pause the turn. Emit token
        // counts only after pending tools resolve so clients do not see progress events while the
        // turn is waiting on the user. This also needs to happen before returning cancellation so
        // token usage already recorded from the completed response is still persisted.
        sess.send_token_count_event(&turn_context).await;
    }

    if cancellation_token.is_cancelled() {
        return Err(CodexErr::TurnAborted);
    }

    if should_emit_turn_diff {
        let unified_diff = {
            let tracker = turn_diff_tracker.lock().await;
            tracker.get_unified_diff()
        };
        if let Some(unified_diff) = unified_diff {
            let msg = EventMsg::TurnDiff(TurnDiffEvent { unified_diff });
            sess.clone().send_event(&turn_context, msg).await;
        }
    }

    outcome
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
