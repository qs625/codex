use crate::AgentRoleConfig;
use crate::DEFAULT_ROLE_NAME;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

/// Returns the cached built-in role declarations.
pub fn configs() -> &'static BTreeMap<String, AgentRoleConfig> {
    static CONFIG: LazyLock<BTreeMap<String, AgentRoleConfig>> = LazyLock::new(|| {
        BTreeMap::from([(
            DEFAULT_ROLE_NAME.to_string(),
            AgentRoleConfig {
                description: Some("Default agent.".to_string()),
                config_file: None,
                nickname_candidates: None,
                ..Default::default()
            },
        )])
    });
    &CONFIG
}

/// Resolves a built-in role `config_file` path to embedded content.
pub fn config_file_contents(path: &Path) -> Option<&'static str> {
    const AWAITER: &str = include_str!("builtins/awaiter.toml");
    match path.to_str()? {
        "awaiter.toml" => Some(AWAITER),
        _ => None,
    }
}
