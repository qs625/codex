use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexAppsAuthContext {
    pub uses_codex_backend: bool,
    pub account_id: Option<String>,
    pub chatgpt_user_id: Option<String>,
    pub is_workspace_account: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexAppsToolsCacheKey {
    account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    is_workspace_account: bool,
}

pub fn codex_apps_tools_cache_key(
    auth_context: Option<&CodexAppsAuthContext>,
) -> CodexAppsToolsCacheKey {
    match auth_context {
        Some(auth_context) => CodexAppsToolsCacheKey {
            account_id: auth_context.account_id.clone(),
            chatgpt_user_id: auth_context.chatgpt_user_id.clone(),
            is_workspace_account: auth_context.is_workspace_account,
        },
        None => CodexAppsToolsCacheKey::default(),
    }
}
