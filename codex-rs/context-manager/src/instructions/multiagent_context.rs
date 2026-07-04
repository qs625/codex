use protocol::AgentPath;

use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiagentContext {
    pub current_thread_path: AgentPath,
    pub direct_subagent_paths: Vec<AgentPath>,
}

impl MultiagentContext {
    pub fn new(current_thread_path: AgentPath, direct_subagent_paths: Vec<AgentPath>) -> Self {
        Self {
            current_thread_path,
            direct_subagent_paths,
        }
    }
}

impl ContextualUserFragment for MultiagentContext {
    const ROLE: &'static str = "user";
    const START_MARKER: &'static str = "<multiagent_context>";
    const END_MARKER: &'static str = "</multiagent_context>";

    fn body(&self) -> String {
        let mut lines = vec![format!(
            "  <current_thread_canonical_path>{}</current_thread_canonical_path>",
            self.current_thread_path
        )];
        if !self.direct_subagent_paths.is_empty() {
            lines.push("  <direct_subagents>".to_string());
            lines.extend(
                self.direct_subagent_paths
                    .iter()
                    .map(|path| format!("    <canonical_path>{path}</canonical_path>")),
            );
            lines.push("  </direct_subagents>".to_string());
        }
        format!("\n{}\n", lines.join("\n"))
    }
}

#[cfg(test)]
#[path = "multiagent_context_tests.rs"]
mod tests;
