//! Bridges Apps SDK-style `openai/fileParams` metadata into Codex's MCP flow.
//!
//! Strategy:
//! - Inspect `_meta["openai/fileParams"]` to discover which tool arguments are
//!   file inputs.
//! - At tool execution time, upload those local files to OpenAI file storage
//!   and rewrite only the declared arguments into the provided-file payload
//!   shape expected by the downstream Apps tool.
//!
//! Model-visible schema masking is owned by `codex-mcp` alongside MCP tool
//! inventory, so this module only handles the execution-time argument rewrite.

use std::path::PathBuf;

use codex_auth_types::RequestAuthSnapshot;
use codex_openai_files_api::OpenAiFileUploadAuth;
use codex_openai_files_api::OpenAiFileUploader;
use model_service_api::AuthProvider;
use model_service_api::auth_provider_from_auth_snapshot;
use serde_json::Value as JsonValue;

/// Resolves user-provided Apps SDK file argument paths before upload.
///
/// Implementations should apply the same path semantics as the owning turn or
/// host runtime. The MCP runtime only needs the resolved local path and does not
/// depend on a concrete session or turn context.
pub trait OpenAiFilePathResolver: Send + Sync {
    /// Resolves a raw tool argument file path into the local path to upload.
    fn resolve_path(&self, file_path: &str) -> PathBuf;
}

pub async fn rewrite_mcp_tool_arguments_for_openai_files(
    uploader: &dyn OpenAiFileUploader,
    auth: Option<&RequestAuthSnapshot>,
    chatgpt_base_url: &str,
    path_resolver: &dyn OpenAiFilePathResolver,
    arguments_value: Option<JsonValue>,
    openai_file_input_params: Option<&[String]>,
) -> Result<Option<JsonValue>, String> {
    let Some(openai_file_input_params) = openai_file_input_params else {
        return Ok(arguments_value);
    };

    let Some(arguments_value) = arguments_value else {
        return Ok(None);
    };
    let Some(arguments) = arguments_value.as_object() else {
        return Ok(Some(arguments_value));
    };
    let mut rewritten_arguments = arguments.clone();

    for field_name in openai_file_input_params {
        let Some(value) = arguments.get(field_name) else {
            continue;
        };
        let Some(uploaded_value) = rewrite_argument_value_for_openai_files(
            uploader,
            auth,
            chatgpt_base_url,
            path_resolver,
            field_name,
            value,
        )
        .await?
        else {
            continue;
        };
        rewritten_arguments.insert(field_name.clone(), uploaded_value);
    }

    if rewritten_arguments == *arguments {
        return Ok(Some(arguments_value));
    }

    Ok(Some(JsonValue::Object(rewritten_arguments)))
}

async fn rewrite_argument_value_for_openai_files(
    uploader: &dyn OpenAiFileUploader,
    auth: Option<&RequestAuthSnapshot>,
    chatgpt_base_url: &str,
    path_resolver: &dyn OpenAiFilePathResolver,
    field_name: &str,
    value: &JsonValue,
) -> Result<Option<JsonValue>, String> {
    match value {
        JsonValue::String(path_or_file_ref) => {
            let rewritten = build_uploaded_local_argument_value(
                uploader,
                auth,
                chatgpt_base_url,
                path_resolver,
                field_name,
                /*index*/ None,
                path_or_file_ref,
            )
            .await?;
            Ok(Some(rewritten))
        }
        JsonValue::Array(values) => {
            let mut rewritten_values = Vec::with_capacity(values.len());
            for (index, item) in values.iter().enumerate() {
                let Some(path_or_file_ref) = item.as_str() else {
                    return Ok(None);
                };
                let rewritten = build_uploaded_local_argument_value(
                    uploader,
                    auth,
                    chatgpt_base_url,
                    path_resolver,
                    field_name,
                    Some(index),
                    path_or_file_ref,
                )
                .await?;
                rewritten_values.push(rewritten);
            }
            Ok(Some(JsonValue::Array(rewritten_values)))
        }
        _ => Ok(None),
    }
}

