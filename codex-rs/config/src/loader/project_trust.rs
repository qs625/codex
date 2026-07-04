use std::collections::HashMap;
use std::path::Path;

use dunce::canonicalize as normalize_path;
use protocol::config_types::TrustLevel;

/// Canonicalizes a path into the key used by the projects trust map.
pub fn project_trust_key(path: &Path) -> String {
    normalized_project_trust_keys(path)
        .into_iter()
        .next()
        .unwrap_or_else(|| normalize_project_trust_lookup_key(path.to_string_lossy().to_string()))
}

/// Returns canonical and original lookup keys for project trust matching.
pub fn normalized_project_trust_keys(path: &Path) -> Vec<String> {
    let normalized_path = normalize_project_trust_lookup_key(path.to_string_lossy().to_string());
    let normalized_canonical_path = normalize_project_trust_lookup_key(
        normalize_path(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string(),
    );
    if normalized_path == normalized_canonical_path {
        vec![normalized_canonical_path]
    } else {
        vec![normalized_canonical_path, normalized_path]
    }
}

/// Finds a project trust entry by exact or normalized lookup key.
pub fn project_trust_for_lookup_key(
    projects_trust: &HashMap<String, TrustLevel>,
    lookup_key: &str,
) -> Option<(String, TrustLevel)> {
    if let Some(trust_level) = projects_trust.get(lookup_key).copied() {
        return Some((lookup_key.to_string(), trust_level));
    }

    let mut normalized_matches: Vec<_> = projects_trust
        .iter()
        .filter(|(key, _)| normalize_project_trust_lookup_key((*key).clone()) == lookup_key)
        .collect();
    normalized_matches.sort_by(|(left, _), (right, _)| left.cmp(right));
    normalized_matches
        .first()
        .map(|(key, trust_level)| ((**key).clone(), **trust_level))
}

fn normalize_project_trust_lookup_key(key: String) -> String {
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}
