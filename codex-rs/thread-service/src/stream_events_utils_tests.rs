use super::HandleOutputCtx;
use super::TurnItemContributorPolicy;
use super::finalize_non_tool_response_item;
use super::handle_non_tool_response_item;
use super::handle_output_item_done;
use super::image_generation_artifact_path;
use super::save_image_generation_result;
use crate::TaskKind;
use crate::session::tests::make_session_and_context;
use crate::session::tests::make_session_and_context_with_rx;
use crate::session::tests::test_tool_inputs;
use crate::tasks::SessionTask;
use crate::tasks::SessionTaskContext;
use codex_extension_api::ExtensionData;
use codex_extension_api::TurnItemContributionFuture;
use codex_extension_api::TurnItemContributor;
use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;
use protocol::AgentPath;
use protocol::config_types::CollaborationMode;
use protocol::config_types::ModeKind;
use protocol::config_types::Settings;
use protocol::error::CodexErr;
use protocol::items::AgentMessageContent;
use protocol::items::TurnItem;
use protocol::memory_citation::MemoryCitation;
use protocol::models::ContentItem;
use protocol::models::MessagePhase;
use protocol::models::ResponseItem;
use protocol::protocol::EventMsg;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use protocol::user_input::UserInput;
use std::sync::Arc;
use thread_service_api::TurnDiffTracker;
use tokio_util::sync::CancellationToken;

fn assistant_output_text(text: &str) -> ResponseItem {
    assistant_output_text_with_phase(text, /*phase*/ None)
}

fn assistant_output_text_with_phase(text: &str, phase: Option<MessagePhase>) -> ResponseItem {
    ResponseItem::Message {
        id: Some("msg-1".to_string()),
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase,
    }
}

fn agent_message_visible_text(item: &protocol::items::AgentMessageItem) -> String {
    item.content
        .iter()
        .map(|entry| match entry {
            AgentMessageContent::Text { text } => text.as_str(),
        })
        .collect()
}

#[tokio::test]
async fn finalize_non_tool_response_item_strips_citations_from_assistant_message() {
    let (session, turn_context) = make_session_and_context().await;
    let item = assistant_output_text(
        "hello<oai-mem-citation><citation_entries>\nMEMORY.md:1-2|note=[x]\n</citation_entries>\n<rollout_ids>\n019cc2ea-1dff-7902-8d40-c8f6e5d83cc4\n</rollout_ids></oai-mem-citation> world",
    );

    let finalized = finalize_non_tool_response_item(
        &session,
        &turn_context,
        TurnItemContributorPolicy::Skip,
        &item,
        /*plan_mode*/ false,
    )
    .await
    .expect("assistant message should parse");

    let TurnItem::AgentMessage(agent_message) = finalized.turn_item else {
        panic!("expected agent message");
    };
    let text = agent_message
        .content
        .iter()
        .map(|entry| match entry {
            protocol::items::AgentMessageContent::Text { text } => text.as_str(),
        })
        .collect::<String>();
    assert_eq!(text, "hello world");
    let memory_citation = agent_message
        .memory_citation
        .expect("memory citation should be parsed");
    assert_eq!(memory_citation.entries.len(), 1);
    assert_eq!(memory_citation.entries[0].path, "MEMORY.md");
    assert_eq!(
        memory_citation.rollout_ids,
        vec!["019cc2ea-1dff-7902-8d40-c8f6e5d83cc4".to_string()]
    );
}

struct TestTurnItemContributor;

#[derive(Debug)]
struct TurnItemContributorRan;

impl TurnItemContributor for TestTurnItemContributor {
    fn contribute<'a>(
        &'a self,
        _thread_store: &'a ExtensionData,
        turn_store: &'a ExtensionData,
        item: &'a mut TurnItem,
    ) -> TurnItemContributionFuture<'a> {
        Box::pin(async move {
            turn_store.insert(TurnItemContributorRan);
            if let TurnItem::AgentMessage(agent_message) = item {
                agent_message.memory_citation = Some(MemoryCitation {
                    entries: Vec::new(),
                    rollout_ids: Vec::new(),
                });
            }
            Ok(())
        })
    }
}

struct RewriteAgentMessageContributor;

impl TurnItemContributor for RewriteAgentMessageContributor {
    fn contribute<'a>(
        &'a self,
        _thread_store: &'a ExtensionData,
        _turn_store: &'a ExtensionData,
        item: &'a mut TurnItem,
    ) -> TurnItemContributionFuture<'a> {
        Box::pin(async move {
            if let TurnItem::AgentMessage(agent_message) = item {
                agent_message.content = vec![AgentMessageContent::Text {
                    text: "contributed assistant text".to_string(),
                }];
            }
            Ok(())
        })
    }
}

struct NeverEndingTask;

