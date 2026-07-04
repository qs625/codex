use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use codex_auth_types::RequestAuthSnapshot;
use transport_client_types::Request;
use http::Method;
use model_service_api::ModelSelectionPolicy;
use model_service_api::ModelServiceApi;
use model_service_api::OPENAI_PROVIDER_ID;
use model_service_api::ProviderHttpRequest;

const REMOTE_SKILLS_API_TIMEOUT: Duration = Duration::from_secs(30);

// Low-level client for the remote skill API. This is intentionally kept around for
// future wiring, but it is not used yet by any active product surface.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSkillScope {
    WorkspaceShared,
    AllShared,
    Personal,
    Example,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSkillProductSurface {
    Chatgpt,
    Codex,
    Api,
    Atlas,
}

fn as_query_scope(scope: RemoteSkillScope) -> Option<&'static str> {
    match scope {
        RemoteSkillScope::WorkspaceShared => Some("workspace-shared"),
        RemoteSkillScope::AllShared => Some("all-shared"),
        RemoteSkillScope::Personal => Some("personal"),
        RemoteSkillScope::Example => Some("example"),
    }
}

fn as_query_product_surface(product_surface: RemoteSkillProductSurface) -> &'static str {
    match product_surface {
        RemoteSkillProductSurface::Chatgpt => "chatgpt",
        RemoteSkillProductSurface::Codex => "codex",
        RemoteSkillProductSurface::Api => "api",
        RemoteSkillProductSurface::Atlas => "atlas",
    }
}

fn ensure_codex_backend_auth(auth: Option<&RequestAuthSnapshot>) -> Result<&RequestAuthSnapshot> {
    let Some(auth) = auth else {
        anyhow::bail!("chatgpt authentication required for remote skill scopes");
    };
    if !auth.uses_codex_backend() {
        anyhow::bail!(
            "chatgpt authentication required for remote skill scopes; api key auth is not supported"
        );
    }
    Ok(auth)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSkillDownloadResult {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct RemoteSkillsResponse {
    #[serde(rename = "hazelnuts")]
    skills: Vec<RemoteSkill>,
}

#[derive(Debug, Deserialize)]
struct RemoteSkill {
    id: String,
    name: String,
    description: String,
}

pub async fn list_remote_skills(
    model_service: &dyn ModelServiceApi,
    chatgpt_base_url: String,
    auth: Option<&RequestAuthSnapshot>,
    scope: RemoteSkillScope,
    product_surface: RemoteSkillProductSurface,
    enabled: Option<bool>,
) -> Result<Vec<RemoteSkillSummary>> {
    let base_url = chatgpt_base_url.trim_end_matches('/');
    let auth = ensure_codex_backend_auth(auth)?;

    let url = format!("{base_url}/hazelnuts");
    let product_surface = as_query_product_surface(product_surface);
    let mut query_params = vec![("product_surface", product_surface)];
    if let Some(scope) = as_query_scope(scope) {
        query_params.push(("scope", scope));
    }
    if let Some(enabled) = enabled {
        let enabled = if enabled { "true" } else { "false" };
        query_params.push(("enabled", enabled));
    }

    let request = build_remote_skill_request(url, query_params, auth);
    let response = model_service
        .execute_provider_http(request)
        .await
        .with_context(|| format!("Failed to send request to {url}"))?;
    let body = String::from_utf8(response.body.to_vec())
        .context("Failed to decode skills response body")?;

    let parsed: RemoteSkillsResponse =
        serde_json::from_str(&body).context("Failed to parse skills response")?;

    Ok(parsed
        .skills
        .into_iter()
        .map(|skill| RemoteSkillSummary {
            id: skill.id,
            name: skill.name,
            description: skill.description,
        })
        .collect())
}

pub async fn export_remote_skill(
    model_service: &dyn ModelServiceApi,
    chatgpt_base_url: String,
    codex_home: PathBuf,
    auth: Option<&RequestAuthSnapshot>,
    skill_id: &str,
) -> Result<RemoteSkillDownloadResult> {
    let auth = ensure_codex_backend_auth(auth)?;

    let base_url = chatgpt_base_url.trim_end_matches('/');
    let url = format!("{base_url}/hazelnuts/{skill_id}/export");
    let request = build_remote_skill_request(url.clone(), Vec::new(), auth);
    let response = model_service
        .execute_provider_http(request)
        .await
        .with_context(|| format!("Failed to send download request to {url}"))?;
    let body = response.body;

    if !is_zip_payload(&body) {
        anyhow::bail!("Downloaded remote skill payload is not a zip archive");
    }

    let output_dir = codex_home.join("skills").join(skill_id);
    tokio::fs::create_dir_all(&output_dir)
        .await
        .context("Failed to create downloaded skills directory")?;

    let zip_bytes = body.to_vec();
    let output_dir_clone = output_dir.clone();
    let prefix_candidates = vec![skill_id.to_string()];
    tokio::task::spawn_blocking(move || {
        extract_zip_to_dir(zip_bytes, &output_dir_clone, &prefix_candidates)
    })
    .await
    .context("Zip extraction task failed")??;

    Ok(RemoteSkillDownloadResult {
        id: skill_id.to_string(),
        path: output_dir,
    })
}

fn build_remote_skill_request(
    url: String,
    query_params: Vec<(&'static str, &'static str)>,
    auth: &RequestAuthSnapshot,
) -> ProviderHttpRequest {
    let mut request = Request::new(Method::GET, append_query_params(url, &query_params));
    request.timeout = Some(REMOTE_SKILLS_API_TIMEOUT);
    ProviderHttpRequest {
        selection: ModelSelectionPolicy {
            provider_hint: Some(OPENAI_PROVIDER_ID.to_string()),
            ..Default::default()
        },
        auth: Some(auth.clone()),
        request,
    }
}

fn append_query_params(url: String, query_params: &[(&str, &str)]) -> String {
    if query_params.is_empty() {
        return url;
    }

    let separator = if url.contains('?') { '&' } else { '?' };
    let query = query_params
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{url}{separator}{query}")
}

fn safe_join(base: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => {
                anyhow::bail!("Invalid file path in remote skill payload: {name}");
            }
        }
    }
    Ok(base.join(path))
}

fn is_zip_payload(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
}

fn extract_zip_to_dir(
    bytes: Vec<u8>,
    output_dir: &Path,
    prefix_candidates: &[String],
) -> Result<()> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("Failed to open zip archive")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("Failed to read zip entry")?;
        if file.is_dir() {
            continue;
        }
        let raw_name = file.name().to_string();
        let normalized = normalize_zip_name(&raw_name, prefix_candidates);
        let Some(normalized) = normalized else {
            continue;
        };
        let file_path = safe_join(output_dir, &normalized)?;
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent dir for {normalized}"))?;
        }
        let mut out = std::fs::File::create(&file_path)
            .with_context(|| format!("Failed to create file {normalized}"))?;
        std::io::copy(&mut file, &mut out)
            .with_context(|| format!("Failed to write skill file {normalized}"))?;
    }
    Ok(())
}

fn normalize_zip_name(name: &str, prefix_candidates: &[String]) -> Option<String> {
    let mut trimmed = name.trim_start_matches("./");
    for prefix in prefix_candidates {
        if prefix.is_empty() {
            continue;
        }
        let prefix = format!("{prefix}/");
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            trimmed = rest;
            break;
        }
    }
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
