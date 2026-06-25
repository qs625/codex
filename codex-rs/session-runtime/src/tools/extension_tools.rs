use std::sync::Arc;

use codex_extension_api::ExtensionToolExecutor;

use crate::session::session::Session;

pub(crate) fn extension_tool_executors(session: &Session) -> Vec<Arc<dyn ExtensionToolExecutor>> {
    session
        .services
        .extensions
        .tool_contributors()
        .iter()
        .flat_map(|contributor| {
            contributor.tools(
                &session.services.session_extension_data,
                &session.services.thread_extension_data,
            )
        })
        .collect()
}