impl SessionTask for NeverEndingTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }

    fn span_name(&self) -> &'static str {
        "session_task.never_ending"
    }

    async fn run(
        self: Arc<Self>,
        _session: Arc<SessionTaskContext>,
        _ctx: Arc<crate::session::turn_context::TurnContext>,
        _input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        cancellation_token.cancelled().await;
        None
    }
}

#[tokio::test]
async fn handle_non_tool_response_item_runs_turn_item_contributors_only_when_requested() {
    let (mut session, turn_context) = make_session_and_context().await;
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.turn_item_contributor(Arc::new(TestTurnItemContributor));
    session.services.extensions = Arc::new(builder.build());
    let turn_store = ExtensionData::new(turn_context.sub_id.clone());
    let item = assistant_output_text(
        "hello<oai-mem-citation>ignored by memory parser</oai-mem-citation> world",
    );

    let provisional_turn_item = handle_non_tool_response_item(
        &session,
        &turn_context,
        TurnItemContributorPolicy::Skip,
        &item,
        /*plan_mode*/ false,
    )
    .await
    .expect("assistant message should parse");

    assert!(turn_store.get::<TurnItemContributorRan>().is_none());
    let TurnItem::AgentMessage(provisional_agent_message) = provisional_turn_item else {
        panic!("expected agent message");
    };
    assert_eq!(provisional_agent_message.memory_citation, None);

    let finalized = finalize_non_tool_response_item(
        &session,
        &turn_context,
        TurnItemContributorPolicy::Run(&turn_store),
        &item,
        /*plan_mode*/ false,
    )
    .await
    .expect("assistant message should parse");

    assert!(turn_store.get::<TurnItemContributorRan>().is_some());
    let TurnItem::AgentMessage(agent_message) = finalized.turn_item else {
        panic!("expected agent message");
    };
    assert!(agent_message.memory_citation.is_some());
    let text = agent_message
        .content
        .iter()
        .map(|entry| match entry {
            protocol::items::AgentMessageContent::Text { text } => text.as_str(),
        })
        .collect::<String>();
    assert_eq!(text, "hello world");
}

#[tokio::test]
async fn handle_output_item_done_returns_contributed_last_agent_message() {
    let (mut session, turn_context) = make_session_and_context().await;
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.turn_item_contributor(Arc::new(RewriteAgentMessageContributor));
    session.services.extensions = Arc::new(builder.build());
    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    let item = assistant_output_text("original assistant text");
    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&session),
        turn_context: Arc::clone(&turn_context),
        turn_store: Arc::new(ExtensionData::new(turn_context.sub_id.clone())),
        tool_inputs: test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context)),
        turn_diff_tracker: tracker,
        cancellation_token: CancellationToken::new(),
    };

    let output = handle_output_item_done(&mut ctx, item, /*previously_active_item*/ None)
        .await
        .expect("assistant message should complete");

    assert_eq!(
        output.last_agent_message.as_deref(),
        Some("contributed assistant text")
    );
}

#[tokio::test]
async fn handle_output_item_done_emits_artifact_items_from_assistant_marker() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    let tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    let text = concat!(
        "before ",
        "<<<MORPHEUS_ARTIFACT {\"title\":\"Demo\",\"mime_type\":\"text/html\",\"language\":\"html\"}>>>",
        "<p>Hello</p>",
        "<<<END_MORPHEUS_ARTIFACT>>>",
        " after"
    );
    let item = assistant_output_text(text);
    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&session),
        turn_context: Arc::clone(&turn_context),
        turn_store: Arc::new(ExtensionData::new(turn_context.sub_id.clone())),
        tool_inputs: test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context)),
        turn_diff_tracker: tracker,
        cancellation_token: CancellationToken::new(),
    };

    let output = handle_output_item_done(&mut ctx, item, /*previously_active_item*/ None)
        .await
        .expect("assistant message should complete");

    assert_eq!(output.last_agent_message.as_deref(), Some("before  after"));
    let mut completed_items = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while completed_items.len() < 3 {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("expected item completed")
            .expect("event channel");
        if let EventMsg::ItemCompleted(completed) = event.msg {
            completed_items.push(completed.item);
        }
    }

    let TurnItem::AgentMessage(before) = &completed_items[0] else {
        panic!("expected leading message");
    };
    assert_eq!(agent_message_visible_text(before), "before ");
    let TurnItem::ConversationArtifact(artifact) = &completed_items[1] else {
        panic!("expected artifact item");
    };
    assert_eq!(artifact.title, "Demo");
    assert_eq!(artifact.mime_type, "text/html");
    assert_eq!(artifact.language.as_deref(), Some("html"));
    assert_eq!(artifact.content, "<p>Hello</p>");
    let TurnItem::AgentMessage(after) = &completed_items[2] else {
        panic!("expected trailing message");
    };
    assert_eq!(agent_message_visible_text(after), " after");
}

