use std::sync::Arc;

mod approval_request;
mod prompt;

pub use approval_request::FormattedGuardianAction;
pub use approval_request::GuardianApprovalRequest;
pub use approval_request::GuardianMcpAnnotations;
pub use approval_request::GuardianNetworkAccessTrigger;
pub use approval_request::format_guardian_action_pretty;
pub use approval_request::guardian_approval_request_to_json;
pub use approval_request::guardian_assessment_action;
pub use approval_request::guardian_request_target_item_id;
pub use approval_request::guardian_request_turn_id;
pub use approval_request::guardian_reviewed_action;
use codex_extension_api::AgentSpawnFuture;
use codex_extension_api::AgentSpawner;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::ThreadLifecycleContributor;
use codex_extension_api::ThreadStartInput;
use codex_protocol::ThreadId;
pub use prompt::GuardianAssessment;
pub use prompt::GuardianPromptItems;
pub use prompt::GuardianPromptMode;
pub use prompt::GuardianTranscriptCursor;
pub use prompt::GuardianTranscriptEntry;
pub use prompt::GuardianTranscriptEntryKind;
pub use prompt::build_guardian_prompt_items_from_entries;
pub use prompt::collect_guardian_transcript_entries;
pub use prompt::guardian_output_schema;
pub use prompt::guardian_policy_prompt;
pub use prompt::guardian_policy_prompt_with_config;
pub use prompt::parse_guardian_assessment;
pub use prompt::render_guardian_transcript_entries;

const GUARDIAN_MAX_ACTION_STRING_TOKENS: usize = 16_000;
const TRUNCATION_TAG: &str = "truncated";
pub const MAX_CONSECUTIVE_GUARDIAN_DENIALS_PER_TURN: u32 = 3;
pub const MAX_RECENT_AUTO_REVIEW_DENIALS_PER_TURN: u32 = 10;
pub const AUTO_REVIEW_DENIAL_WINDOW_SIZE: usize = 50;
pub const AUTO_REVIEW_DENIED_ACTION_APPROVAL_DEVELOPER_PREFIX: &str =
    "The user has manually approved a specific action that was previously `Rejected`.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardianRejection {
    pub rationale: String,
    pub source: codex_protocol::protocol::GuardianAssessmentDecisionSource,
}

#[derive(Debug, Default)]
pub struct GuardianRejectionCircuitBreaker {
    turns: std::collections::HashMap<String, GuardianRejectionCircuitBreakerTurn>,
}

#[derive(Debug, Default)]
struct GuardianRejectionCircuitBreakerTurn {
    consecutive_denials: u32,
    recent_denials: std::collections::VecDeque<bool>,
    interrupt_triggered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardianRejectionCircuitBreakerAction {
    Continue,
    InterruptTurn {
        consecutive_denials: u32,
        recent_denials: u32,
    },
}

impl GuardianRejectionCircuitBreaker {
    pub fn clear_turn(&mut self, turn_id: &str) {
        self.turns.remove(turn_id);
    }

    pub fn record_denial(&mut self, turn_id: &str) -> GuardianRejectionCircuitBreakerAction {
        let turn = self.turns.entry(turn_id.to_string()).or_default();
        turn.consecutive_denials = turn.consecutive_denials.saturating_add(1);
        Self::record_recent_review(turn, /*denied*/ true);
        let recent_denials = turn.recent_denials.iter().filter(|denied| **denied).count() as u32;
        if !turn.interrupt_triggered
            && (turn.consecutive_denials >= MAX_CONSECUTIVE_GUARDIAN_DENIALS_PER_TURN
                || recent_denials >= MAX_RECENT_AUTO_REVIEW_DENIALS_PER_TURN)
        {
            turn.interrupt_triggered = true;
            GuardianRejectionCircuitBreakerAction::InterruptTurn {
                consecutive_denials: turn.consecutive_denials,
                recent_denials,
            }
        } else {
            GuardianRejectionCircuitBreakerAction::Continue
        }
    }

    pub fn record_non_denial(&mut self, turn_id: &str) {
        let turn = self.turns.entry(turn_id.to_string()).or_default();
        turn.consecutive_denials = 0;
        Self::record_recent_review(turn, /*denied*/ false);
    }

