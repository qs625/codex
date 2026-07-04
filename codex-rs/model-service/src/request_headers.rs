use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;

pub(crate) use model_service_api::build_session_headers;
pub(crate) use model_service_api::insert_header;

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