#[tokio::test]
async fn plan_mode_handle_output_item_done_emits_artifact_items_from_assistant_marker() {
    let (session, mut turn_context, rx) = make_session_and_context_with_rx().await;
    Arc::get_mut(&mut turn_context)
        .expect("turn context should be uniquely owned")
        .collaboration_mode = CollaborationMode {
        mode: ModeKind::Plan,
        settings: Settings {
            model: "test-model".to_string(),
            reasoning_effort: None,
            developer_instructions: None,
        },
    };
    let tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    let text = concat!(
        "before ",
        "<<<MORPHEUS_ARTIFACT {\"title\":\"Plan Demo\",\"mime_type\":\"application/json\",\"language\":\"json\"}>>>",
        "{\"ok\":true}",
        "<<<END_MORPHEUS_ARTIFACT>>>",
        " after"
    );
    let item = assistant_output_text(text);
    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&session),
        turn_context: Arc::clone(&turn_context),
        turn_store: Arc::new(ExtensionData::new(turn_context.sub_id.clone())),
        tool_inputs: test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context)),
        turn_diff_tracker: tracker,
        cancellation_token: CancellationToken::new(),
    };

    let output = handle_output_item_done(&mut ctx, item, /*previously_active_item*/ None)
        .await
        .expect("assistant message should complete");

    assert_eq!(output.last_agent_message.as_deref(), Some("before  after"));
    let mut completed_items = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while completed_items.len() < 3 {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("expected item completed")
            .expect("event channel");
        if let EventMsg::ItemCompleted(completed) = event.msg {
            completed_items.push(completed.item);
        }
    }

    assert!(matches!(completed_items[0], TurnItem::AgentMessage(_)));
    let TurnItem::ConversationArtifact(artifact) = &completed_items[1] else {
        panic!("expected artifact item");
    };
    assert_eq!(artifact.title, "Plan Demo");
    assert_eq!(artifact.mime_type, "application/json");
    assert_eq!(artifact.content, "{\"ok\":true}");
    assert!(matches!(completed_items[2], TurnItem::AgentMessage(_)));
}

#[tokio::test]
async fn handle_output_item_done_defers_mailbox_for_artifact_only_assistant_marker() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    session
        .spawn_task(Arc::clone(&turn_context), Vec::new(), NeverEndingTask)
        .await;
    let tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    let text = concat!(
        "<<<MORPHEUS_ARTIFACT {\"title\":\"Only\",\"mime_type\":\"text/html\",\"language\":\"html\"}>>>",
        "<p>Hello</p>",
        "<<<END_MORPHEUS_ARTIFACT>>>"
    );
    let item = assistant_output_text(text);
    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&session),
        turn_context: Arc::clone(&turn_context),
        turn_store: Arc::new(ExtensionData::new(turn_context.sub_id.clone())),
        tool_inputs: test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context)),
        turn_diff_tracker: tracker,
        cancellation_token: CancellationToken::new(),
    };

    let output = handle_output_item_done(&mut ctx, item, /*previously_active_item*/ None)
        .await
        .expect("assistant message should complete");

    assert_eq!(output.last_agent_message, None);
    let completed = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("expected item completed")
            .expect("event channel");
        if let EventMsg::ItemCompleted(completed) = event.msg {
            break completed;
        }
    };
    assert!(matches!(completed.item, TurnItem::ConversationArtifact(_)));

    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "late update".to_string(),
        InterAgentOperation::Unknown,
    )
    .with_trigger_turn(true);
    assert!(
        !session.enqueue_mailbox_communication(communication).await,
        "artifact-only final answer should keep later mailbox input out of the active turn"
    );
    assert!(
        session.has_pending_mailbox_items().await,
        "later mailbox input should remain buffered for the next turn"
    );

    session
        .abort_all_tasks(protocol::protocol::TurnAbortReason::Replaced)
        .await;
}

#[tokio::test]
async fn finalized_turn_item_defers_mailbox_for_contributed_visible_text() {
    let (mut session, turn_context) = make_session_and_context().await;
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.turn_item_contributor(Arc::new(RewriteAgentMessageContributor));
    session.services.extensions = Arc::new(builder.build());
    let turn_store = ExtensionData::new(turn_context.sub_id.clone());
    let item = assistant_output_text("<oai-mem-citation>hidden only</oai-mem-citation>");

    let finalized = finalize_non_tool_response_item(
        &session,
        &turn_context,
        TurnItemContributorPolicy::Run(&turn_store),
        &item,
        /*plan_mode*/ false,
    )
    .await
    .expect("assistant message should parse");

    assert_eq!(
        finalized.facts.last_agent_message.as_deref(),
        Some("contributed assistant text")
    );
    assert!(finalized.facts.defers_mailbox_delivery_to_next_turn);
}

