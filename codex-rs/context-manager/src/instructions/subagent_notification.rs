use codex_protocol::protocol::AgentStatus;

use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq)]
pub struct SubagentNotification {
    pub agent_reference: String,
    pub status: AgentStatus,
}

impl SubagentNotification {
    pub fn new(agent_reference: impl Into<String>, status: AgentStatus) -> Self {
        Self {
            agent_reference: agent_reference.into(),
            status,
        }
    }
}

impl ContextualUserFragment for SubagentNotification {
    const ROLE: &'static str = "user";
    const START_MARKER: &'static str = "<subagent_notification>";
    const END_MARKER: &'static str = "</subagent_notification>";

    fn body(&self) -> String {
        format!(
            "\n{}\n",
            serde_json::json!({
                "agent_path": &self.agent_reference,
                "status": &self.status,
            })
        )
    }
}
