pub(crate) mod agent_resolver;
pub(crate) mod control;
pub(crate) mod external;
pub(crate) mod job_tools;
pub(crate) mod multi_agent;
pub(crate) mod role;
pub(crate) mod spawn_support;

pub(crate) mod registry {
    pub(crate) use codex_agent_runtime::AgentMode;
    pub(crate) use codex_agent_runtime::SpawnAgentOptions;
    pub(crate) use codex_agent_runtime::exceeds_thread_spawn_depth_limit;
    pub(crate) use codex_agent_runtime::next_thread_spawn_depth;
}

pub(crate) mod status {
    use protocol::protocol::AgentStatus;

    pub(crate) use codex_agent_runtime::agent_status_from_event;
    pub(crate) use codex_agent_runtime::is_final;

    const MAX_CHILD_COMPLETION_CONTENT_CHARS: usize = 12_000;

    pub(crate) fn child_completion_content_from_status(status: &AgentStatus) -> String {
        let content = match status {
            AgentStatus::Completed(Some(message)) if !message.trim().is_empty() => message.trim(),
            AgentStatus::Completed(_) => "completed",
            AgentStatus::Errored(message) if !message.trim().is_empty() => message.trim(),
            AgentStatus::Errored(_) => "errored",
            AgentStatus::Shutdown => "shutdown",
            AgentStatus::NotFound => "not found",
            AgentStatus::PendingInit => "pending initialization",
            AgentStatus::Running => "running",
            AgentStatus::Interrupted => "interrupted",
        };
        truncate_child_completion_content(content)
    }

    fn truncate_child_completion_content(content: &str) -> String {
        if content.chars().count() <= MAX_CHILD_COMPLETION_CONTENT_CHARS {
            return content.to_string();
        }
        let suffix = "...";
        let content_chars = MAX_CHILD_COMPLETION_CONTENT_CHARS.saturating_sub(suffix.len());
        let mut truncated = content.chars().take(content_chars).collect::<String>();
        truncated.push_str(suffix);
        truncated
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn child_completion_content_maps_typed_status_to_plain_text() {
            assert_eq!(
                child_completion_content_from_status(&AgentStatus::Completed(Some(
                    " done ".to_string()
                ))),
                "done"
            );
            assert_eq!(
                child_completion_content_from_status(&AgentStatus::Completed(None)),
                "completed"
            );
            assert_eq!(
                child_completion_content_from_status(&AgentStatus::Errored(" failed ".to_string())),
                "failed"
            );
            assert_eq!(
                child_completion_content_from_status(&AgentStatus::Errored(String::new())),
                "errored"
            );
            assert_eq!(
                child_completion_content_from_status(&AgentStatus::Shutdown),
                "shutdown"
            );
            assert_eq!(
                child_completion_content_from_status(&AgentStatus::NotFound),
                "not found"
            );
        }

        #[test]
        fn child_completion_content_truncates_to_total_limit() {
            let content = "x".repeat(MAX_CHILD_COMPLETION_CONTENT_CHARS + 10);

            let truncated =
                child_completion_content_from_status(&AgentStatus::Completed(Some(content)));

            assert_eq!(
                truncated.chars().count(),
                MAX_CHILD_COMPLETION_CONTENT_CHARS
            );
            assert!(truncated.ends_with("..."));
        }
    }
}

pub(crate) use control::AgentControl;
pub(crate) use protocol::protocol::AgentStatus;
pub(crate) use registry::AgentMode;
pub(crate) use registry::SpawnAgentOptions;
pub(crate) use registry::exceeds_thread_spawn_depth_limit;
pub(crate) use registry::next_thread_spawn_depth;
pub(crate) use status::agent_status_from_event;
pub(crate) use status::child_completion_content_from_status;
