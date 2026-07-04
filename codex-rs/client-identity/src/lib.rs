//! Shared Codex client identity headers and User-Agent helpers.

use codex_terminal_detection::user_agent;
use http::HeaderMap;
use http::HeaderValue;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::RwLock;

pub const DEFAULT_ORIGINATOR: &str = "codex_cli_rs";
pub const CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";
pub const RESIDENCY_HEADER_NAME: &str = "x-openai-internal-codex-residency";

pub use codex_config_types::ResidencyRequirement;

#[derive(Debug, Clone)]
pub struct Originator {
    pub value: String,
    pub header_value: HeaderValue,
}

static ORIGINATOR: LazyLock<RwLock<Option<Originator>>> = LazyLock::new(|| RwLock::new(None));
static REQUIREMENTS_RESIDENCY: LazyLock<RwLock<Option<ResidencyRequirement>>> =
    LazyLock::new(|| RwLock::new(None));
pub static USER_AGENT_SUFFIX: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Debug)]
pub enum SetOriginatorError {
    InvalidHeaderValue,
    AlreadyInitialized,
}

fn get_originator_value(provided: Option<String>) -> Originator {
    let value = std::env::var(CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR)
        .ok()
        .or(provided)
        .unwrap_or(DEFAULT_ORIGINATOR.to_string());

    match HeaderValue::from_str(&value) {
        Ok(header_value) => Originator {
            value,
            header_value,
        },
        Err(e) => {
            tracing::error!("Unable to turn originator override {value} into header value: {e}");
            Originator {
                value: DEFAULT_ORIGINATOR.to_string(),
                header_value: HeaderValue::from_static(DEFAULT_ORIGINATOR),
            }
        }
    }
}

pub fn set_default_originator(value: String) -> Result<(), SetOriginatorError> {
    if HeaderValue::from_str(&value).is_err() {
        return Err(SetOriginatorError::InvalidHeaderValue);
    }
    let originator = get_originator_value(Some(value));
    let Ok(mut guard) = ORIGINATOR.write() else {
        return Err(SetOriginatorError::AlreadyInitialized);
    };
    if guard.is_some() {
        return Err(SetOriginatorError::AlreadyInitialized);
    }
    *guard = Some(originator);
    Ok(())
}

pub fn set_default_client_residency_requirement(enforce_residency: Option<ResidencyRequirement>) {
    let Ok(mut guard) = REQUIREMENTS_RESIDENCY.write() else {
        tracing::warn!("Failed to acquire requirements residency lock");
        return;
    };
    *guard = enforce_residency;
}

pub fn originator() -> Originator {
    if let Ok(guard) = ORIGINATOR.read()
        && let Some(originator) = guard.as_ref()
    {
        return originator.clone();
    }

    if std::env::var(CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR).is_ok() {
        let originator = get_originator_value(/*provided*/ None);
        if let Ok(mut guard) = ORIGINATOR.write() {
            match guard.as_ref() {
                Some(originator) => return originator.clone(),
                None => *guard = Some(originator.clone()),
            }
        }
        return originator;
    }

    get_originator_value(/*provided*/ None)
}

pub fn is_first_party_originator(originator_value: &str) -> bool {
    originator_value == DEFAULT_ORIGINATOR
        || originator_value == "codex-tui"
        || originator_value == "codex_vscode"
        || originator_value.starts_with("Codex ")
}

pub fn is_first_party_chat_originator(originator_value: &str) -> bool {
    originator_value == "codex_atlas" || originator_value == "codex_chatgpt_desktop"
}

pub fn default_identity_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("originator", originator().header_value);
    if let Ok(guard) = REQUIREMENTS_RESIDENCY.read()
        && let Some(requirement) = guard.as_ref()
        && !headers.contains_key(RESIDENCY_HEADER_NAME)
    {
        let value = match requirement {
            ResidencyRequirement::Us => HeaderValue::from_static("us"),
        };
        headers.insert(RESIDENCY_HEADER_NAME, value);
    }
    headers
}

pub fn get_codex_user_agent() -> String {
    let build_version = env!("CARGO_PKG_VERSION");
    let os_info = os_info::get();
    let originator = originator();
    let prefix = format!(
        "{}/{build_version} ({} {}; {}) {}",
        originator.value.as_str(),
        os_info.os_type(),
        os_info.version(),
        os_info.architecture().unwrap_or("unknown"),
        user_agent()
    );
    let suffix = USER_AGENT_SUFFIX
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    let suffix = suffix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(String::new, |value| format!(" ({value})"));

    let candidate = format!("{prefix}{suffix}");
    sanitize_user_agent(candidate, &prefix)
}