    fn record_recent_review(turn: &mut GuardianRejectionCircuitBreakerTurn, denied: bool) {
        turn.recent_denials.push_back(denied);
        if turn.recent_denials.len() > AUTO_REVIEW_DENIAL_WINDOW_SIZE {
            turn.recent_denials.pop_front();
        }
    }
}

fn guardian_truncate_text(content: &str, token_cap: usize) -> (String, bool) {
    if content.is_empty() {
        return (String::new(), false);
    }

    let max_bytes = codex_utils_output_truncation::approx_bytes_for_tokens(token_cap);
    if content.len() <= max_bytes {
        return (content.to_string(), false);
    }

    let omitted_tokens = codex_utils_output_truncation::approx_tokens_from_byte_count(
        content.len().saturating_sub(max_bytes),
    );
    let marker = format!("<{TRUNCATION_TAG} omitted_approx_tokens=\"{omitted_tokens}\" />");
    if max_bytes <= marker.len() {
        return (marker, true);
    }

    let available_bytes = max_bytes.saturating_sub(marker.len());
    let prefix_budget = available_bytes / 2;
    let suffix_budget = available_bytes.saturating_sub(prefix_budget);
    let (prefix, suffix) = split_guardian_truncation_bounds(content, prefix_budget, suffix_budget);

    (format!("{prefix}{marker}{suffix}"), true)
}

fn split_guardian_truncation_bounds(
    content: &str,
    prefix_bytes: usize,
    suffix_bytes: usize,
) -> (&str, &str) {
    if content.is_empty() {
        return ("", "");
    }

    let len = content.len();
    let suffix_start_target = len.saturating_sub(suffix_bytes);
    let mut prefix_end = 0usize;
    let mut suffix_start = len;
    let mut suffix_started = false;

    for (index, ch) in content.char_indices() {
        let char_end = index + ch.len_utf8();
        if char_end <= prefix_bytes {
            prefix_end = char_end;
            continue;
        }

        if index >= suffix_start_target {
            if !suffix_started {
                suffix_start = index;
                suffix_started = true;
            }
            continue;
        }
    }

    if suffix_start < prefix_end {
        suffix_start = prefix_end;
    }

    (&content[..prefix_end], &content[suffix_start..])
}

/// Guardian extension dependencies supplied by the host at construction time.
#[derive(Clone, Debug)]
pub struct GuardianExtension<S> {
    agent_spawner: S,
}

impl<S> GuardianExtension<S> {
    /// Creates a guardian extension with its host-provided agent spawn helper.
    pub fn new(agent_spawner: S) -> Self {
        Self { agent_spawner }
    }

    /// Delegates one guardian-owned subagent spawn request to the host helper.
    pub fn spawn_subagent<'a, R>(
        &'a self,
        forked_from_thread_id: ThreadId,
        request: R,
    ) -> AgentSpawnFuture<'a, <S as AgentSpawner<R>>::Spawned, <S as AgentSpawner<R>>::Error>
    where
        S: AgentSpawner<R>,
    {
        self.agent_spawner
            .spawn_subagent(forked_from_thread_id, request)
    }
}

/// Thread-local guardian state captured when the host starts a thread.
#[derive(Clone, Copy, Debug)]
pub struct GuardianThreadContext {
    forked_from_thread_id: ThreadId,
}

impl GuardianThreadContext {
    /// Returns the thread that future guardian subagents should fork from by default.
    pub fn forked_from_thread_id(&self) -> ThreadId {
        self.forked_from_thread_id
    }
}

impl<C, S> ThreadLifecycleContributor<C> for GuardianExtension<S>
where
    S: Send + Sync,
{
    fn on_thread_start(&self, input: ThreadStartInput<'_, C>) {
        let Ok(forked_from_thread_id) = ThreadId::from_string(input.thread_store.level_id()) else {
            return;
        };
        input.thread_store.insert(GuardianThreadContext {
            forked_from_thread_id,
        });
    }
}

/// Installs the guardian contributors into the extension registry.
pub fn install<C, S>(registry: &mut ExtensionRegistryBuilder<C>, agent_spawner: S)
where
    S: Send + Sync + 'static,
{
    registry.thread_lifecycle_contributor(Arc::new(GuardianExtension::new(agent_spawner)));
}

#[cfg(test)]
mod tests {
    use codex_protocol::approvals::NetworkApprovalProtocol;
    use codex_protocol::models::SandboxPermissions;
    use codex_utils_absolute_path::AbsolutePathBuf;

    use super::AUTO_REVIEW_DENIAL_WINDOW_SIZE;
    use super::GuardianApprovalRequest;
    use super::GuardianMcpAnnotations;
    use super::GuardianNetworkAccessTrigger;
    use super::GuardianRejectionCircuitBreaker;
    use super::GuardianRejectionCircuitBreakerAction;
    use super::format_guardian_action_pretty;
    use super::guardian_approval_request_to_json;

    fn test_path_buf(path: &str) -> AbsolutePathBuf {
        AbsolutePathBuf::try_from(std::path::PathBuf::from(path)).unwrap()
    }

