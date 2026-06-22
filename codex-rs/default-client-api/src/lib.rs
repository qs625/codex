//! Lightweight default Codex client User-Agent and full header helpers.
//!
//! This crate owns `User-Agent` construction and full default HTTP headers without depending on
//! reqwest or the concrete HTTP client runtime. Originator and residency state live in
//! `codex-client-identity` and are re-exported here for compatibility.

pub use codex_client_identity::CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR;
pub use codex_client_identity::DEFAULT_ORIGINATOR;
pub use codex_client_identity::Originator;
pub use codex_client_identity::RESIDENCY_HEADER_NAME;
pub use codex_client_identity::ResidencyRequirement;
pub use codex_client_identity::SetOriginatorError;
pub use codex_client_identity::default_identity_headers;
pub use codex_client_identity::is_first_party_chat_originator;
pub use codex_client_identity::is_first_party_originator;
pub use codex_client_identity::originator;
pub use codex_client_identity::set_default_client_residency_requirement;
pub use codex_client_identity::set_default_originator;
use codex_terminal_detection::user_agent;
use http::HeaderMap;
use http::HeaderValue;
use http::header::USER_AGENT;
use std::sync::LazyLock;
use std::sync::Mutex;

/// Set this to add a suffix to the User-Agent string.
///
/// It is not ideal that we're using a global singleton for this.
/// This is primarily designed to differentiate MCP clients from each other.
/// Because there can only be one MCP server per process, it should be safe for this to be a global static.
/// However, future users of this should use this with caution as a result.
/// In addition, we want to be confident that this value is used for ALL clients and doing that requires a
/// lot of wiring and it's easy to miss code paths by doing so.
/// See https://github.com/openai/codex/pull/3388/files for an example of what that would look like.
/// Finally, we want to make sure this is set for ALL mcp clients without needing to know a special env var
/// or having to set data that they already specified in the mcp initialize request somewhere else.
///
/// A space is automatically added between the suffix and the rest of the User-Agent string.
/// The full user agent string is returned from the mcp initialize response.
/// Parenthesis will be added by Codex. This should only specify what goes inside of the parenthesis.
pub static USER_AGENT_SUFFIX: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

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

/// Sanitize the user agent string.
///
/// Invalid characters are replaced with an underscore.
///
/// If the user agent fails to parse, it falls back to fallback and then to ORIGINATOR.
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
        headers.insert(USER_AGENT, user_agent);
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::sanitize_user_agent;
    use super::*;
    use pretty_assertions::assert_eq;

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
