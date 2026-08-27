use protocol::items::ConversationArtifactItem;
use protocol::items::ConversationArtifactSource;
use protocol::items::TurnItem;
use serde::Deserialize;
use serde::Serialize;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use url::Url;
use uuid::Uuid;

use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;
use crate::planning::PUBLISH_ARTIFACT_TOOL_NAME;
use crate::planning::ToolSpec;
use crate::planning::create_publish_artifact_tool;

const DEFAULT_URL_ARTIFACT_MIME_TYPE: &str = "text/uri-list";

pub(crate) fn specs(_request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    vec![create_publish_artifact_tool()]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name == PUBLISH_ARTIFACT_TOOL_NAME
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) async fn dispatch(
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadRuntimeCapability,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let args: PublishArtifactArgs = parse_arguments(&call)?;
    let artifact = args.into_artifact()?;
    let output = PublishArtifactOutput {
        id: artifact.id.clone(),
        title: artifact.title.clone(),
        source_type: artifact_source_type(&artifact),
    };
    session
        .emit_turn_item_completed(turn, TurnItem::ConversationArtifact(artifact))
        .await;

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(FunctionToolOutput::from_text(
            serde_json::to_string(&output).map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize publish_artifact output: {err}"
                ))
            })?,
            Some(true),
        )),
        post_tool_use_payload: None,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishArtifactArgs {
    title: String,
    source: PublishArtifactSourceArgs,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum PublishArtifactSourceArgs {
    #[serde(rename_all = "camelCase")]
    Inline {
        content: String,
        mime_type: String,
        #[serde(default)]
        language: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Url {
        url: String,
        #[serde(default)]
        mime_type: Option<String>,
        #[serde(default)]
        fallback_content: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishArtifactOutput {
    id: String,
    title: String,
    source_type: &'static str,
}

impl PublishArtifactArgs {
    fn into_artifact(self) -> Result<ConversationArtifactItem, FunctionCallError> {
        let title = normalized_required_string(self.title, "title")?;
        let id = format!("artifact-{}", Uuid::new_v4());
        let artifact = match self.source {
            PublishArtifactSourceArgs::Inline {
                content,
                mime_type,
                language,
            } => {
                let mime_type = normalized_required_string(mime_type, "source.mimeType")?;
                let language = language.and_then(normalized_optional_string);
                ConversationArtifactItem {
                    id,
                    title,
                    source: Some(ConversationArtifactSource::Inline {
                        content: content.clone(),
                        mime_type: mime_type.clone(),
                        language: language.clone(),
                        truncated: false,
                    }),
                    mime_type,
                    content,
                    language,
                    truncated: false,
                }
            }
            PublishArtifactSourceArgs::Url {
                url,
                mime_type,
                fallback_content,
            } => {
                let url = validate_artifact_url(&url)?;
                let mime_type = mime_type.and_then(normalized_optional_string);
                let fallback_content = fallback_content.and_then(normalized_optional_string);
                ConversationArtifactItem {
                    id,
                    title,
                    source: Some(ConversationArtifactSource::Url {
                        url: url.clone(),
                        mime_type: mime_type.clone(),
                        fallback_content: fallback_content.clone(),
                    }),
                    mime_type: mime_type
                        .clone()
                        .unwrap_or_else(|| DEFAULT_URL_ARTIFACT_MIME_TYPE.to_string()),
                    content: fallback_content.unwrap_or_else(|| url.clone()),
                    language: None,
                    truncated: false,
                }
            }
        };
        Ok(artifact)
    }
}

fn artifact_source_type(artifact: &ConversationArtifactItem) -> &'static str {
    match artifact.resolved_source() {
        ConversationArtifactSource::Inline { .. } => "inline",
        ConversationArtifactSource::Url { .. } => "url",
    }
}

fn parse_arguments<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(call.function_arguments()?).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to parse {} arguments: {err}",
            call.tool_name
        ))
    })
}

