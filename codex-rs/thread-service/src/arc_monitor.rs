use std::env;
use std::time::Duration;

use codex_context_manager::is_contextual_user_message_content;
use model_service_api::ArcMonitorResult;
use model_service_api::ArcMonitorResultOutcome;
use model_service_api::ArcMonitorRuntimeRequest;
use model_service_api::auth_provider_from_auth_snapshot;
use model_service_api::build_arc_monitor_request as build_arc_monitor_request_from_history;
use tracing::warn;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

#[cfg(test)]
pub(crate) use model_service_api::ArcMonitorChatMessage;
#[cfg(test)]
pub(crate) use model_service_api::ArcMonitorMetadata;
#[cfg(test)]
pub(crate) use model_service_api::ArcMonitorPolicies;
#[cfg(test)]
pub(crate) use model_service_api::ArcMonitorRequest;

const ARC_MONITOR_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_ARC_MONITOR_ENDPOINT_OVERRIDE: &str = "CODEX_ARC_MONITOR_ENDPOINT_OVERRIDE";
const CODEX_ARC_MONITOR_TOKEN: &str = "CODEX_ARC_MONITOR_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArcMonitorOutcome {
    Ok,
    SteerModel(String),
    AskUser(String),
}

pub(crate) async fn monitor_action(
    sess: &Session,
    turn_context: &TurnContext,
    action: serde_json::Value,
    protection_client_callsite: &'static str,
) -> ArcMonitorOutcome {
    let auth = match turn_context.auth_runtime.as_ref() {
        Some(auth_runtime) => match auth_runtime.auth().await {
            Some(auth) if auth.uses_codex_backend() => Some(auth),
            _ => None,
        },
        None => None,
    };
    let env_token = read_non_empty_env_var(CODEX_ARC_MONITOR_TOKEN);
    if env_token.is_none() && auth.is_none() {
        return ArcMonitorOutcome::Ok;
    }

    let url = read_non_empty_env_var(CODEX_ARC_MONITOR_ENDPOINT_OVERRIDE).unwrap_or_else(|| {
        format!(
            "{}/codex/safety/arc",
            turn_context.config.chatgpt_base_url.trim_end_matches('/')
        )
    });
    let action = match action {
        serde_json::Value::Object(action) => action,
        _ => {
            warn!("skipping safety monitor because action payload is not an object");
            return ArcMonitorOutcome::Ok;
        }
    };
    let body =
        build_arc_monitor_request(sess, turn_context, action, protection_client_callsite).await;
    let body = match serde_json::to_value(&body) {
        Ok(body) => body,
        Err(err) => {
            warn!(error = %err, %url, "failed to serialize safety monitor request");
            return ArcMonitorOutcome::Ok;
        }
    };
    let auth_headers: http::HeaderMap = if env_token.is_none() {
        auth.as_ref()
            .map(auth_provider_from_auth_snapshot)
            .map(|auth_provider| auth_provider.to_auth_headers())
            .unwrap_or_default()
    } else {
        http::HeaderMap::new()
    };

    let response = match sess
        .services
        .api_runtime_factory
        .arc_monitor_client()
        .send(ArcMonitorRuntimeRequest {
            url: url.clone(),
            body,
            bearer_token: env_token,
            auth_headers,
            timeout: ARC_MONITOR_TIMEOUT,
        })
        .await
    {
        Ok(response) => response,
        Err(err) => {
            warn!(error = %err, %url, "safety monitor request failed");
            return ArcMonitorOutcome::Ok;
        }
    };
    let status = response.status;
    if !status.is_success() {
        warn!(
            %status,
            %url,
            response_text = response.body_text,
            "safety monitor returned non-success status"
        );
        return ArcMonitorOutcome::Ok;
    }

    let response = match serde_json::from_str::<ArcMonitorResult>(&response.body_text) {
        Ok(response) => response,
        Err(err) => {
            warn!(error = %err, %url, "failed to parse safety monitor response");
            return ArcMonitorOutcome::Ok;
        }
    };
    tracing::debug!(
        risk_score = response.risk_score,
        risk_level = ?response.risk_level,
        evidence_count = response.evidence.len(),
        "safety monitor completed"
    );

    let short_reason = response.short_reason.trim();
    let rationale = response.rationale.trim();
    match response.outcome {
        ArcMonitorResultOutcome::Ok => ArcMonitorOutcome::Ok,
        ArcMonitorResultOutcome::AskUser => {
            if !short_reason.is_empty() {
                ArcMonitorOutcome::AskUser(short_reason.to_string())
            } else if !rationale.is_empty() {
                ArcMonitorOutcome::AskUser(rationale.to_string())
            } else {
                ArcMonitorOutcome::AskUser(
                    "Additional confirmation is required before this tool call can continue."
                        .to_string(),
                )
            }
        }
        ArcMonitorResultOutcome::SteerModel => {
            if !rationale.is_empty() {
                ArcMonitorOutcome::SteerModel(rationale.to_string())
            } else if !short_reason.is_empty() {
                ArcMonitorOutcome::SteerModel(short_reason.to_string())
            } else {
                ArcMonitorOutcome::SteerModel(
                    "Tool call was cancelled because of safety risks.".to_string(),
                )
            }
        }
    }
}

fn read_non_empty_env_var(key: &str) -> Option<String> {
    match env::var(key) {
        Ok(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => {
            warn!(
                env_var = key,
                "ignoring non-unicode safety monitor env override"
            );
            None
        }
    }
}

async fn build_arc_monitor_request(
    sess: &Session,
    turn_context: &TurnContext,
    action: serde_json::Map<String, serde_json::Value>,
    protection_client_callsite: &'static str,
) -> model_service_api::ArcMonitorRequest {
    let history = sess.clone_history().await;
    let conversation_id = sess.conversation_id.to_string();
    build_arc_monitor_request_from_history(
        conversation_id,
        turn_context.sub_id.clone(),
        Some(protection_client_callsite.to_string()),
        history.raw_items(),
        action,
        is_contextual_user_message_content,
    )
}

#[cfg(test)]
#[path = "arc_monitor_tests.rs"]
mod tests;
