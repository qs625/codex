use crate::config::AgentCapabilityAllowlist;
use crate::config::AgentRoleConfig;
use crate::config::AgentRoleSource;

use super::ContextualUserFragment;

const MAX_RENDERED_AGENTS: usize = 32;
const MAX_RENDERED_ALLOWLIST_PATTERNS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableAgentsInstructions {
    agents: Vec<AvailableAgent>,
    omitted_agents: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AvailableAgent {
    name: String,
    description: String,
    source: Option<String>,
    model: Option<String>,
    model_reasoning_effort: Option<String>,
    tools: AgentCapabilityAllowlist,
    skills: AgentCapabilityAllowlist,
}

impl AvailableAgentsInstructions {
    pub(crate) fn from_agent_roles(
        agent_roles: &std::collections::BTreeMap<String, AgentRoleConfig>,
    ) -> Option<Self> {
        let mut agents = agent_roles
            .iter()
            .filter_map(|(name, role)| {
                Some(AvailableAgent {
                    name: name.clone(),
                    description: role.description.clone()?,
                    source: render_source(name, role),
                    model: role.model.clone(),
                    model_reasoning_effort: role.model_reasoning_effort.clone(),
                    tools: role.tool_allowlist.clone(),
                    skills: role.skill_allowlist.clone(),
                })
            })
            .collect::<Vec<_>>();
        let omitted_agents = agents.len().saturating_sub(MAX_RENDERED_AGENTS);
        agents.truncate(MAX_RENDERED_AGENTS);
        (!agents.is_empty()).then_some(Self {
            agents,
            omitted_agents,
        })
    }
}

impl ContextualUserFragment for AvailableAgentsInstructions {
    const ROLE: &'static str = "developer";
    const START_MARKER: &'static str = "<agents_instructions>";
    const END_MARKER: &'static str = "</agents_instructions>";

    fn body(&self) -> String {
        let mut lines = vec![
            "## Agents".to_string(),
            "Agents are reusable subagent definitions. Use `spawn_agent.agent_type` to start one."
                .to_string(),
            "### Available agents".to_string(),
        ];

        for agent in &self.agents {
            lines.push(format!("- `{}`: {}", agent.name, agent.description));
            if let Some(source) = &agent.source {
                lines.push(format!("  Source: {source}"));
            }
            if let Some(model) = &agent.model {
                lines.push(format!("  Model: {model}"));
            }
            if let Some(effort) = &agent.model_reasoning_effort {
                lines.push(format!("  Effort: {effort}"));
            }
            if let Some(tools) = render_allowlist("Tools", &agent.tools) {
                lines.push(tools);
            }
            if let Some(skills) = render_allowlist("Skills", &agent.skills) {
                lines.push(skills);
            }
        }

        if self.omitted_agents > 0 {
            lines.push(format!(
                "- {} additional agents omitted from this list.",
                self.omitted_agents
            ));
        }

        lines.push("### How to use agents".to_string());
        lines.push(
            "- Choose an `agent_type` whose description matches the delegated task.".to_string(),
        );
        lines.push("- The agent body is applied only to the spawned agent.".to_string());

        format!("\n{}\n", lines.join("\n"))
    }
}

fn render_source(name: &str, role: &AgentRoleConfig) -> Option<String> {
    match &role.source {
        Some(AgentRoleSource::Plugin { plugin_id }) => Some(format!("plugin: {plugin_id}/{name}")),
        None => role
            .source_path
            .as_ref()
            .map(|path| path.display().to_string()),
    }
}

fn render_allowlist(label: &str, allowlist: &AgentCapabilityAllowlist) -> Option<String> {
    match allowlist {
        AgentCapabilityAllowlist::Inherit => None,
        AgentCapabilityAllowlist::All => Some(format!("  {label}: *")),
        AgentCapabilityAllowlist::Patterns(patterns) => {
            let rendered = patterns
                .iter()
                .take(MAX_RENDERED_ALLOWLIST_PATTERNS)
                .cloned()
                .collect::<Vec<_>>();
            let omitted = patterns
                .len()
                .saturating_sub(MAX_RENDERED_ALLOWLIST_PATTERNS);
            let suffix = if omitted > 0 {
                format!(", ... ({omitted} more)")
            } else {
                String::new()
            };
            Some(format!("  {label}: {}{suffix}", rendered.join(", ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;

    use crate::config::AgentCapabilityAllowlist;
    use crate::config::AgentRoleConfig;
    use crate::config::AgentRoleSource;
    use crate::context::ContextualUserFragment;

    use super::AvailableAgentsInstructions;

    #[test]
    fn renders_loaded_agent_frontmatter_without_body() {
        let fragment = AvailableAgentsInstructions::from_agent_roles(&BTreeMap::from([(
            "reviewer".to_string(),
            AgentRoleConfig {
                description: Some("Review code changes.".to_string()),
                model: Some("gpt-5.4".to_string()),
                model_reasoning_effort: Some("high".to_string()),
                source: Some(AgentRoleSource::Plugin {
                    plugin_id: "code-review".to_string(),
                }),
                tool_allowlist: AgentCapabilityAllowlist::Patterns(vec![
                    "exec_command".to_string(),
                    "apply_patch".to_string(),
                ]),
                skill_allowlist: AgentCapabilityAllowlist::All,
                ..Default::default()
            },
        )]))
        .expect("agent instructions should render");

        assert_eq!(
            "\n## Agents\nAgents are reusable subagent definitions. Use `spawn_agent.agent_type` to start one.\n### Available agents\n- `reviewer`: Review code changes.\n  Source: plugin: code-review/reviewer\n  Model: gpt-5.4\n  Effort: high\n  Tools: exec_command, apply_patch\n  Skills: *\n### How to use agents\n- Choose an `agent_type` whose description matches the delegated task.\n- The agent body is applied only to the spawned agent.\n",
            fragment.body()
        );
    }
}