fn sanitize_user_agent(candidate: String, fallback: &str) -> String {
    if HeaderValue::from_str(candidate.as_str()).is_ok() {
        return candidate;
    }

    let sanitized: String = candidate
        .chars()
        .map(|ch| if matches!(ch, ' '..='~') { ch } else { '_' })
        .collect();
    if !sanitized.is_empty() && HeaderValue::from_str(sanitized.as_str()).is_ok() {
        tracing::warn!(
            "Sanitized Codex user agent because provided suffix contained invalid header characters"
        );
        sanitized
    } else if HeaderValue::from_str(fallback).is_ok() {
        tracing::warn!(
            "Falling back to base Codex user agent because provided suffix could not be sanitized"
        );
        fallback.to_string()
    } else {
        tracing::warn!(
            "Falling back to default Codex originator because base user agent string is invalid"
        );
        originator().value
    }
}

pub fn default_headers() -> HeaderMap {
    let mut headers = default_identity_headers();
    if let Ok(user_agent) = HeaderValue::from_str(&get_codex_user_agent()) {
        headers.insert(http::header::USER_AGENT, user_agent);
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn is_first_party_originator_matches_known_values() {
        assert_eq!(is_first_party_originator(DEFAULT_ORIGINATOR), true);
        assert_eq!(is_first_party_originator("codex-tui"), true);
        assert_eq!(is_first_party_originator("codex_vscode"), true);
        assert_eq!(is_first_party_originator("Codex Something Else"), true);
        assert_eq!(is_first_party_originator("codex_cli"), false);
        assert_eq!(is_first_party_originator("Other"), false);
    }

    #[test]
    fn is_first_party_chat_originator_matches_known_values() {
        assert_eq!(is_first_party_chat_originator("codex_atlas"), true);
        assert_eq!(
            is_first_party_chat_originator("codex_chatgpt_desktop"),
            true
        );
        assert_eq!(is_first_party_chat_originator(DEFAULT_ORIGINATOR), false);
        assert_eq!(is_first_party_chat_originator("codex_vscode"), false);
    }

    #[test]
    fn default_identity_headers_include_originator_and_residency() {
        set_default_client_residency_requirement(Some(ResidencyRequirement::Us));

        let headers = default_identity_headers();

        let originator_header = headers
            .get("originator")
            .expect("originator header missing");
        assert_eq!(originator_header.to_str().unwrap(), originator().value);

        let residency_header = headers
            .get(RESIDENCY_HEADER_NAME)
            .expect("residency header missing");
        assert_eq!(residency_header.to_str().unwrap(), "us");

        set_default_client_residency_requirement(/*enforce_residency*/ None);
    }

    #[test]
    fn test_get_codex_user_agent() {
        let user_agent = get_codex_user_agent();
        let originator = originator().value;
        let prefix = format!("{originator}/");
        assert!(user_agent.starts_with(&prefix));
    }

    #[test]
    fn default_headers_include_identity_headers() {
        set_default_client_residency_requirement(Some(ResidencyRequirement::Us));

        let headers = default_headers();

        let originator_header = headers
            .get("originator")
            .expect("originator header missing");
        assert_eq!(originator_header.to_str().unwrap(), originator().value);

        let expected_ua = get_codex_user_agent();
        let ua_header = headers
            .get("user-agent")
            .expect("user-agent header missing");
        assert_eq!(ua_header.to_str().unwrap(), expected_ua);

        let residency_header = headers
            .get(RESIDENCY_HEADER_NAME)
            .expect("residency header missing");
        assert_eq!(residency_header.to_str().unwrap(), "us");

        set_default_client_residency_requirement(/*enforce_residency*/ None);
    }

    #[test]
    fn test_invalid_suffix_is_sanitized() {
        let prefix = "codex_cli_rs/0.0.0";
        let suffix = "bad\rsuffix";

        assert_eq!(
            sanitize_user_agent(format!("{prefix} ({suffix})"), prefix),
            "codex_cli_rs/0.0.0 (bad_suffix)"
        );
    }

    #[test]
    fn test_invalid_suffix_is_sanitized2() {
        let prefix = "codex_cli_rs/0.0.0";
        let suffix = "bad\0suffix";

        assert_eq!(
            sanitize_user_agent(format!("{prefix} ({suffix})"), prefix),
            "codex_cli_rs/0.0.0 (bad_suffix)"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_macos() {
        use regex_lite::Regex;
        let user_agent = get_codex_user_agent();
        let originator = regex_lite::escape(originator().value.as_str());
        let re = Regex::new(&format!(
            r"^{originator}/\d+\.\d+\.\d+ \(Mac OS \d+\.\d+\.\d+; (x86_64|arm64)\) (\S+)$"
        ))
        .unwrap();
        assert!(re.is_match(&user_agent));
    }
}