async fn build_uploaded_local_argument_value(
    uploader: &dyn OpenAiFileUploader,
    auth: Option<&RequestAuthSnapshot>,
    chatgpt_base_url: &str,
    path_resolver: &dyn OpenAiFilePathResolver,
    field_name: &str,
    index: Option<usize>,
    file_path: &str,
) -> Result<JsonValue, String> {
    let resolved_path = path_resolver.resolve_path(file_path);
    let Some(auth) = auth else {
        return Err(
            "ChatGPT auth is required to upload local files for Codex Apps tools".to_string(),
        );
    };
    if !auth.uses_codex_backend() {
        return Err(
            "ChatGPT auth is required to upload local files for Codex Apps tools".to_string(),
        );
    }
    let upload_auth = auth_provider_from_auth_snapshot(auth);
    let upload_auth = ApiProviderOpenAiFileUploadAuth {
        auth: upload_auth.as_ref(),
    };
    let uploaded = uploader
        .upload_local_file(
            chatgpt_base_url.trim_end_matches('/'),
            &upload_auth,
            &resolved_path,
        )
        .await
        .map_err(|error| match index {
            Some(index) => {
                format!("failed to upload `{file_path}` for `{field_name}[{index}]`: {error}")
            }
            None => format!("failed to upload `{file_path}` for `{field_name}`: {error}"),
        })?;
    Ok(serde_json::json!({
        "download_url": uploaded.download_url,
        "file_id": uploaded.file_id,
        "mime_type": uploaded.mime_type,
        "file_name": uploaded.file_name,
        "uri": uploaded.uri,
        "file_size_bytes": uploaded.file_size_bytes,
    }))
}

struct ApiProviderOpenAiFileUploadAuth<'a> {
    auth: &'a dyn AuthProvider,
}

