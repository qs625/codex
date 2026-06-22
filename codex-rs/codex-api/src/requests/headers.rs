pub use codex_api_provider::build_session_headers;
pub(crate) use codex_api_provider::insert_header;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;

pub(crate) fn subagent_header(source: &Option<SessionSource>) -> Option<String> {
    let SessionSource::SubAgent(sub) = source.as_ref()? else {
        return None;
    };
    match sub {
        SubAgentSource::Review => Some("review".to_string()),
        SubAgentSource::Compact => Some("compact".to_string()),
        SubAgentSource::MemoryConsolidation => Some("memory_consolidation".to_string()),
        SubAgentSource::ThreadSpawn { .. } => Some("collab_spawn".to_string()),
        SubAgentSource::Other(label) => Some(label.clone()),
    }
}
