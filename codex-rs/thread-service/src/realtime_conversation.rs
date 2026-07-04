use std::sync::Arc;
use std::sync::atomic::Ordering;

use codex_config_types::RealtimeWsMode;
use http::HeaderMap;
use model_service_api::Provider as ApiProvider;
use model_service_api::RealtimeEventParser;
use model_service_api::RealtimeSessionConfig;
use model_service_api::RealtimeSessionMode;
use protocol::error::CodexErr;
use protocol::error::Result as CodexResult;
use protocol::protocol::CodexErrorInfo;
use protocol::protocol::ConversationAudioParams;
use protocol::protocol::ConversationStartParams;
use protocol::protocol::ConversationStartTransport;
use protocol::protocol::ConversationTextParams;
use protocol::protocol::ErrorEvent;
use protocol::protocol::Event;
use protocol::protocol::EventMsg;
use protocol::protocol::RealtimeConversationClosedEvent;
use protocol::protocol::RealtimeConversationRealtimeEvent;
use protocol::protocol::RealtimeConversationSdpEvent;
use protocol::protocol::RealtimeConversationStartedEvent;
use protocol::protocol::RealtimeConversationVersion as RealtimeWsVersion;
use protocol::protocol::RealtimeEvent;
use protocol::protocol::RealtimeOutputModality;
use protocol::protocol::RealtimeVoice;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

use crate::realtime_context::build_realtime_startup_context;
use crate::session::session::Session;

use codex_realtime::DEFAULT_REALTIME_MODEL;
use codex_realtime::REALTIME_STARTUP_CONTEXT_TOKEN_BUDGET;
pub(crate) use codex_realtime::REALTIME_USER_TEXT_PREFIX;
pub(crate) use codex_realtime::RealtimeConversationManager;
use codex_realtime::RealtimeStart;
use codex_realtime::RealtimeStartOutput;
use codex_realtime::default_realtime_voice;
pub(crate) use codex_realtime::prefix_realtime_v2_text;
use codex_realtime::prepare_realtime_backend_prompt;
use codex_realtime::realtime_delegation_from_handoff;
use codex_realtime::validate_realtime_voice;
use model_service_api::PrepareRealtimeTransportRequest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RealtimeConversationEnd {
    Requested,
    TransportClosed,
    Error,
}

pub(crate) async fn handle_start(
    sess: &Arc<Session>,
    sub_id: String,
    params: ConversationStartParams,
) -> CodexResult<()> {
    let prepared_start = match prepare_realtime_start(sess, params).await {
        Ok(prepared_start) => prepared_start,
        Err(err) => {
            error!("failed to prepare realtime conversation: {err}");
            let message = err.to_string();
            sess.send_event_raw(Event {
                id: sub_id,
                msg: EventMsg::RealtimeConversationRealtime(RealtimeConversationRealtimeEvent {
                    payload: RealtimeEvent::Error(message),
                }),
            })
            .await;
            return Ok(());
        }
    };

    if let Err(err) = handle_start_inner(sess, &sub_id, prepared_start).await {
        error!("failed to start realtime conversation: {err}");
        let message = err.to_string();
        sess.send_event_raw(Event {
            id: sub_id.clone(),
            msg: EventMsg::RealtimeConversationRealtime(RealtimeConversationRealtimeEvent {
                payload: RealtimeEvent::Error(message),
            }),
        })
        .await;
    }
    Ok(())
}

struct PreparedRealtimeConversationStart {
    api_provider: ApiProvider,
    extra_headers: Option<HeaderMap>,
    requested_realtime_session_id: Option<String>,
    version: RealtimeWsVersion,
    session_config: RealtimeSessionConfig,
    transport: ConversationStartTransport,
}

async fn prepare_realtime_start(
    sess: &Arc<Session>,
    params: ConversationStartParams,
) -> CodexResult<PreparedRealtimeConversationStart> {
    let config = sess.get_config().await;
    let transport = params
        .transport
        .unwrap_or(ConversationStartTransport::Websocket);
    let version = config.realtime.version;
    let session_config = build_realtime_session_config(
        sess,
        params.prompt,
        params.realtime_session_id,
        params.output_modality,
        params.voice,
    )
    .await?;
    let requested_realtime_session_id = session_config.session_id.clone();
    let prepared_transport = sess
        .services
        .model_client_api
        .prepare_realtime_transport(PrepareRealtimeTransportRequest {
            requested_realtime_session_id: requested_realtime_session_id.clone(),
            websocket_base_url: config.experimental_realtime_ws_base_url.clone(),
            include_api_key_header: matches!(transport, ConversationStartTransport::Websocket),
        })
        .await
        .map_err(|err| CodexErr::InvalidRequest(err.to_string()))?;
    Ok(PreparedRealtimeConversationStart {
        api_provider: prepared_transport.api_provider,
        extra_headers: prepared_transport.extra_headers,
        requested_realtime_session_id,
        version,
        session_config,
        transport,
    })
}