impl OpenAiFileUploadAuth for ApiProviderOpenAiFileUploadAuth<'_> {
    fn add_auth_headers(&self, headers: &mut http::HeaderMap) {
        self.auth.add_auth_headers(headers);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use codex_auth_types::AuthMode;
    use codex_auth_types::BearerRequestAuthSnapshot;
    use codex_auth_types::RequestAuthSnapshot;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;

    struct CwdPathResolver {
        cwd: PathBuf,
    }

    impl CwdPathResolver {
        fn new(cwd: &Path) -> Self {
            Self {
                cwd: cwd.to_path_buf(),
            }
        }
    }

    impl OpenAiFilePathResolver for CwdPathResolver {
        fn resolve_path(&self, file_path: &str) -> PathBuf {
            let path = PathBuf::from(file_path);
            if path.is_absolute() {
                return path;
            }
            self.cwd.join(path)
        }
    }

    fn chatgpt_auth() -> RequestAuthSnapshot {
        RequestAuthSnapshot::Bearer(BearerRequestAuthSnapshot {
            auth_mode: AuthMode::Chatgpt,
            token: Some("access-token".to_string()),
            account_id: Some("account_id".to_string()),
            chatgpt_user_id: Some("user_id".to_string()),
            is_workspace_account: true,
            is_fedramp_account: false,
        })
    }

    fn reqwest_uploader() -> codex_openai_files::ReqwestOpenAiFileUploader {
        codex_openai_files::ReqwestOpenAiFileUploader
    }

    async fn mount_upload_mocks(
        server: &wiremock::MockServer,
        file_name: &str,
        file_id: &str,
        file_size: u64,
    ) {
        use wiremock::Mock;
        use wiremock::ResponseTemplate;
        use wiremock::matchers::body_json;
        use wiremock::matchers::header;
        use wiremock::matchers::method;
        use wiremock::matchers::path;

        Mock::given(method("POST"))
            .and(path("/backend-api/files"))
            .and(header("chatgpt-account-id", "account_id"))
            .and(body_json(serde_json::json!({
                "file_name": file_name,
                "file_size": file_size,
                "use_case": "codex",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "file_id": file_id,
                "upload_url": format!("{}/upload/{file_id}", server.uri()),
            })))
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("PUT"))
            .and(path(format!("/upload/{file_id}")))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/backend-api/files/{file_id}/uploaded")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "success",
                "download_url": format!("{}/download/{file_id}", server.uri()),
                "file_name": file_name,
                "mime_type": "text/csv",
                "file_size_bytes": file_size,
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    fn write_local_file(dir: &TempDir, file_name: &str, body: &[u8]) {
        std::fs::write(dir.path().join(file_name), body).expect("write local file");
    }

    #[tokio::test]
    async fn openai_file_argument_rewrite_requires_declared_file_params() {
        let dir = tempdir().expect("temp dir");
        let resolver = CwdPathResolver::new(dir.path());
        let arguments = Some(serde_json::json!({
            "file": "/tmp/codex-smoke-file.txt"
        }));

        let rewritten = rewrite_mcp_tool_arguments_for_openai_files(
            &reqwest_uploader(),
            Some(&chatgpt_auth()),
            "https://chatgpt.com/backend-api",
            &resolver,
            arguments.clone(),
            /*openai_file_input_params*/ None,
        )
        .await
        .expect("rewrite should succeed");

        assert_eq!(rewritten, arguments);
    }

    #[tokio::test]
    async fn build_uploaded_local_argument_value_uploads_local_file_path() {
        let server = wiremock::MockServer::start().await;
        mount_upload_mocks(&server, "file_report.csv", "file_123", 5).await;

        let dir = tempdir().expect("temp dir");
        write_local_file(&dir, "file_report.csv", b"hello");
        let resolver = CwdPathResolver::new(dir.path());
        let auth = chatgpt_auth();
        let uploader = reqwest_uploader();
        let rewritten = build_uploaded_local_argument_value(
            &uploader,
            Some(&auth),
            &format!("{}/backend-api", server.uri()),
            &resolver,
            "file",
            /*index*/ None,
            "file_report.csv",
        )
        .await
        .expect("rewrite should upload the local file");

        assert_eq!(
            rewritten,
            serde_json::json!({
                "download_url": format!("{}/download/file_123", server.uri()),
                "file_id": "file_123",
                "mime_type": "text/csv",
                "file_name": "file_report.csv",
                "uri": "sediment://file_123",
                "file_size_bytes": 5,
            })
        );
    }

    #[tokio::test]
    async fn rewrite_argument_value_for_openai_files_rewrites_scalar_path() {
        let server = wiremock::MockServer::start().await;
        mount_upload_mocks(&server, "file_report.csv", "file_123", 5).await;

        let dir = tempdir().expect("temp dir");
        write_local_file(&dir, "file_report.csv", b"hello");
        let resolver = CwdPathResolver::new(dir.path());
        let auth = chatgpt_auth();
        let uploader = reqwest_uploader();
        let rewritten = rewrite_argument_value_for_openai_files(
            &uploader,
            Some(&auth),
            &format!("{}/backend-api", server.uri()),
            &resolver,
            "file",
            &serde_json::json!("file_report.csv"),
        )
        .await
        .expect("rewrite should succeed");

        assert_eq!(
            rewritten,
            Some(serde_json::json!({
                "download_url": format!("{}/download/file_123", server.uri()),
                "file_id": "file_123",
                "mime_type": "text/csv",
                "file_name": "file_report.csv",
                "uri": "sediment://file_123",
                "file_size_bytes": 5,
            }))
        );
    }

    #[tokio::test]
    async fn rewrite_argument_value_for_openai_files_rewrites_array_paths() {
        let server = wiremock::MockServer::start().await;
        mount_upload_mocks(&server, "one.csv", "file_1", 3).await;
        mount_upload_mocks(&server, "two.csv", "file_2", 3).await;

        let dir = tempdir().expect("temp dir");
        write_local_file(&dir, "one.csv", b"one");
        write_local_file(&dir, "two.csv", b"two");
        let resolver = CwdPathResolver::new(dir.path());
        let auth = chatgpt_auth();
        let uploader = reqwest_uploader();
        let rewritten = rewrite_argument_value_for_openai_files(
            &uploader,
            Some(&auth),
            &format!("{}/backend-api", server.uri()),
            &resolver,
            "files",
            &serde_json::json!(["one.csv", "two.csv"]),
        )
        .await
        .expect("rewrite should succeed");

        assert_eq!(
            rewritten,
            Some(serde_json::json!([
                {
                    "download_url": format!("{}/download/file_1", server.uri()),
                    "file_id": "file_1",
                    "mime_type": "text/csv",
                    "file_name": "one.csv",
                    "uri": "sediment://file_1",
                    "file_size_bytes": 3,
                },
                {
                    "download_url": format!("{}/download/file_2", server.uri()),
                    "file_id": "file_2",
                    "mime_type": "text/csv",
                    "file_name": "two.csv",
                    "uri": "sediment://file_2",
                    "file_size_bytes": 3,
                }
            ]))
        );
    }

    #[tokio::test]
    async fn rewrite_mcp_tool_arguments_for_openai_files_surfaces_upload_failures() {
        let dir = tempdir().expect("temp dir");
        let resolver = CwdPathResolver::new(dir.path());
        let auth = chatgpt_auth();
        let error = rewrite_mcp_tool_arguments_for_openai_files(
            &reqwest_uploader(),
            Some(&auth),
            "https://chatgpt.com/backend-api",
            &resolver,
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
