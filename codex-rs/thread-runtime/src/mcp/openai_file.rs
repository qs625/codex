use serde_json::Value as JsonValue;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

pub(crate) async fn rewrite_mcp_tool_arguments_for_openai_files(
    sess: &Session,
    turn_context: &TurnContext,
    arguments_value: Option<JsonValue>,
    openai_file_input_params: Option<&[String]>,
) -> Result<Option<JsonValue>, String> {
    sess.rewrite_mcp_tool_arguments_for_openai_files(
        turn_context,
        arguments_value,
        openai_file_input_params,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use codex_login::CodexAuth;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;
    use crate::session::tests::make_session_and_context;

    #[tokio::test]
    async fn core_wrapper_uses_turn_context_path_resolution() {
        use wiremock::Mock;
        use wiremock::MockServer;
        use wiremock::ResponseTemplate;
        use wiremock::matchers::body_json;
        use wiremock::matchers::header;
        use wiremock::matchers::method;
        use wiremock::matchers::path;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/backend-api/files"))
            .and(header("chatgpt-account-id", "account_id"))
            .and(body_json(serde_json::json!({
                "file_name": "file_report.csv",
                "file_size": 5,
                "use_case": "codex",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "file_id": "file_123",
                "upload_url": format!("{}/upload/file_123", server.uri()),
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/upload/file_123"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/backend-api/files/file_123/uploaded"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "success",
                "download_url": format!("{}/download/file_123", server.uri()),
                "file_name": "file_report.csv",
                "mime_type": "text/csv",
                "file_size_bytes": 5,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (mut session, mut turn_context) = make_session_and_context().await;
        session.services.auth_runtime = crate::test_support::auth_manager_from_auth(
            CodexAuth::create_dummy_chatgpt_auth_for_testing(),
        );
        session.services.openai_file_uploader =
            Arc::new(codex_openai_files::ReqwestOpenAiFileUploader);

        let dir = tempdir().expect("temp dir");
        tokio::fs::write(dir.path().join("file_report.csv"), b"hello")
            .await
            .expect("write local file");
        #[allow(deprecated)]
        {
            turn_context.cwd = AbsolutePathBuf::try_from(dir.path()).expect("absolute path");
        }

        let mut config = (*turn_context.config).clone();
        config.chatgpt_base_url = format!("{}/backend-api", server.uri());
        turn_context.config = Arc::new(config);

        let rewritten = rewrite_mcp_tool_arguments_for_openai_files(
            &session,
            &turn_context,
            Some(serde_json::json!({
                "file": "file_report.csv",
            })),
            Some(&["file".to_string()]),
        )
        .await
        .expect("rewrite should succeed");

        assert_eq!(
            rewritten,
            Some(serde_json::json!({
                "file": {
                    "download_url": format!("{}/download/file_123", server.uri()),
                    "file_id": "file_123",
                    "mime_type": "text/csv",
                    "file_name": "file_report.csv",
                    "uri": "sediment://file_123",
                    "file_size_bytes": 5,
                },
            }))
        );
    }

    #[tokio::test]
    async fn core_wrapper_surfaces_upload_failures() {
        let (mut session, turn_context) = make_session_and_context().await;
        session.services.auth_runtime = crate::test_support::auth_manager_from_auth(
            CodexAuth::create_dummy_chatgpt_auth_for_testing(),
        );
        session.services.openai_file_uploader =
            Arc::new(codex_openai_files::ReqwestOpenAiFileUploader);
        let error = rewrite_mcp_tool_arguments_for_openai_files(
            &session,
            &turn_context,
            Some(serde_json::json!({
                "file": "/definitely/missing/file.csv",
            })),
            Some(&["file".to_string()]),
        )
        .await
        .expect_err("missing file should fail");

        assert!(error.contains("failed to upload"));
        assert!(error.contains("file"));
    }
}