pub(crate) async fn build_realtime_session_config(
    sess: &Arc<Session>,
    prompt: Option<Option<String>>,
    realtime_session_id: Option<String>,
    output_modality: RealtimeOutputModality,
    voice: Option<RealtimeVoice>,
) -> CodexResult<RealtimeSessionConfig> {
    let config = sess.get_config().await;
    let prompt = prepare_realtime_backend_prompt(
        prompt,
        config.experimental_realtime_ws_backend_prompt.clone(),
    );
    let startup_context = match config.experimental_realtime_ws_startup_context.clone() {
        Some(startup_context) => startup_context,
        None => {
            build_realtime_startup_context(sess.as_ref(), REALTIME_STARTUP_CONTEXT_TOKEN_BUDGET)
                .await
                .unwrap_or_default()
        }
    };
    let prompt = match (prompt.is_empty(), startup_context.is_empty()) {
        (true, true) => String::new(),
        (true, false) => startup_context,
        (false, true) => prompt,
        (false, false) => format!("{prompt}\n\n{startup_context}"),
    };
    let model = Some(
        config
            .experimental_realtime_ws_model
            .clone()
            .unwrap_or_else(|| DEFAULT_REALTIME_MODEL.to_string()),
    );
    let event_parser = match config.realtime.version {
        RealtimeWsVersion::V1 => RealtimeEventParser::V1,
        RealtimeWsVersion::V2 => RealtimeEventParser::RealtimeV2,
    };
    if config.realtime.version == RealtimeWsVersion::V1
        && matches!(output_modality, RealtimeOutputModality::Text)
    {
        return Err(CodexErr::InvalidRequest(
            "text realtime output modality requires realtime v2".to_string(),
        ));
    }
    let session_mode = match config.realtime.session_type {
        RealtimeWsMode::Conversational => RealtimeSessionMode::Conversational,
        RealtimeWsMode::Transcription => RealtimeSessionMode::Transcription,
    };
    let voice = voice
        .or(config.realtime.voice)
        .unwrap_or_else(|| default_realtime_voice(config.realtime.version));
    validate_realtime_voice(config.realtime.version, voice)?;
    Ok(RealtimeSessionConfig {
        instructions: prompt,
        model,
        session_id: Some(realtime_session_id.unwrap_or_else(|| sess.conversation_id.to_string())),
        event_parser,
        session_mode,
        output_modality,
        voice,
    })
}