    #[test]
    fn guardian_rejection_circuit_breaker_interrupts_after_three_consecutive_denials() {
        let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::Continue
        );
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::Continue
        );
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::InterruptTurn {
                consecutive_denials: 3,
                recent_denials: 3,
            }
        );
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::Continue
        );
    }

    #[test]
    fn guardian_rejection_circuit_breaker_resets_consecutive_denials_on_non_denial() {
        let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::Continue
        );
        circuit_breaker.record_non_denial("turn-1");
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::Continue
        );
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::Continue
        );
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::InterruptTurn {
                consecutive_denials: 3,
                recent_denials: 4,
            }
        );
    }

    #[test]
    fn auto_review_rejection_circuit_breaker_interrupts_after_ten_recent_denials() {
        let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
        for _ in 0..9 {
            assert_eq!(
                circuit_breaker.record_denial("turn-1"),
                GuardianRejectionCircuitBreakerAction::Continue
            );
            circuit_breaker.record_non_denial("turn-1");
        }
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::InterruptTurn {
                consecutive_denials: 1,
                recent_denials: 10,
            }
        );
    }

    #[test]
    fn auto_review_rejection_circuit_breaker_forgets_denials_outside_recent_review_window() {
        let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
        for _ in 0..9 {
            assert_eq!(
                circuit_breaker.record_denial("turn-1"),
                GuardianRejectionCircuitBreakerAction::Continue
            );
            circuit_breaker.record_non_denial("turn-1");
        }
        for _ in 0..(AUTO_REVIEW_DENIAL_WINDOW_SIZE - 18) {
            circuit_breaker.record_non_denial("turn-1");
        }
        assert_eq!(
            circuit_breaker.record_denial("turn-1"),
            GuardianRejectionCircuitBreakerAction::Continue
        );
    }

    #[test]
    fn format_guardian_action_pretty_truncates_large_string_fields() -> serde_json::Result<()> {
        let patch = "line\n".repeat(100_000);
        let action = GuardianApprovalRequest::ApplyPatch {
            id: "patch-1".to_string(),
            cwd: test_path_buf("/tmp"),
            files: Vec::new(),
            patch: patch.clone(),
        };

        let rendered = format_guardian_action_pretty(&action)?;

        assert!(rendered.text.contains("\"tool\": \"apply_patch\""));
        assert!(rendered.text.contains("<truncated omitted_approx_tokens="));
        assert!(rendered.text.len() < patch.len());
        assert!(rendered.truncated);
        Ok(())
    }

    #[test]
    fn guardian_approval_request_to_json_renders_mcp_tool_call_shape() -> serde_json::Result<()> {
        let action = GuardianApprovalRequest::McpToolCall {
            id: "call-1".to_string(),
            server: "mcp_server".to_string(),
            tool_name: "browser_navigate".to_string(),
            arguments: Some(serde_json::json!({
                "url": "https://example.com",
            })),
            connector_id: None,
            connector_name: Some("Playwright".to_string()),
            connector_description: None,
            tool_title: Some("Navigate".to_string()),
            tool_description: None,
            annotations: Some(GuardianMcpAnnotations {
                destructive_hint: Some(true),
                open_world_hint: None,
                read_only_hint: Some(false),
            }),
        };

        assert_eq!(
            guardian_approval_request_to_json(&action)?,
            serde_json::json!({
                "tool": "mcp_tool_call",
                "server": "mcp_server",
                "tool_name": "browser_navigate",
                "arguments": {
                    "url": "https://example.com",
                },
                "connector_name": "Playwright",
                "tool_title": "Navigate",
                "annotations": {
                    "destructive_hint": true,
                    "read_only_hint": false,
                },
            })
        );
        Ok(())
    }

    #[test]
    fn guardian_approval_request_to_json_renders_network_access_trigger() -> serde_json::Result<()>
    {
        let cwd = test_path_buf("/repo");
        let action = GuardianApprovalRequest::NetworkAccess {
            id: "network-1".to_string(),
            turn_id: "turn-1".to_string(),
            target: "https://example.com:443".to_string(),
            host: "example.com".to_string(),
            protocol: NetworkApprovalProtocol::Https,
            port: 443,
            trigger: Some(GuardianNetworkAccessTrigger {
                call_id: "call-1".to_string(),
                tool_name: "shell".to_string(),
                command: vec!["curl".to_string(), "https://example.com".to_string()],
                cwd: cwd.clone(),
                sandbox_permissions: SandboxPermissions::UseDefault,
                additional_permissions: None,
                justification: Some("Fetch the release metadata.".to_string()),
                tty: None,
            }),
        };

        assert_eq!(
            guardian_approval_request_to_json(&action)?,
            serde_json::json!({
                "tool": "network_access",
                "target": "https://example.com:443",
                "host": "example.com",
                "protocol": "https",
                "port": 443,
                "trigger": {
                    "callId": "call-1",
                    "toolName": "shell",
                    "command": ["curl", "https://example.com"],
                    "cwd": cwd.to_string_lossy().to_string(),
                    "sandboxPermissions": "use_default",
                    "justification": "Fetch the release metadata.",
                },
            })
        );
        Ok(())
    }
}
