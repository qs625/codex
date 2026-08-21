#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabAgentInteractionEndEvent {
    /// Identifier for the collab tool call.
    pub call_id: String,
    #[serde(default)]
    pub completed_at_ms: i64,
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// Thread ID of the receiver.
    pub receiver_thread_id: ThreadId,
    /// Canonical path of the receiver.
    pub receiver_agent_path: String,
    /// Optional nickname assigned to the receiver agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_agent_nickname: Option<String>,
    /// Optional role assigned to the receiver agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_agent_role: Option<String>,
    /// Prompt sent from the sender to the receiver. Can be empty to prevent CoT
    /// leaking at the beginning.
    pub prompt: String,
    /// Last known status of the receiver agent reported to the sender agent.
    pub status: AgentStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabListAgentsBeginEvent {
    /// Identifier for the collab tool call.
    pub call_id: String,
    #[serde(default)]
    pub started_at_ms: i64,
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// Optional path prefix filter passed to list_agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
pub struct CollabListedAgent {
    /// Canonical path of the listed agent.
    pub agent_path: String,
    /// Optional nickname assigned to the listed agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_nickname: Option<String>,
    /// Optional role assigned to the listed agent.
    #[serde(default, alias = "agent_type", skip_serializing_if = "Option::is_none")]
    pub agent_role: Option<String>,
    /// Last known lifecycle of the listed agent.
    pub lifecycle_status: ThreadLifecycleStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabListAgentsEndEvent {
    /// Identifier for the collab tool call.
    pub call_id: String,
    #[serde(default)]
    pub completed_at_ms: i64,
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// Optional path prefix filter passed to list_agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    pub success: bool,
    pub agents: Vec<CollabListedAgent>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabWaitingBeginEvent {
    #[serde(default)]
    pub started_at_ms: i64,
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// Thread ID of the receivers.
    pub receiver_thread_ids: Vec<ThreadId>,
    /// Optional nicknames/roles for receivers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receiver_agents: Vec<CollabAgentRef>,
    /// Timeout requested for the wait call, in milliseconds.
    pub timeout_ms: i64,
    /// ID of the waiting call.
    pub call_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabWaitingEndEvent {
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// ID of the waiting call.
    pub call_id: String,
    #[serde(default)]
    pub completed_at_ms: i64,
    /// Timeout requested for the wait call, in milliseconds.
    pub timeout_ms: i64,
    /// Optional receiver metadata paired with final lifecycle statuses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_lifecycles: Vec<CollabAgentLifecycleEntry>,
    /// Last known lifecycle statuses of the receiver agents reported to the sender agent.
    pub lifecycle_statuses: HashMap<ThreadId, ThreadLifecycleStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabCloseBeginEvent {
    /// Identifier for the collab tool call.
    pub call_id: String,
    #[serde(default)]
    pub started_at_ms: i64,
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// Thread ID of the receiver.
    pub receiver_thread_id: ThreadId,
    /// Canonical path of the receiver.
    pub receiver_agent_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabCloseEndEvent {
    /// Identifier for the collab tool call.
    pub call_id: String,
    #[serde(default)]
    pub completed_at_ms: i64,
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// Thread ID of the receiver.
    pub receiver_thread_id: ThreadId,
    /// Canonical path of the receiver.
    pub receiver_agent_path: String,
    /// Optional nickname assigned to the receiver agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_agent_nickname: Option<String>,
    /// Optional role assigned to the receiver agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_agent_role: Option<String>,
    /// Last known status of the receiver agent reported to the sender agent before
    /// the close.
    pub status: AgentStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabResumeBeginEvent {
    /// Identifier for the collab tool call.
    pub call_id: String,
    #[serde(default)]
    pub started_at_ms: i64,
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// Thread ID of the receiver.
    pub receiver_thread_id: ThreadId,
    /// Canonical path of the receiver.
    pub receiver_agent_path: String,
    /// Optional nickname assigned to the receiver agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_agent_nickname: Option<String>,
    /// Optional role assigned to the receiver agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_agent_role: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema, TS)]
pub struct CollabResumeEndEvent {
    /// Identifier for the collab tool call.
    pub call_id: String,
    #[serde(default)]
    pub completed_at_ms: i64,
    /// Thread ID of the sender.
    pub sender_thread_id: ThreadId,
    /// Canonical path of the sender.
    pub sender_agent_path: String,
    /// Thread ID of the receiver.
    pub receiver_thread_id: ThreadId,
    /// Canonical path of the receiver.
    pub receiver_agent_path: String,
    /// Optional nickname assigned to the receiver agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_agent_nickname: Option<String>,
    /// Optional role assigned to the receiver agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_agent_role: Option<String>,
    /// Last known status of the receiver agent reported to the sender agent after
    /// resume.
    pub status: AgentStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::FileChangeItem;
    use crate::items::ImageGenerationItem;
    use crate::items::McpToolCallItem;
    use crate::items::McpToolCallStatus;
    use crate::items::UserMessageItem;
    use crate::items::WebSearchItem;
    use crate::mcp::CallToolResult;
    use crate::permissions::FileSystemAccessMode;
    use crate::permissions::FileSystemPath;
    use crate::permissions::FileSystemSandboxEntry;
    use crate::permissions::FileSystemSandboxPolicy;
    use crate::permissions::FileSystemSpecialPath;
    use crate::permissions::NetworkSandboxPolicy;
    use anyhow::Result;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use codex_utils_absolute_path::test_support::test_path_buf;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;
    use tempfile::TempDir;

    fn sorted_writable_roots(roots: Vec<WritableRoot>) -> Vec<(PathBuf, Vec<PathBuf>)> {
        let mut sorted_roots: Vec<(PathBuf, Vec<PathBuf>)> = roots
            .into_iter()
            .map(|root| {
                let mut read_only_subpaths: Vec<PathBuf> = root
                    .read_only_subpaths
                    .into_iter()
                    .map(|path| path.to_path_buf())
                    .collect();
                read_only_subpaths.sort();
                (root.root.to_path_buf(), read_only_subpaths)
            })
            .collect();
        sorted_roots.sort_by(|left, right| left.0.cmp(&right.0));
        sorted_roots
    }

    fn sandbox_policy_allows_read(policy: &SandboxPolicy, _path: &Path, _cwd: &Path) -> bool {
        policy.has_full_disk_read_access()
    }

    fn sandbox_policy_allows_write(policy: &SandboxPolicy, path: &Path, cwd: &Path) -> bool {
        if policy.has_full_disk_write_access() {
            return true;
        }

        policy
            .get_writable_roots_with_cwd(cwd)
            .iter()
            .any(|root| root.is_path_writable(path))
    }

    #[test]
    fn session_source_from_startup_arg_maps_known_values() {
        assert_eq!(
            SessionSource::from_startup_arg("vscode").unwrap(),
            SessionSource::VSCode
        );
        assert_eq!(
            SessionSource::from_startup_arg("app-server").unwrap(),
            SessionSource::Mcp
        );
    }

    #[test]
    fn inter_agent_communication_response_input_item_uses_non_json_commentary_context() {
        let communication = InterAgentCommunication {
            author: AgentPath::root(),
            recipient: AgentPath::root().join("reviewer").expect("recipient path"),
            other_recipients: vec![AgentPath::root().join("worker").expect("recipient path")],
            content: "review the diff".to_string(),
            content_parts: Vec::new(),
            operation: InterAgentOperation::Unknown,
            trigger_turn: true,
            sender_thread_id: None,
            recipient_thread_id: None,
            status: None,
            lifecycle_status: None,
            agent_nickname: None,
            agent_role: None,
        };

        let ResponseInputItem::Message {
            role,
            content,
            phase,
        } = communication.to_response_input_item()
        else {
            panic!("inter-agent context should format as a model message");
        };

        assert_eq!(role, "assistant");
        assert_eq!(phase, Some(MessagePhase::Commentary));
        let [ContentItem::OutputText { text }] = content.as_slice() else {
            panic!("inter-agent context should be a single text item");
        };
        assert!(text.contains("Inter-agent communication received."));
        assert!(text.contains("Author: /"));
        assert!(text.contains("Recipient: /root/reviewer"));
        assert!(text.contains("Operation: unknown"));
        assert!(text.contains("Content:\nreview the diff"));
        assert!(!text.trim_start().starts_with('{'));
        assert!(!text.contains("\"author\""));
    }

    #[test]
    fn inter_agent_communication_structured_image_ref_becomes_user_image_input() {
        let image_url = "data:image/png;base64,abc".to_string();
        let communication = InterAgentCommunication::new(
            AgentPath::root(),
            AgentPath::root().join("reviewer").expect("recipient path"),
            Vec::new(),
            "[image:img-1]".to_string(),
            InterAgentOperation::FollowupTask,
        )
        .with_content_parts(vec![
            InterAgentContentPart::Text {
                text: "Please inspect this UI.".to_string(),
            },
            InterAgentContentPart::ImageRef {
                attachment_id: "img-1".to_string(),
                image_url: Some(image_url.clone()),
            },
        ]);

        assert_eq!(
            communication.to_response_input_item(),
            ResponseInputItem::Message {
                role: "user".to_string(),
                content: vec![
                    ContentItem::InputText {
                        text: "Please inspect this UI.".to_string(),
                    },
                    ContentItem::InputText {
                        text: "Image attachment_id=img-1 begins".to_string(),
                    },
                    ContentItem::InputText {
                        text: crate::models::image_open_tag_text_with_attachment_id("img-1"),
                    },
                    ContentItem::InputImage {
                        image_url,
                        detail: Some(crate::models::DEFAULT_IMAGE_DETAIL),
                    },
                    ContentItem::InputText {
                        text: crate::models::image_close_tag_text(),
                    },
                    ContentItem::InputText {
                        text: "Image attachment_id=img-1 ends".to_string(),
                    },
                ],
                phase: None,
            }
        );
    }

    #[test]
    fn session_source_from_startup_arg_normalizes_custom_values() {
        assert_eq!(
            SessionSource::from_startup_arg("atlas").unwrap(),
            SessionSource::Custom("atlas".to_string())
        );
        assert_eq!(
            SessionSource::from_startup_arg(" Atlas ").unwrap(),
            SessionSource::Custom("atlas".to_string())
        );
    }

    #[test]
    fn session_source_restriction_product_defaults_non_subagent_sources_to_codex() {
        assert_eq!(
            SessionSource::Cli.restriction_product(),
            Some(Product::Codex)
        );
        assert_eq!(
            SessionSource::VSCode.restriction_product(),
            Some(Product::Codex)
        );
        assert_eq!(
            SessionSource::Exec.restriction_product(),
            Some(Product::Codex)
        );
        assert_eq!(
            SessionSource::Mcp.restriction_product(),
            Some(Product::Codex)
        );
        assert_eq!(
            SessionSource::Unknown.restriction_product(),
            Some(Product::Codex)
        );
    }

    #[test]
    fn session_source_restriction_product_does_not_guess_subagent_products() {
        assert_eq!(
            SessionSource::SubAgent(SubAgentSource::Review).restriction_product(),
            None
        );
        assert_eq!(
            SessionSource::Internal(InternalSessionSource::MemoryConsolidation)
                .restriction_product(),
            None
        );
    }

    #[test]
    fn session_source_restriction_product_maps_custom_sources_to_products() {
        assert_eq!(
            SessionSource::Custom("chatgpt".to_string()).restriction_product(),
            Some(Product::Chatgpt)
        );
        assert_eq!(
            SessionSource::Custom("ATLAS".to_string()).restriction_product(),
            Some(Product::Atlas)
        );
        assert_eq!(
            SessionSource::Custom("codex".to_string()).restriction_product(),
            Some(Product::Codex)
        );
        assert_eq!(
            SessionSource::Custom("atlas-dev".to_string()).restriction_product(),
            None
        );
    }

    #[test]
    fn session_source_matches_product_restriction() {
        assert!(
            SessionSource::Custom("chatgpt".to_string())
                .matches_product_restriction(&[Product::Chatgpt])
        );
        assert!(
            !SessionSource::Custom("chatgpt".to_string())
                .matches_product_restriction(&[Product::Codex])
        );
        assert!(SessionSource::VSCode.matches_product_restriction(&[Product::Codex]));
        assert!(
            !SessionSource::Custom("atlas-dev".to_string())
                .matches_product_restriction(&[Product::Atlas])
        );
        assert!(SessionSource::Custom("atlas-dev".to_string()).matches_product_restriction(&[]));
    }

    fn sandbox_policy_probe_paths(policy: &SandboxPolicy, cwd: &Path) -> Vec<PathBuf> {
        let mut paths = vec![cwd.to_path_buf()];
        for root in policy.get_writable_roots_with_cwd(cwd) {
            paths.push(root.root.to_path_buf());
            paths.extend(
                root.read_only_subpaths
                    .into_iter()
                    .map(|path| path.to_path_buf()),
            );
        }
        paths.sort();
        paths.dedup();
        paths
    }

    fn assert_same_sandbox_policy_semantics(
        expected: &SandboxPolicy,
        actual: &SandboxPolicy,
        cwd: &Path,
    ) {
        assert_eq!(
            actual.has_full_disk_read_access(),
            expected.has_full_disk_read_access()
        );
        assert_eq!(
            actual.has_full_disk_write_access(),
            expected.has_full_disk_write_access()
        );
        assert_eq!(
            actual.has_full_network_access(),
            expected.has_full_network_access()
        );
        let mut probe_paths = sandbox_policy_probe_paths(expected, cwd);
        probe_paths.extend(sandbox_policy_probe_paths(actual, cwd));
        probe_paths.sort();
        probe_paths.dedup();

        for path in probe_paths {
            assert_eq!(
                sandbox_policy_allows_read(actual, &path, cwd),
                sandbox_policy_allows_read(expected, &path, cwd),
                "read access mismatch for {}",
                path.display()
            );
            assert_eq!(
                sandbox_policy_allows_write(actual, &path, cwd),
                sandbox_policy_allows_write(expected, &path, cwd),
                "write access mismatch for {}",
                path.display()
            );
        }
    }

    #[test]
    fn external_sandbox_reports_full_access_flags() {
        let restricted = SandboxPolicy::ExternalSandbox {
            network_access: NetworkAccess::Restricted,
        };
        assert!(restricted.has_full_disk_write_access());
        assert!(!restricted.has_full_network_access());

        let enabled = SandboxPolicy::ExternalSandbox {
            network_access: NetworkAccess::Enabled,
        };
        assert!(enabled.has_full_disk_write_access());
        assert!(enabled.has_full_network_access());
    }

    #[test]
    fn read_only_reports_network_access_flags() {
        let restricted = SandboxPolicy::new_read_only_policy();
        assert!(!restricted.has_full_network_access());

        let enabled = SandboxPolicy::ReadOnly {
            network_access: true,
        };
        assert!(enabled.has_full_network_access());
    }

    #[test]
    fn granular_approval_config_mcp_elicitation_flag_is_field_driven() {
        assert!(
            GranularApprovalConfig {
                sandbox_approval: false,
                rules: false,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: true,
            }
            .allows_mcp_elicitations()
        );
        assert!(
            !GranularApprovalConfig {
                sandbox_approval: false,
                rules: false,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: false,
            }
            .allows_mcp_elicitations()
        );
    }

    #[test]
    fn granular_approval_config_skill_approval_flag_is_field_driven() {
        assert!(
            GranularApprovalConfig {
                sandbox_approval: false,
                rules: false,
                skill_approval: true,
                request_permissions: false,
                mcp_elicitations: false,
            }
            .allows_skill_approval()
        );
        assert!(
            !GranularApprovalConfig {
                sandbox_approval: false,
                rules: false,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: false,
            }
            .allows_skill_approval()
        );
    }

    #[test]
    fn granular_approval_config_request_permissions_flag_is_field_driven() {
        assert!(
            GranularApprovalConfig {
                sandbox_approval: false,
                rules: false,
                skill_approval: false,
                request_permissions: true,
                mcp_elicitations: false,
            }
            .allows_request_permissions()
        );
        assert!(
            !GranularApprovalConfig {
                sandbox_approval: false,
                rules: false,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: false,
            }
            .allows_request_permissions()
        );
    }

    #[test]
    fn granular_approval_config_defaults_missing_optional_flags_to_false() {
        let decoded = serde_json::from_value::<GranularApprovalConfig>(serde_json::json!({
            "sandbox_approval": true,
            "rules": false,
            "mcp_elicitations": true,
        }))
        .expect("granular approval config should deserialize");

        assert_eq!(
            decoded,
            GranularApprovalConfig {
                sandbox_approval: true,
                rules: false,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: true,
            }
        );
    }

    #[test]
    fn restricted_file_system_policy_reports_full_access_from_root_entries() {
        let read_only = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        }]);
        assert!(read_only.has_full_disk_read_access());
        assert!(!read_only.has_full_disk_write_access());
        assert!(!read_only.include_platform_defaults());

        let writable = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Write,
        }]);
        assert!(writable.has_full_disk_read_access());
        assert!(writable.has_full_disk_write_access());
    }

    #[test]
    fn restricted_file_system_policy_treats_root_with_carveouts_as_scoped_access() {
        let cwd = TempDir::new().expect("tempdir");
        let canonical_cwd = codex_utils_absolute_path::canonicalize_preserving_symlinks(cwd.path())
            .expect("canonicalize cwd");
        let root = AbsolutePathBuf::from_absolute_path(&canonical_cwd)
            .expect("absolute canonical tempdir")
            .as_path()
            .ancestors()
            .last()
            .and_then(|path| AbsolutePathBuf::from_absolute_path(path).ok())
            .expect("filesystem root");
        let blocked = AbsolutePathBuf::resolve_path_against_base("blocked", cwd.path());
        let expected_blocked = AbsolutePathBuf::from_absolute_path(
            codex_utils_absolute_path::canonicalize_preserving_symlinks(cwd.path())
                .expect("canonicalize cwd")
                .join("blocked"),
        )
        .expect("canonical blocked");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: blocked },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert!(!policy.has_full_disk_read_access());
        assert!(!policy.has_full_disk_write_access());
        assert_eq!(
            policy.get_readable_roots_with_cwd(cwd.path()),
            vec![root.clone()]
        );
        assert_eq!(
            policy.get_unreadable_roots_with_cwd(cwd.path()),
            vec![expected_blocked.clone()]
        );

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, root);
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .iter()
                .any(|path| path.as_path() == expected_blocked.as_path())
        );
    }

    #[test]
    fn restricted_file_system_policy_derives_effective_paths() {
        let cwd = TempDir::new().expect("tempdir");
        std::fs::create_dir_all(cwd.path().join(".agents")).expect("create .agents");
        std::fs::create_dir_all(cwd.path().join(".codex")).expect("create .codex");
        let canonical_cwd = codex_utils_absolute_path::canonicalize_preserving_symlinks(cwd.path())
            .expect("canonicalize cwd");
        let cwd_absolute =
            AbsolutePathBuf::from_absolute_path(&canonical_cwd).expect("absolute tempdir");
        let secret = AbsolutePathBuf::resolve_path_against_base("secret", cwd.path());
        let expected_secret = AbsolutePathBuf::from_absolute_path(canonical_cwd.join("secret"))
            .expect("canonical secret");
        let expected_agents = AbsolutePathBuf::from_absolute_path(canonical_cwd.join(".agents"))
            .expect("canonical .agents");
        let expected_codex = AbsolutePathBuf::from_absolute_path(canonical_cwd.join(".codex"))
            .expect("canonical .codex");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Minimal,
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: secret },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert!(!policy.has_full_disk_read_access());
        assert!(!policy.has_full_disk_write_access());
        assert!(policy.include_platform_defaults());
        assert_eq!(
            policy.get_readable_roots_with_cwd(cwd.path()),
            vec![cwd_absolute.clone()]
        );
        assert_eq!(
            policy.get_unreadable_roots_with_cwd(cwd.path()),
            vec![expected_secret.clone()]
        );

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, cwd_absolute);
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .iter()
                .any(|path| path.as_path() == expected_secret.as_path())
        );
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .iter()
                .any(|path| path.as_path() == expected_agents.as_path())
        );
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .iter()
                .any(|path| path.as_path() == expected_codex.as_path())
        );
    }

    #[test]
    fn restricted_file_system_policy_treats_read_entries_as_read_only_subpaths() {
        let cwd = TempDir::new().expect("tempdir");
        let canonical_cwd = codex_utils_absolute_path::canonicalize_preserving_symlinks(cwd.path())
            .expect("canonicalize cwd");
        let docs = AbsolutePathBuf::resolve_path_against_base("docs", cwd.path());
        let docs_public = AbsolutePathBuf::resolve_path_against_base("docs/public", cwd.path());
        let expected_docs = AbsolutePathBuf::from_absolute_path(canonical_cwd.join("docs"))
            .expect("canonical docs");
        let expected_docs_public =
            AbsolutePathBuf::from_absolute_path(canonical_cwd.join("docs/public"))
                .expect("canonical docs/public");
        let expected_dot_codex = AbsolutePathBuf::from_absolute_path(canonical_cwd.join(".codex"))
            .expect("canonical .codex");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs_public },
                access: FileSystemAccessMode::Write,
            },
        ]);

        assert!(!policy.has_full_disk_write_access());
        assert_eq!(
            sorted_writable_roots(policy.get_writable_roots_with_cwd(cwd.path())),
            vec![
                (
                    canonical_cwd,
                    vec![
                        expected_dot_codex.to_path_buf(),
                        expected_docs.to_path_buf()
                    ],
                ),
                (expected_docs_public.to_path_buf(), Vec::new()),
            ]
        );
    }

    #[test]
    fn file_system_policy_rejects_legacy_bridge_for_non_workspace_writes() {
        let cwd = if cfg!(windows) {
            Path::new(r"C:\workspace")
        } else {
            Path::new("/tmp/workspace")
        };
        let external_write_path = if cfg!(windows) {
            AbsolutePathBuf::from_absolute_path(r"C:\temp").expect("absolute windows temp path")
        } else {
            AbsolutePathBuf::from_absolute_path("/tmp").expect("absolute tmp path")
        };
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: external_write_path,
            },
            access: FileSystemAccessMode::Write,
        }]);

        let err = policy
            .to_legacy_sandbox_policy(NetworkSandboxPolicy::Restricted, cwd)
            .expect_err("non-workspace writes should be rejected");

        assert!(
            err.to_string()
                .contains("filesystem writes outside the workspace root"),
            "{err}"
        );
    }

    #[test]
    fn legacy_sandbox_policy_semantics_survive_split_bridge() {
        let cwd = TempDir::new().expect("tempdir");
        let writable_root = AbsolutePathBuf::resolve_path_against_base("writable", cwd.path());
        let policies = [
            SandboxPolicy::DangerFullAccess,
            SandboxPolicy::ExternalSandbox {
                network_access: NetworkAccess::Restricted,
            },
            SandboxPolicy::ExternalSandbox {
                network_access: NetworkAccess::Enabled,
            },
            SandboxPolicy::ReadOnly {
                network_access: false,
            },
            SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![],
                network_access: false,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: true,
            },
            SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![writable_root],
                network_access: true,
                exclude_tmpdir_env_var: false,
                exclude_slash_tmp: true,
            },
        ];

        for expected in policies {
            let actual =
                FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(&expected, cwd.path())
                    .to_legacy_sandbox_policy(NetworkSandboxPolicy::from(&expected), cwd.path())
                    .expect("legacy bridge should preserve legacy policy semantics");

            assert_same_sandbox_policy_semantics(&expected, &actual, cwd.path());
        }
    }

    #[test]
    fn item_started_event_from_web_search_emits_begin_event() {
        let event = ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            item: TurnItem::WebSearch(WebSearchItem {
                id: "search-1".into(),
                query: "find docs".into(),
                action: WebSearchAction::Search {
                    query: Some("find docs".into()),
                    queries: None,
                },
            }),
            started_at_ms: 0,
        };

        let legacy_events = event.as_legacy_events(/*show_raw_agent_reasoning*/ false);
        assert_eq!(legacy_events.len(), 1);
        match &legacy_events[0] {
            EventMsg::WebSearchBegin(event) => assert_eq!(event.call_id, "search-1"),
            _ => panic!("expected WebSearchBegin event"),
        }
    }

    #[test]
    fn item_started_event_from_non_web_search_emits_no_legacy_events() {
        let event = ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            item: TurnItem::UserMessage(UserMessageItem::new(&[])),
            started_at_ms: 0,
        };

        assert!(
            event
                .as_legacy_events(/*show_raw_agent_reasoning*/ false)
                .is_empty()
        );
    }

    #[test]
    fn item_started_event_from_image_generation_emits_begin_event() {
        let event = ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            item: TurnItem::ImageGeneration(ImageGenerationItem {
                id: "ig-1".into(),
                status: "in_progress".into(),
                revised_prompt: None,
                result: String::new(),
                saved_path: None,
            }),
            started_at_ms: 0,
        };

        let legacy_events = event.as_legacy_events(/*show_raw_agent_reasoning*/ false);
        assert_eq!(legacy_events.len(), 1);
        match &legacy_events[0] {
            EventMsg::ImageGenerationBegin(event) => assert_eq!(event.call_id, "ig-1"),
            _ => panic!("expected ImageGenerationBegin event"),
        }
    }

    #[test]
    fn item_started_event_from_file_change_emits_patch_begin_event() {
        let event = ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            started_at_ms: 0,
            item: TurnItem::FileChange(FileChangeItem {
                id: "patch-1".into(),
                changes: [(
                    PathBuf::from("new.txt"),
                    FileChange::Add {
                        content: "hello".into(),
                    },
                )]
                .into_iter()
                .collect(),
                status: None,
                auto_approved: Some(true),
                stdout: None,
                stderr: None,
            }),
        };

        let legacy_events = event.as_legacy_events(/*show_raw_agent_reasoning*/ false);
        assert_eq!(legacy_events.len(), 1);
        match &legacy_events[0] {
            EventMsg::PatchApplyBegin(event) => {
                assert_eq!(event.call_id, "patch-1");
                assert_eq!(event.turn_id, "turn-1");
                assert!(event.auto_approved);
                assert!(event.changes.contains_key(&PathBuf::from("new.txt")));
            }
            _ => panic!("expected PatchApplyBegin event"),
        }
    }

    #[test]
    fn item_started_event_from_mcp_tool_call_emits_begin_event() {
        let event = ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            started_at_ms: 0,
            item: TurnItem::McpToolCall(McpToolCallItem {
                id: "mcp-1".into(),
                server: "server".into(),
                tool: "tool".into(),
                arguments: json!({"arg": "value"}),
                mcp_app_resource_uri: Some("app://connector".into()),
                status: McpToolCallStatus::InProgress,
                result: None,
                error: None,
                duration: None,
            }),
        };

        let legacy_events = event.as_legacy_events(/*show_raw_agent_reasoning*/ false);
        assert_eq!(legacy_events.len(), 1);
        match &legacy_events[0] {
            EventMsg::McpToolCallBegin(event) => {
                assert_eq!(event.call_id, "mcp-1");
                assert_eq!(event.invocation.server, "server");
                assert_eq!(event.invocation.tool, "tool");
                assert_eq!(
                    event.mcp_app_resource_uri.as_deref(),
                    Some("app://connector")
                );
            }
            _ => panic!("expected McpToolCallBegin event"),
        }
    }

    #[test]
    fn item_completed_event_from_image_generation_emits_end_event() {
        let event = ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            item: TurnItem::ImageGeneration(ImageGenerationItem {
                id: "ig-1".into(),
                status: "completed".into(),
                revised_prompt: Some("A tiny blue square".into()),
                result: "Zm9v".into(),
                saved_path: Some(test_path_buf("/tmp/ig-1.png").abs()),
            }),
            completed_at_ms: 0,
        };

        let legacy_events = event.as_legacy_events(/*show_raw_agent_reasoning*/ false);
        assert_eq!(legacy_events.len(), 1);
        match &legacy_events[0] {
            EventMsg::ImageGenerationEnd(event) => {
                assert_eq!(event.call_id, "ig-1");
                assert_eq!(event.status, "completed");
                assert_eq!(event.revised_prompt.as_deref(), Some("A tiny blue square"));
                assert_eq!(event.result, "Zm9v");
                assert_eq!(
                    event.saved_path.as_ref().map(AbsolutePathBuf::as_path),
                    Some(test_path_buf("/tmp/ig-1.png").as_path())
                );
            }
            _ => panic!("expected ImageGenerationEnd event"),
        }
    }

    #[test]
    fn item_completed_event_from_file_change_emits_patch_end_event() {
        let event = ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            completed_at_ms: 0,
            item: TurnItem::FileChange(FileChangeItem {
                id: "patch-1".into(),
                changes: [(
                    PathBuf::from("new.txt"),
                    FileChange::Add {
                        content: "hello".into(),
                    },
                )]
                .into_iter()
                .collect(),
                status: Some(PatchApplyStatus::Completed),
                auto_approved: None,
                stdout: Some("Done!".into()),
                stderr: Some(String::new()),
            }),
        };

        let legacy_events = event.as_legacy_events(/*show_raw_agent_reasoning*/ false);
        assert_eq!(legacy_events.len(), 1);
        match &legacy_events[0] {
            EventMsg::PatchApplyEnd(event) => {
                assert_eq!(event.call_id, "patch-1");
                assert_eq!(event.turn_id, "turn-1");
                assert_eq!(event.stdout, "Done!");
                assert!(event.success);
                assert_eq!(event.status, PatchApplyStatus::Completed);
                assert!(event.changes.contains_key(&PathBuf::from("new.txt")));
            }
            _ => panic!("expected PatchApplyEnd event"),
        }
    }

    #[test]
    fn item_completed_event_from_mcp_tool_call_emits_end_event() {
        let event = ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            completed_at_ms: 0,
            item: TurnItem::McpToolCall(McpToolCallItem {
                id: "mcp-1".into(),
                server: "server".into(),
                tool: "tool".into(),
                arguments: json!({"arg": "value"}),
                mcp_app_resource_uri: Some("app://connector".into()),
                status: McpToolCallStatus::Completed,
                result: Some(CallToolResult {
                    content: vec![json!({"type": "text", "text": "ok"})],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
                error: None,
                duration: Some(Duration::from_millis(42)),
            }),
        };

        let legacy_events = event.as_legacy_events(/*show_raw_agent_reasoning*/ false);
        assert_eq!(legacy_events.len(), 1);
        match &legacy_events[0] {
            EventMsg::McpToolCallEnd(event) => {
                assert_eq!(event.call_id, "mcp-1");
                assert_eq!(event.invocation.server, "server");
                assert_eq!(event.invocation.tool, "tool");
                assert_eq!(
                    event.mcp_app_resource_uri.as_deref(),
                    Some("app://connector")
                );
                assert_eq!(event.duration, Duration::from_millis(42));
                assert!(event.is_success());
            }
            _ => panic!("expected McpToolCallEnd event"),
        }
    }

    #[test]
    fn item_started_event_requires_started_at_ms() {
        let mut value = serde_json::to_value(ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            item: TurnItem::UserMessage(UserMessageItem::new(&[])),
            started_at_ms: 123,
        })
        .unwrap();
        value.as_object_mut().unwrap().remove("started_at_ms");

        assert!(serde_json::from_value::<ItemStartedEvent>(value).is_err());
    }

    #[test]
    fn item_completed_event_defaults_missing_completed_at_ms() {
        let mut value = serde_json::to_value(ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".into(),
            item: TurnItem::UserMessage(UserMessageItem::new(&[])),
            completed_at_ms: 123,
        })
        .unwrap();
        value.as_object_mut().unwrap().remove("completed_at_ms");

        let event = serde_json::from_value::<ItemCompletedEvent>(value).unwrap();
        assert_eq!(event.completed_at_ms, 0);
    }
    #[test]
    fn rollback_failed_error_does_not_affect_turn_status() {
        let event = ErrorEvent {
            message: "rollback failed".into(),
            codex_error_info: Some(CodexErrorInfo::ThreadRollbackFailed),
        };
        assert!(!event.affects_turn_status());
    }

    #[test]
    fn active_turn_not_steerable_error_does_not_affect_turn_status() {
        let event = ErrorEvent {
            message: "cannot steer a review turn".into(),
            codex_error_info: Some(CodexErrorInfo::ActiveTurnNotSteerable {
                turn_kind: NonSteerableTurnKind::Review,
            }),
        };
        assert!(!event.affects_turn_status());
    }

    #[test]
    fn generic_error_affects_turn_status() {
        let event = ErrorEvent {
            message: "generic".into(),
            codex_error_info: Some(CodexErrorInfo::Other),
        };
        assert!(event.affects_turn_status());
    }

    #[test]
    fn conversation_op_serializes_as_unnested_variants() {
        let audio = Op::RealtimeConversationAudio(ConversationAudioParams {
            frame: RealtimeAudioFrame {
                data: "AQID".to_string(),
                sample_rate: 24_000,
                num_channels: 1,
                samples_per_channel: Some(480),
                item_id: None,
            },
        });
        let start = Op::RealtimeConversationStart(ConversationStartParams {
            output_modality: RealtimeOutputModality::Audio,
            prompt: Some(Some("be helpful".to_string())),
            realtime_session_id: Some("conv_1".to_string()),
            transport: None,
            voice: None,
        });
        let webrtc_start = Op::RealtimeConversationStart(ConversationStartParams {
            output_modality: RealtimeOutputModality::Audio,
            prompt: Some(Some("be helpful".to_string())),
            realtime_session_id: Some("conv_1".to_string()),
            transport: Some(ConversationStartTransport::Webrtc {
                sdp: "v=offer\r\n".to_string(),
            }),
            voice: Some(RealtimeVoice::Cove),
        });
        let text = Op::RealtimeConversationText(ConversationTextParams {
            text: "hello".to_string(),
        });
        let close = Op::RealtimeConversationClose;
        let default_prompt_start = Op::RealtimeConversationStart(ConversationStartParams {
            output_modality: RealtimeOutputModality::Audio,
            prompt: None,
            realtime_session_id: None,
            transport: None,
            voice: None,
        });
        let null_prompt_start = Op::RealtimeConversationStart(ConversationStartParams {
            output_modality: RealtimeOutputModality::Audio,
            prompt: Some(None),
            realtime_session_id: None,
            transport: None,
            voice: None,
        });
        let list_voices = Op::RealtimeConversationListVoices;

        assert_eq!(
            serde_json::to_value(&start).unwrap(),
            json!({
                "type": "realtime_conversation_start",
                "output_modality": "audio",
                "prompt": "be helpful",
                "realtime_session_id": "conv_1"
            })
        );
        assert_eq!(
            serde_json::to_value(&default_prompt_start).unwrap(),
            json!({
                "type": "realtime_conversation_start",
                "output_modality": "audio"
            })
        );
        assert_eq!(
            serde_json::to_value(&null_prompt_start).unwrap(),
            json!({
                "type": "realtime_conversation_start",
                "output_modality": "audio",
                "prompt": null
            })
        );
        assert_eq!(
            serde_json::from_value::<Op>(json!({
                "type": "realtime_conversation_start",
                "output_modality": "audio"
            }))
            .unwrap(),
            default_prompt_start
        );
        assert_eq!(
            serde_json::from_value::<Op>(json!({
                "type": "realtime_conversation_start",
                "output_modality": "audio",
                "prompt": null
            }))
            .unwrap(),
            null_prompt_start
        );
        assert_eq!(
            serde_json::to_value(&audio).unwrap(),
            json!({
                "type": "realtime_conversation_audio",
                "frame": {
                    "data": "AQID",
                    "sample_rate": 24000,
                    "num_channels": 1,
                    "samples_per_channel": 480
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<Op>(serde_json::to_value(&text).unwrap()).unwrap(),
            text
        );
        assert_eq!(
            serde_json::to_value(&close).unwrap(),
            json!({
                "type": "realtime_conversation_close"
            })
        );
        assert_eq!(
            serde_json::from_value::<Op>(serde_json::to_value(&close).unwrap()).unwrap(),
            close
        );
        assert_eq!(
            serde_json::to_value(&list_voices).unwrap(),
            json!({
                "type": "realtime_conversation_list_voices"
            })
        );
        assert_eq!(
            serde_json::from_value::<Op>(serde_json::to_value(&list_voices).unwrap()).unwrap(),
            list_voices
        );
        assert_eq!(
            serde_json::to_value(&webrtc_start).unwrap(),
            json!({
                "type": "realtime_conversation_start",
                "output_modality": "audio",
                "prompt": "be helpful",
                "realtime_session_id": "conv_1",
                "transport": {
                    "type": "webrtc",
                    "sdp": "v=offer\r\n"
                },
                "voice": "cove"
            })
        );
    }

    #[test]
    fn realtime_conversation_started_event_uses_realtime_session_id() {
        let event = RealtimeConversationStartedEvent {
            realtime_session_id: Some("conv_1".to_string()),
            version: RealtimeConversationVersion::V2,
        };

        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "realtime_session_id": "conv_1",
                "version": "v2"
            })
        );
    }

    #[test]
    fn realtime_voice_list_is_stable() {
        assert_eq!(
            RealtimeVoicesList::builtin(),
            RealtimeVoicesList {
                v1: vec![
                    RealtimeVoice::Juniper,
                    RealtimeVoice::Maple,
                    RealtimeVoice::Spruce,
                    RealtimeVoice::Ember,
                    RealtimeVoice::Vale,
                    RealtimeVoice::Breeze,
                    RealtimeVoice::Arbor,
                    RealtimeVoice::Sol,
                    RealtimeVoice::Cove,
                ],
                v2: vec![
                    RealtimeVoice::Alloy,
                    RealtimeVoice::Ash,
                    RealtimeVoice::Ballad,
                    RealtimeVoice::Coral,
                    RealtimeVoice::Echo,
                    RealtimeVoice::Sage,
                    RealtimeVoice::Shimmer,
                    RealtimeVoice::Verse,
                    RealtimeVoice::Marin,
                    RealtimeVoice::Cedar,
                ],
                default_v1: RealtimeVoice::Cove,
                default_v2: RealtimeVoice::Marin,
            }
        );
    }

    #[test]
    fn user_input_serialization_omits_final_output_json_schema_when_none() -> Result<()> {
        let op = Op::UserInput {
            environments: None,
            items: Vec::new(),
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        };

        let json_op = serde_json::to_value(op)?;
        assert_eq!(json_op, json!({ "type": "user_input", "items": [] }));

        Ok(())
    }

    #[test]
    fn user_input_deserializes_without_final_output_json_schema_field() -> Result<()> {
        let op: Op = serde_json::from_value(json!({ "type": "user_input", "items": [] }))?;

        assert_eq!(
            op,
            Op::UserInput {
                environments: None,
                items: Vec::new(),
                final_output_json_schema: None,
                responsesapi_client_metadata: None,
            }
        );

        Ok(())
    }

    #[test]
    fn user_input_serialization_includes_final_output_json_schema_when_some() -> Result<()> {
        let schema = json!({
            "type": "object",
            "properties": {
                "answer": { "type": "string" }
            },
            "required": ["answer"],
            "additionalProperties": false
        });
        let op = Op::UserInput {
            environments: None,
            items: Vec::new(),
            final_output_json_schema: Some(schema.clone()),
            responsesapi_client_metadata: None,
        };

        let json_op = serde_json::to_value(op)?;
        assert_eq!(
            json_op,
            json!({
                "type": "user_input",
                "items": [],
                "final_output_json_schema": schema,
            })
        );

        Ok(())
    }

    #[test]
    fn user_input_with_responsesapi_client_metadata_round_trips() -> Result<()> {
        let op = Op::UserInput {
            environments: None,
            items: Vec::new(),
            final_output_json_schema: None,
            responsesapi_client_metadata: Some(HashMap::from([(
                "fiber_run_id".to_string(),
                "fiber-123".to_string(),
            )])),
        };

        let json_op = serde_json::to_value(&op)?;
        assert_eq!(
            json_op,
            json!({
                "type": "user_input",
                "items": [],
                "responsesapi_client_metadata": {
                    "fiber_run_id": "fiber-123",
                }
            })
        );
        assert_eq!(serde_json::from_value::<Op>(json_op)?, op);

        Ok(())
    }

    #[test]
    fn user_input_text_serializes_empty_text_elements() -> Result<()> {
        let input = UserInput::Text {
            text: "hello".to_string(),
            text_elements: Vec::new(),
        };

        let json_input = serde_json::to_value(input)?;
        assert_eq!(
            json_input,
            json!({
                "type": "text",
                "text": "hello",
                "text_elements": [],
            })
        );

        Ok(())
    }

    #[test]
    fn user_message_event_serializes_empty_metadata_vectors() -> Result<()> {
        let event = UserMessageEvent {
            message: "hello".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        };

        let json_event = serde_json::to_value(event)?;
        assert_eq!(
            json_event,
            json!({
                "message": "hello",
                "local_images": [],
                "skills": [],
                "text_elements": [],
            })
        );

        Ok(())
    }

    #[test]
    fn turn_aborted_event_deserializes_without_turn_id() -> Result<()> {
        let event: EventMsg = serde_json::from_value(json!({
            "type": "turn_aborted",
            "reason": "interrupted",
        }))?;

        match event {
            EventMsg::TurnAborted(TurnAbortedEvent {
                turn_id, reason, ..
            }) => {
                assert_eq!(turn_id, None);
                assert_eq!(reason, TurnAbortReason::Interrupted);
            }
            _ => panic!("expected turn_aborted event"),
        }

        Ok(())
    }

    #[test]
    fn turn_context_item_deserializes_without_network() -> Result<()> {
        let item: TurnContextItem = serde_json::from_value(json!({
            "cwd": test_path_buf("/tmp"),
            "approval_policy": "never",
            "sandbox_policy": { "type": "danger-full-access" },
            "model": "gpt-5",
            "summary": "auto",
        }))?;

        assert_eq!(item.trace_id, None);
        assert_eq!(item.network, None);
        assert_eq!(item.file_system_sandbox_policy, None);
        Ok(())
    }

    #[test]
    fn turn_context_item_serializes_network_when_present() -> Result<()> {
        let item = TurnContextItem {
            turn_id: None,
            trace_id: None,
            cwd: test_path_buf("/tmp"),
            current_date: None,
            timezone: None,
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            permission_profile: None,
            network: Some(TurnContextNetworkItem {
                allowed_domains: vec!["api.example.com".to_string()],
                denied_domains: vec!["blocked.example.com".to_string()],
            }),
            file_system_sandbox_policy: Some(FileSystemSandboxPolicy::restricted(vec![
                FileSystemSandboxEntry {
                    path: FileSystemPath::GlobPattern {
                        pattern: "/tmp/private/**/*.txt".to_string(),
                    },
                    access: FileSystemAccessMode::None,
                },
            ])),
            model: "gpt-5".to_string(),
            personality: None,
            collaboration_mode: None,
            realtime_active: None,
            effort: None,
            summary: ReasoningSummaryConfig::Auto,
            user_instructions: None,
            developer_instructions: None,
            final_output_json_schema: None,
            truncation_policy: None,
        };

        let value = serde_json::to_value(item)?;
        assert_eq!(
            value["network"],
            json!({
                "allowed_domains": ["api.example.com"],
                "denied_domains": ["blocked.example.com"],
            })
        );
        assert_eq!(
            value["file_system_sandbox_policy"],
            json!({
                "kind": "restricted",
                "entries": [{
                    "path": {
                        "type": "glob_pattern",
                        "pattern": "/tmp/private/**/*.txt"
                    },
                    "access": "none"
                }]
            })
        );
        Ok(())
    }

    /// Serialize Event to verify that its JSON representation has the expected
    /// amount of nesting.
    #[test]
    fn serialize_event() -> Result<()> {
        let session_id = SessionId::from_string("67e55044-10b1-426f-9247-bb680e5fe0c7")?;
        let thread_id = ThreadId::from_string("67e55044-10b1-426f-9247-bb680e5fe0c8")?;
        let rollout_file = NamedTempFile::new()?;
        let permission_profile = PermissionProfile::read_only();
        let event = Event {
            id: "1234".to_string(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id,
                thread_id,
                forked_from_id: None,
                thread_source: None,
                thread_name: None,
                model: "codex-mini-latest".to_string(),
                model_provider_id: "openai".to_string(),
                service_tier: None,
                approval_policy: AskForApproval::Never,
                approvals_reviewer: ApprovalsReviewer::User,
                permission_profile: permission_profile.clone(),
                active_permission_profile: None,
                cwd: test_path_buf("/home/user/project").abs(),
                reasoning_effort: Some(ReasoningEffortConfig::default()),
                initial_messages: None,
                network_proxy: None,
                rollout_path: Some(rollout_file.path().to_path_buf()),
            }),
        };

        let expected = json!({
            "id": "1234",
            "msg": {
                "type": "session_configured",
                "session_id": "67e55044-10b1-426f-9247-bb680e5fe0c7",
                "thread_id": "67e55044-10b1-426f-9247-bb680e5fe0c8",
                "model": "codex-mini-latest",
                "model_provider_id": "openai",
                "approval_policy": "never",
                "approvals_reviewer": "user",
                "permission_profile": permission_profile,
                "cwd": test_path_buf("/home/user/project"),
                "reasoning_effort": "medium",
                "rollout_path": format!("{}", rollout_file.path().display()),
            }
        });
        assert_eq!(expected, serde_json::to_value(&event)?);
        Ok(())
    }

    #[test]
    fn deserialize_legacy_session_configured_event_uses_sandbox_policy() -> Result<()> {
        let cwd = test_path_buf("/home/user/project");
        let value = json!({
            "session_id": "67e55044-10b1-426f-9247-bb680e5fe0c8",
            "model": "codex-mini-latest",
            "model_provider_id": "openai",
            "approval_policy": "never",
            "approvals_reviewer": "user",
            "sandbox_policy": {
                "type": "read-only"
            },
            "cwd": cwd,
        });

        let event: SessionConfiguredEvent = serde_json::from_value(value)?;
        assert_eq!(event.permission_profile, PermissionProfile::read_only());
        Ok(())
    }

    #[test]
    fn vec_u8_as_base64_serialization_and_deserialization() -> Result<()> {
        let event = ExecCommandOutputDeltaEvent {
            call_id: "call21".to_string(),
            sequence: None,
            generates_notification: false,
            created_at_ms: 0,
            stream: ExecOutputStream::Stdout,
            chunk: vec![1, 2, 3, 4, 5],
        };
        let serialized = serde_json::to_string(&event)?;
        assert_eq!(
            r#"{"call_id":"call21","generates_notification":false,"created_at_ms":0,"stream":"stdout","chunk":"AQIDBAU="}"#,
            serialized,
        );

        let deserialized: ExecCommandOutputDeltaEvent = serde_json::from_str(&serialized)?;
        assert_eq!(deserialized, event);
        Ok(())
    }

    #[test]
    fn serialize_mcp_startup_update_event() -> Result<()> {
        let event = Event {
            id: "init".to_string(),
            msg: EventMsg::McpStartupUpdate(McpStartupUpdateEvent {
                server: "srv".to_string(),
                status: McpStartupStatus::Failed {
                    error: "boom".to_string(),
                },
            }),
        };

        let value = serde_json::to_value(&event)?;
        assert_eq!(value["msg"]["type"], "mcp_startup_update");
        assert_eq!(value["msg"]["server"], "srv");
        assert_eq!(value["msg"]["status"]["state"], "failed");
        assert_eq!(value["msg"]["status"]["error"], "boom");
        Ok(())
    }

    #[test]
    fn serialize_mcp_startup_complete_event() -> Result<()> {
        let event = Event {
            id: "init".to_string(),
            msg: EventMsg::McpStartupComplete(McpStartupCompleteEvent {
                ready: vec!["a".to_string()],
                failed: vec![McpStartupFailure {
                    server: "b".to_string(),
                    error: "bad".to_string(),
                }],
                cancelled: vec!["c".to_string()],
            }),
        };

        let value = serde_json::to_value(&event)?;
        assert_eq!(value["msg"]["type"], "mcp_startup_complete");
        assert_eq!(value["msg"]["ready"][0], "a");
        assert_eq!(value["msg"]["failed"][0]["server"], "b");
        assert_eq!(value["msg"]["failed"][0]["error"], "bad");
        assert_eq!(value["msg"]["cancelled"][0], "c");
        Ok(())
    }

    #[test]
    fn token_usage_info_new_or_append_updates_context_window_when_provided() {
        let initial = Some(TokenUsageInfo {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(258_400),
        });
        let last = Some(TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: 10,
        });

        let info = TokenUsageInfo::new_or_append(&initial, &last, Some(128_000))
            .expect("new_or_append should return info");

        assert_eq!(info.model_context_window, Some(128_000));
    }

    #[test]
    fn token_usage_info_new_or_append_preserves_context_window_when_not_provided() {
        let initial = Some(TokenUsageInfo {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(258_400),
        });
        let last = Some(TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: 10,
        });

        let info =
            TokenUsageInfo::new_or_append(&initial, &last, /*model_context_window*/ None)
                .expect("new_or_append should return info");

        assert_eq!(info.model_context_window, Some(258_400));
    }
}