#[tokio::test]
async fn finalized_turn_item_keeps_mailbox_open_for_commentary_text() {
    let (mut session, turn_context) = make_session_and_context().await;
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.turn_item_contributor(Arc::new(RewriteAgentMessageContributor));
    session.services.extensions = Arc::new(builder.build());
    let turn_store = ExtensionData::new(turn_context.sub_id.clone());
    let item = assistant_output_text_with_phase("still working", Some(MessagePhase::Commentary));

    let finalized = finalize_non_tool_response_item(
        &session,
        &turn_context,
        TurnItemContributorPolicy::Run(&turn_store),
        &item,
        /*plan_mode*/ false,
    )
    .await
    .expect("assistant message should parse");

    assert_eq!(
        finalized.facts.last_agent_message.as_deref(),
        Some("contributed assistant text")
    );
    assert!(!finalized.facts.defers_mailbox_delivery_to_next_turn);
}

#[tokio::test]
async fn save_image_generation_result_saves_base64_to_png_in_codex_home() {
    let codex_home = tempfile::tempdir().expect("create codex home");
    let codex_home = codex_home.path().abs();
    let expected_path = image_generation_artifact_path(&codex_home, "session-1", "ig_save_base64");
    let _ = std::fs::remove_file(&expected_path);

    let saved_path =
        save_image_generation_result(&codex_home, "session-1", "ig_save_base64", "Zm9v")
            .await
            .expect("image should be saved");

    assert_eq!(saved_path, expected_path);
    assert_eq!(std::fs::read(&saved_path).expect("saved file"), b"foo");
    let _ = std::fs::remove_file(&saved_path);
}

#[tokio::test]
async fn save_image_generation_result_rejects_data_url_payload() {
    let result = "data:image/jpeg;base64,Zm9v";
    let codex_home = tempfile::tempdir().expect("create codex home");
    let codex_home = codex_home.path().abs();

    let err = save_image_generation_result(&codex_home, "session-1", "ig_456", result)
        .await
        .expect_err("data url payload should error");
    assert!(matches!(err, CodexErr::InvalidRequest(_)));
}

#[tokio::test]
async fn save_image_generation_result_overwrites_existing_file() {
    let codex_home = tempfile::tempdir().expect("create codex home");
    let codex_home = codex_home.path().abs();
    let existing_path = image_generation_artifact_path(&codex_home, "session-1", "ig_overwrite");
    std::fs::create_dir_all(
        existing_path
            .parent()
            .expect("generated image path should have a parent"),
    )
    .expect("create image output dir");
    std::fs::write(&existing_path, b"existing").expect("seed existing image");

    let saved_path = save_image_generation_result(&codex_home, "session-1", "ig_overwrite", "Zm9v")
        .await
        .expect("image should be saved");

    assert_eq!(saved_path, existing_path);
    assert_eq!(std::fs::read(&saved_path).expect("saved file"), b"foo");
    let _ = std::fs::remove_file(&saved_path);
}

#[tokio::test]
async fn save_image_generation_result_sanitizes_call_id_for_codex_home_output_path() {
    let codex_home = tempfile::tempdir().expect("create codex home");
    let codex_home = codex_home.path().abs();
    let expected_path = image_generation_artifact_path(&codex_home, "session-1", "../ig/..");
    let _ = std::fs::remove_file(&expected_path);

    let saved_path = save_image_generation_result(&codex_home, "session-1", "../ig/..", "Zm9v")
        .await
        .expect("image should be saved");

    assert_eq!(saved_path, expected_path);
    assert_eq!(std::fs::read(&saved_path).expect("saved file"), b"foo");
    let _ = std::fs::remove_file(&saved_path);
}

#[tokio::test]
async fn save_image_generation_result_rejects_non_standard_base64() {
    let codex_home = tempfile::tempdir().expect("create codex home");
    let codex_home = codex_home.path().abs();
    let err = save_image_generation_result(&codex_home, "session-1", "ig_urlsafe", "_-8")
        .await
        .expect_err("non-standard base64 should error");
    assert!(matches!(err, CodexErr::InvalidRequest(_)));
}

#[tokio::test]
async fn save_image_generation_result_rejects_non_base64_data_urls() {
    let codex_home = tempfile::tempdir().expect("create codex home");
    let codex_home = codex_home.path().abs();
    let err = save_image_generation_result(
        &codex_home,
        "session-1",
        "ig_svg",
        "data:image/svg+xml,<svg/>",
    )
    .await
    .expect_err("non-base64 data url should error");
    assert!(matches!(err, CodexErr::InvalidRequest(_)));
}