async fn handle_start_inner(
    sess: &Arc<Session>,
    sub_id: &str,
    prepared_start: PreparedRealtimeConversationStart,
) -> CodexResult<()> {
    let PreparedRealtimeConversationStart {
        api_provider,
        extra_headers,
        requested_realtime_session_id,
        version,
        session_config,
        transport,
    } = prepared_start;
    info!("starting realtime conversation");
    let sdp = match transport {
        ConversationStartTransport::Websocket => None,
        ConversationStartTransport::Webrtc { sdp } => Some(sdp),
    };
    let start = RealtimeStart {
        api_provider,
        extra_headers,
        session_config,
        model_client: Arc::clone(&sess.services.model_client_api),
        sdp,
    };
    let start_output = sess.conversation.start(start).await?;

    info!("realtime conversation started");

    sess.send_event_raw(Event {
        id: sub_id.to_string(),
        msg: EventMsg::RealtimeConversationStarted(RealtimeConversationStartedEvent {
            realtime_session_id: requested_realtime_session_id,
            version,
        }),
    })
    .await;

    let RealtimeStartOutput {
        realtime_active,
        events_rx,
        sdp,
    } = start_output;
    if let Some(sdp) = sdp {
        sess.send_event_raw(Event {
            id: sub_id.to_string(),
            msg: EventMsg::RealtimeConversationSdp(RealtimeConversationSdpEvent { sdp }),
        })
        .await;
    }

    let sess_clone = Arc::clone(sess);
    let sub_id = sub_id.to_string();
    let fanout_realtime_active = Arc::clone(&realtime_active);
    let fanout_task = tokio::spawn(async move {
        let ev = |msg| Event {
            id: sub_id.clone(),
            msg,
        };
        let mut end = RealtimeConversationEnd::TransportClosed;
        while let Ok(event) = events_rx.recv().await {
            if !fanout_realtime_active.load(Ordering::Relaxed) {
                break;
            }
            match &event {
                RealtimeEvent::AudioOut(_) => {}
                _ => {
                    info!(
                        event = ?event,
                        "received realtime conversation event"
                    );
                }
            }
            if let RealtimeEvent::Error(_) = &event {
                end = RealtimeConversationEnd::Error;
            }
            if let RealtimeEvent::HandoffRequested(handoff) = &event
                && let Some(text) = realtime_delegation_from_handoff(handoff)
            {
                debug!(text = %text, "[realtime-text] realtime conversation text output");
                let sess_for_routed_text = Arc::clone(&sess_clone);
                sess_for_routed_text.route_realtime_text_input(text).await;
            }
            if !fanout_realtime_active.load(Ordering::Relaxed) {
                break;
            }
            sess_clone
                .send_event_raw(ev(EventMsg::RealtimeConversationRealtime(
                    RealtimeConversationRealtimeEvent {
                        payload: event.clone(),
                    },
                )))
                .await;
        }
        if fanout_realtime_active.swap(false, Ordering::Relaxed) {
            match end {
                RealtimeConversationEnd::TransportClosed => {
                    info!("realtime conversation transport closed");
                }
                RealtimeConversationEnd::Requested | RealtimeConversationEnd::Error => {}
            }
            sess_clone
                .conversation
                .finish_if_active(&fanout_realtime_active)
                .await;
            send_realtime_conversation_closed(&sess_clone, sub_id, end).await;
        }
    });
    sess.conversation
        .register_fanout_task(&realtime_active, fanout_task)
        .await;

    Ok(())
}

pub(crate) async fn handle_audio(
    sess: &Arc<Session>,
    sub_id: String,
    params: ConversationAudioParams,
) {
    if let Err(err) = sess.conversation.audio_in(params.frame).await {
        error!("failed to append realtime audio: {err}");
        if sess.conversation.running_state().await.is_some() {
            warn!("realtime audio input failed while the session was already ending");
        } else {
            send_conversation_error(sess, sub_id, err.to_string(), CodexErrorInfo::BadRequest)
                .await;
        }
    }
}

pub(crate) async fn handle_text(
    sess: &Arc<Session>,
    sub_id: String,
    params: ConversationTextParams,
) {
    debug!(text = %params.text, "[realtime-text] appending realtime conversation text input");
    if let Err(err) = sess.conversation.text_in(params.text).await {
        error!("failed to append realtime text: {err}");
        if sess.conversation.running_state().await.is_some() {
            warn!("realtime text input failed while the session was already ending");
        } else {
            send_conversation_error(sess, sub_id, err.to_string(), CodexErrorInfo::BadRequest)
                .await;
        }
    }
}

pub(crate) async fn handle_close(sess: &Arc<Session>, sub_id: String) {
    end_realtime_conversation(sess, sub_id, RealtimeConversationEnd::Requested).await;
}

async fn send_conversation_error(
    sess: &Arc<Session>,
    sub_id: String,
    message: String,
    codex_error_info: CodexErrorInfo,
) {
    sess.send_event_raw(Event {
        id: sub_id,
        msg: EventMsg::Error(ErrorEvent {
            message,
            codex_error_info: Some(codex_error_info),
        }),
    })
    .await;
}

async fn end_realtime_conversation(
    sess: &Arc<Session>,
    sub_id: String,
    end: RealtimeConversationEnd,
) {
    let _ = sess.conversation.shutdown().await;
    send_realtime_conversation_closed(sess, sub_id, end).await;
}

async fn send_realtime_conversation_closed(
    sess: &Arc<Session>,
    sub_id: String,
    end: RealtimeConversationEnd,
) {
    let reason = match end {
        RealtimeConversationEnd::Requested => Some("requested".to_string()),
        RealtimeConversationEnd::TransportClosed => Some("transport_closed".to_string()),
        RealtimeConversationEnd::Error => Some("error".to_string()),
    };

    sess.send_event_raw(Event {
        id: sub_id,
        msg: EventMsg::RealtimeConversationClosed(RealtimeConversationClosedEvent { reason }),
    })
    .await;
}