fn normalized_required_string(value: String, field: &str) -> Result<String, FunctionCallError> {
    normalized_optional_string(value).ok_or_else(|| {
        FunctionCallError::RespondToModel(format!("{field} must be a non-empty string"))
    })
}

fn normalized_optional_string(value: String) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn validate_artifact_url(value: &str) -> Result<String, FunctionCallError> {
    let value = normalized_required_string(value.to_string(), "source.url")?;
    let parsed = Url::parse(&value).map_err(|err| {
        FunctionCallError::RespondToModel(format!("source.url must be a valid URL: {err}"))
    })?;
    if parsed.host_str().is_none() {
        return Err(FunctionCallError::RespondToModel(
            "source.url must include a host".to_string(),
        ));
    }
    match parsed.scheme() {
        "https" => {}
        "http" if is_local_http_url(&parsed) => {}
        "http" => {
            return Err(FunctionCallError::RespondToModel(
                "source.url may use http only for localhost, .localhost, or 127.0.0.1".to_string(),
            ));
        }
        _ => {
            return Err(FunctionCallError::RespondToModel(
                "source.url must use http or https".to_string(),
            ));
        }
    }
    Ok(parsed.to_string())
}

fn is_local_http_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host.eq_ignore_ascii_case("127.0.0.1")
        || host.to_ascii_lowercase().ends_with(".localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_and_local_http_urls() {
        assert_eq!(
            validate_artifact_url("https://example.com/app").unwrap(),
            "https://example.com/app"
        );
        assert_eq!(
            validate_artifact_url("http://localhost:5173/").unwrap(),
            "http://localhost:5173/"
        );
        assert_eq!(
            validate_artifact_url("http://preview.localhost/").unwrap(),
            "http://preview.localhost/"
        );
        assert_eq!(
            validate_artifact_url("http://127.0.0.1:3000/").unwrap(),
            "http://127.0.0.1:3000/"
        );
    }

    #[test]
    fn rejects_invalid_or_non_local_http_urls() {
        assert!(validate_artifact_url("not a url").is_err());
        assert!(validate_artifact_url("file:///tmp/a.html").is_err());
        assert!(validate_artifact_url("http://example.com/app").is_err());
        assert!(validate_artifact_url("https:///missing-host").is_err());
    }

    #[test]
    fn inline_args_build_inline_source_artifact() {
        let artifact = PublishArtifactArgs {
            title: "Inline demo".to_string(),
            source: PublishArtifactSourceArgs::Inline {
                content: "<main>Hi</main>".to_string(),
                mime_type: "text/html".to_string(),
                language: Some("html".to_string()),
            },
        }
        .into_artifact()
        .unwrap();

        assert_eq!(artifact.title, "Inline demo");
        assert_eq!(artifact.mime_type, "text/html");
        assert_eq!(artifact.content, "<main>Hi</main>");
        assert_eq!(
            artifact.source,
            Some(ConversationArtifactSource::Inline {
                content: "<main>Hi</main>".to_string(),
                mime_type: "text/html".to_string(),
                language: Some("html".to_string()),
                truncated: false,
            })
        );
    }

    #[test]
    fn url_args_build_url_source_artifact() {
        let artifact = PublishArtifactArgs {
            title: "Preview".to_string(),
            source: PublishArtifactSourceArgs::Url {
                url: "http://localhost:5173".to_string(),
                mime_type: Some("text/html".to_string()),
                fallback_content: Some("Open preview".to_string()),
            },
        }
        .into_artifact()
        .unwrap();

        assert_eq!(artifact.mime_type, "text/html");
        assert_eq!(artifact.content, "Open preview");
        assert_eq!(
            artifact.source,
            Some(ConversationArtifactSource::Url {
                url: "http://localhost:5173/".to_string(),
                mime_type: Some("text/html".to_string()),
                fallback_content: Some("Open preview".to_string()),
            })
        );
    }
}
