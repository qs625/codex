use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use codex_api_provider::AuthProvider;
use serde::Deserialize;
use serde::Serialize;

/// Uploaded OpenAI file metadata returned by an OpenAI file upload runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadedOpenAiFile {
    pub file_id: String,
    pub uri: String,
    pub download_url: String,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub mime_type: Option<String>,
    pub path: PathBuf,
}

pub type OpenAiFileUploadResult = Result<UploadedOpenAiFile, String>;

pub type OpenAiFileUploadFuture<'a> =
    Pin<Box<dyn Future<Output = OpenAiFileUploadResult> + Send + 'a>>;

/// Uploads local files into OpenAI file storage for runtimes that need file-backed tool inputs.
///
/// Implementations own transport, retry, and backend-specific behavior. Callers in core should
/// depend on this trait rather than a concrete HTTP implementation.
pub trait OpenAiFileUploader: Send + Sync {
    fn upload_local_file<'a>(
        &'a self,
        base_url: &'a str,
        auth: &'a dyn AuthProvider,
        path: &'a Path,
    ) -> OpenAiFileUploadFuture<'a>;
}

pub type SharedOpenAiFileUploader = Arc<dyn OpenAiFileUploader>;

/// Uploader used by runtimes that do not provide OpenAI file storage.
#[derive(Debug, Default)]
pub struct DisabledOpenAiFileUploader;

impl OpenAiFileUploader for DisabledOpenAiFileUploader {
    fn upload_local_file<'a>(
        &'a self,
        _base_url: &'a str,
        _auth: &'a dyn AuthProvider,
        _path: &'a Path,
    ) -> OpenAiFileUploadFuture<'a> {
        Box::pin(async { Err("OpenAI file uploads are not available in this runtime".to_string()) })
    }
}
