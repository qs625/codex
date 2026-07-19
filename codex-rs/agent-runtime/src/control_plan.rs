use crate::AgentMetadata;
use crate::AgentMode;
use crate::SpawnAgentForkMode;
use crate::SpawnReservation;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::error::CodexErr;
use protocol::protocol::AgentStatus;
use protocol::protocol::Op;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::protocol::ThreadLifecycleFinalStatus;
use protocol::protocol::ThreadLifecycleStatus;
use protocol::protocol::ThreadLifecycleWaitReason;
use protocol::protocol::TurnEnvironmentSelection;
use protocol::user_input::UserInput;
use serde::Serialize;
use std::collections::HashMap;

const AGENT_NAMES: &str = include_str!("agent_names.txt");
const ROOT_LAST_TASK_MESSAGE: &str = "Main thread";

#[derive(Clone, Debug, Default)]
pub struct SpawnAgentOptions {
    pub fork_parent_spawn_call_id: Option<String>,
    pub fork_mode: Option<SpawnAgentForkMode>,
    pub environments: Option<Vec<TurnEnvironmentSelection>>,
    pub agent_mode: AgentMode,
}

#[derive(Clone, Debug)]
pub struct LiveAgent {
    pub thread_id: ThreadId,
    pub metadata: crate::AgentMetadata,
    pub status: AgentStatus,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ListedAgent {
    pub agent_name: String,
    pub lifecycle_status: ThreadLifecycleStatus,
    pub last_task_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedAgentCandidate {
    pub thread_id: ThreadId,
    pub agent_name: String,
    pub last_task_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListAgentsPlan {
    pub include_root: bool,
    pub candidates: Vec<ListedAgentCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadSpawnChild {
    pub parent_thread_id: ThreadId,
    pub thread_id: ThreadId,
    pub metadata: AgentMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveAgentShutdownAction {
    SubmitWithoutWait,
    SubmitAndWait,
    AlreadyShutdownWait,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentThreadActivityInputs {
    pub manager_available: bool,
    pub active_event_subscription_count: usize,
    pub thread_found: bool,
    pub has_active_turn: bool,
    pub status: Option<AgentStatus>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadLifecycleInputs {
    pub manager_available: bool,
    pub active_event_subscription_count: usize,
    pub thread_found: bool,
    pub has_active_turn: bool,
    pub live_agent_status: Option<AgentStatus>,
    pub persisted_final_agent_status: Option<AgentStatus>,
}

pub struct ThreadSpawnPlanInput<'a> {
    pub parent_thread_id: ThreadId,
    pub depth: i32,
    pub agent_path: Option<AgentPath>,
    pub agent_role: Option<String>,
    pub agent_mode: AgentMode,
    pub configured_nickname_candidates: Option<&'a [String]>,
    pub preferred_agent_nickname: Option<&'a str>,
}

pub fn default_agent_nickname_list() -> Vec<&'static str> {
    AGENT_NAMES
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect()
}

pub fn agent_nickname_candidates(configured_candidates: Option<&[String]>) -> Vec<String> {
    if let Some(candidates) = configured_candidates {
        return candidates.to_vec();
    }

    default_agent_nickname_list()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect()
}

pub fn prepare_thread_spawn_plan(
    reservation: &mut SpawnReservation,
    input: ThreadSpawnPlanInput<'_>,
) -> protocol::error::Result<(SessionSource, AgentMetadata)> {
    let ThreadSpawnPlanInput {
        parent_thread_id,
        depth,
        agent_path,
        agent_role,
        agent_mode,
        configured_nickname_candidates,
        preferred_agent_nickname,
    } = input;

    if let Some(agent_path) = agent_path.as_ref() {
        reservation.reserve_agent_path(agent_path)?;
    }
    let candidate_names = agent_nickname_candidates(configured_nickname_candidates);
    let candidate_name_refs: Vec<&str> = candidate_names.iter().map(String::as_str).collect();
    let agent_nickname =
        Some(reservation.reserve_agent_nickname_with_preference(
            &candidate_name_refs,
            preferred_agent_nickname,
        )?);
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id,
        depth,
        agent_path: agent_path.clone(),
        agent_nickname: agent_nickname.clone(),
        agent_role: agent_role.clone(),
    });
    let agent_metadata = AgentMetadata {
        agent_id: None,
        agent_path,
        agent_nickname,
        agent_role,
        agent_mode,
        last_task_message: None,
        counted: true,
    };
    Ok((session_source, agent_metadata))
}

pub fn thread_spawn_parent_thread_id(session_source: &SessionSource) -> Option<ThreadId> {
    match session_source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id, ..
        }) => Some(*parent_thread_id),
        _ => None,
    }
}

pub fn should_register_session_root(session_source: &SessionSource) -> bool {
    thread_spawn_parent_thread_id(session_source).is_none()
}

pub fn current_agent_path_for_session(
    session_source: &SessionSource,
    registry_metadata: Option<&AgentMetadata>,
) -> AgentPath {
    session_source
        .get_agent_path()
        .or_else(|| registry_metadata.and_then(|metadata| metadata.agent_path.clone()))
        .unwrap_or_else(AgentPath::root)
}

pub fn resolve_agent_reference_path(
    current_agent_path: &AgentPath,
    agent_reference: &str,
) -> Result<AgentPath, String> {
    current_agent_path.resolve(agent_reference)
}

pub fn agent_subtree_thread_ids(
    root_thread_id: ThreadId,
    descendants: Vec<ThreadId>,
) -> Vec<ThreadId> {
    let mut thread_ids = vec![root_thread_id];
    thread_ids.extend(descendants);
    thread_ids
}

pub fn direct_subagent_paths_from_children(
    children: Vec<(ThreadId, AgentMetadata)>,
) -> Vec<AgentPath> {
    children
        .into_iter()
        .filter_map(|(_, metadata)| metadata.agent_path)
        .collect()
}

pub fn should_release_agent_after_thread_request_error(error: &CodexErr) -> bool {
    matches!(error, CodexErr::InternalAgentDied)
}

pub fn live_agent_shutdown_action(
    thread_found: bool,
    status: Option<&AgentStatus>,
) -> LiveAgentShutdownAction {
    if !thread_found {
        return LiveAgentShutdownAction::SubmitWithoutWait;
    }
    if status.is_some_and(|status| matches!(status, AgentStatus::Shutdown)) {
        return LiveAgentShutdownAction::AlreadyShutdownWait;
    }
    LiveAgentShutdownAction::SubmitAndWait
}

pub fn should_ignore_descendant_shutdown_error(error: &CodexErr) -> bool {
    matches!(
        error,
        CodexErr::ThreadNotFound(_) | CodexErr::InternalAgentDied
    )
}

pub fn agent_thread_is_active_from_inputs(inputs: AgentThreadActivityInputs) -> bool {
    let lifecycle = normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
        manager_available: inputs.manager_available,
        active_event_subscription_count: inputs.active_event_subscription_count,
        thread_found: inputs.thread_found,
        has_active_turn: inputs.has_active_turn,
        live_agent_status: inputs.status,
        persisted_final_agent_status: None,
    });
    thread_lifecycle_is_active(&lifecycle)
}

pub fn normalized_thread_lifecycle_from_inputs(
    inputs: ThreadLifecycleInputs,
) -> ThreadLifecycleStatus {
    if !inputs.manager_available {
        return ThreadLifecycleStatus::SystemError {
            message: Some("thread manager unavailable".to_string()),
        };
    }

    let agent_status = match inputs.live_agent_status {
        Some(AgentStatus::PendingInit) => inputs
            .persisted_final_agent_status
            .clone()
            .unwrap_or(AgentStatus::PendingInit),
        Some(status) => status,
        None => {
            if let Some(status) = inputs.persisted_final_agent_status {
                return thread_lifecycle_from_final_agent_status(status);
            }
            if !inputs.thread_found {
                return ThreadLifecycleStatus::NotLoaded;
            }
            return ThreadLifecycleStatus::Active {
                active_flags: Vec::new(),
            };
        }
    };

    if crate::is_final(&agent_status) {
        return thread_lifecycle_from_final_agent_status(agent_status);
    }
    if inputs.active_event_subscription_count > 0 {
        return ThreadLifecycleStatus::Waiting {
            reason: ThreadLifecycleWaitReason::EventSubscription,
        };
    }
    if inputs.has_active_turn {
        return ThreadLifecycleStatus::Active {
            active_flags: Vec::new(),
        };
    }
    match agent_status {
        AgentStatus::PendingInit => ThreadLifecycleStatus::Initializing,
        AgentStatus::Running => ThreadLifecycleStatus::Active {
            active_flags: Vec::new(),
        },
        AgentStatus::NotFound => ThreadLifecycleStatus::NotLoaded,
        AgentStatus::Completed(_)
        | AgentStatus::Errored(_)
        | AgentStatus::Interrupted
        | AgentStatus::Shutdown => {
            thread_lifecycle_from_final_agent_status(agent_status)
        }
    }
}

pub fn thread_lifecycle_is_active(lifecycle: &ThreadLifecycleStatus) -> bool {
    matches!(
        lifecycle,
        ThreadLifecycleStatus::Initializing
            | ThreadLifecycleStatus::Active { .. }
            | ThreadLifecycleStatus::Waiting { .. }
    )
}

pub fn thread_lifecycle_from_final_agent_status(status: AgentStatus) -> ThreadLifecycleStatus {
    match status {
        AgentStatus::Completed(last_agent_message) => ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Completed { last_agent_message },
        },
        AgentStatus::Errored(message) => ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Errored {
                message: Some(message),
            },
        },
        AgentStatus::Interrupted => ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Interrupted,
        },
        AgentStatus::Shutdown => ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Shutdown,
        },
        AgentStatus::PendingInit | AgentStatus::Running => ThreadLifecycleStatus::Active {
            active_flags: Vec::new(),
        },
        AgentStatus::NotFound => ThreadLifecycleStatus::NotLoaded,
    }
}

pub fn any_agent_thread_active(active_flags: impl IntoIterator<Item = bool>) -> bool {
    active_flags.into_iter().any(|active| active)
}

pub fn thread_spawn_depth(session_source: &SessionSource) -> Option<i32> {
    match session_source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn { depth, .. }) => Some(*depth),
        _ => None,
    }
}

pub fn agent_matches_prefix(agent_path: Option<&AgentPath>, prefix: &AgentPath) -> bool {
    agent_path.is_some_and(|agent_path| {
        agent_path == prefix
            || agent_path
                .as_str()
                .strip_prefix(prefix.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

pub fn root_listed_agent(lifecycle_status: ThreadLifecycleStatus) -> ListedAgent {
    ListedAgent {
        agent_name: AgentPath::root().to_string(),
        lifecycle_status,
        last_task_message: Some(ROOT_LAST_TASK_MESSAGE.to_string()),
    }
}

pub fn list_agents_plan(
    current_agent_path: &AgentPath,
    path_prefix: Option<&str>,
    mut live_agents: Vec<AgentMetadata>,
) -> Result<ListAgentsPlan, String> {
    let resolved_prefix = path_prefix
        .map(|prefix| current_agent_path.resolve(prefix))
        .transpose()?;

    live_agents.sort_by(|left, right| {
        left.agent_path
            .as_deref()
            .unwrap_or_default()
            .cmp(right.agent_path.as_deref().unwrap_or_default())
            .then_with(|| {
                left.agent_id
                    .map(|id| id.to_string())
                    .unwrap_or_default()
                    .cmp(&right.agent_id.map(|id| id.to_string()).unwrap_or_default())
            })
    });

    let root_path = AgentPath::root();
    let include_root = match resolved_prefix.as_ref() {
        Some(prefix) => agent_matches_prefix(Some(&root_path), prefix),
        None => agent_matches_prefix(Some(current_agent_path), &root_path),
    };
    let candidates = live_agents
        .into_iter()
        .filter_map(|metadata| {
            let thread_id = metadata.agent_id?;
            if resolved_prefix
                .as_ref()
                .is_some_and(|prefix| !agent_matches_prefix(metadata.agent_path.as_ref(), prefix))
            {
                return None;
            }

            let agent_name = metadata
                .agent_path
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| thread_id.to_string());
            Some(ListedAgentCandidate {
                thread_id,
                agent_name,
                last_task_message: metadata.last_task_message,
            })
        })
        .collect();

    Ok(ListAgentsPlan {
        include_root,
        candidates,
    })
}

pub fn build_thread_spawn_children_by_parent(
    children: Vec<ThreadSpawnChild>,
) -> HashMap<ThreadId, Vec<(ThreadId, AgentMetadata)>> {
    let mut children_by_parent = HashMap::<ThreadId, Vec<(ThreadId, AgentMetadata)>>::new();
    for child in children {
        children_by_parent
            .entry(child.parent_thread_id)
            .or_default()
            .push((child.thread_id, child.metadata));
    }

    for children in children_by_parent.values_mut() {
        children.sort_by(|left, right| {
            left.1
                .agent_path
                .as_deref()
                .unwrap_or_default()
                .cmp(right.1.agent_path.as_deref().unwrap_or_default())
                .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
        });
    }

    children_by_parent
}

pub fn thread_spawn_descendants(
    root_thread_id: ThreadId,
    mut children_by_parent: HashMap<ThreadId, Vec<(ThreadId, AgentMetadata)>>,
) -> Vec<ThreadId> {
    let mut descendants = Vec::new();
    let mut stack = children_by_parent
        .remove(&root_thread_id)
        .unwrap_or_default()
        .into_iter()
        .map(|(child_thread_id, _)| child_thread_id)
        .rev()
        .collect::<Vec<_>>();

    while let Some(thread_id) = stack.pop() {
        descendants.push(thread_id);
        if let Some(children) = children_by_parent.remove(&thread_id) {
            for (child_thread_id, _) in children.into_iter().rev() {
                stack.push(child_thread_id);
            }
        }
    }

    descendants
}

pub fn render_input_preview(initial_operation: &Op) -> String {
    match initial_operation {
        Op::UserInput { items, .. } => items
            .iter()
            .map(|item| match item {
                UserInput::Text { text, .. } => text.clone(),
                UserInput::Image { .. } => "[image]".to_string(),
                UserInput::LocalImage { path } => format!("[local_image:{}]", path.display()),
                UserInput::Skill { name, path } => format!("[skill:${name}]({})", path.display()),
                UserInput::Mention { name, path } => format!("[mention:${name}]({path})"),
                _ => "[input]".to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Op::InterAgentCommunication { communication } => communication.content.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_prefix_matches_root_and_descendants() {
        let root = AgentPath::root();
        let worker = AgentPath::try_from("/root/worker").expect("agent path");
        let nested = AgentPath::try_from("/root/worker/reviewer").expect("agent path");
        let sibling = AgentPath::try_from("/root/owner").expect("agent path");
        let project = AgentPath::try_from("/project").expect("project path");

        assert!(agent_matches_prefix(Some(&worker), &root));
        assert!(agent_matches_prefix(Some(&worker), &worker));
        assert!(agent_matches_prefix(Some(&nested), &worker));
        assert!(!agent_matches_prefix(Some(&sibling), &worker));
        assert!(!agent_matches_prefix(Some(&project), &root));
        assert!(!agent_matches_prefix(None, &worker));
    }

    #[test]
    fn prepare_thread_spawn_plan_reserves_path_and_builds_metadata() {
        let registry = std::sync::Arc::new(crate::AgentRegistry::default());
        let mut reservation = registry.reserve_spawn_slot(None).expect("reservation");
        let parent_thread_id = ThreadId::new();
        let agent_path = AgentPath::try_from("/root/owner").expect("agent path");
        let candidates = vec!["Owner".to_string()];

        let (session_source, metadata) = prepare_thread_spawn_plan(
            &mut reservation,
            ThreadSpawnPlanInput {
                parent_thread_id,
                depth: 2,
                agent_path: Some(agent_path.clone()),
                agent_role: Some("feature-owner".to_string()),
                agent_mode: AgentMode::Management,
                configured_nickname_candidates: Some(&candidates),
                preferred_agent_nickname: None,
            },
        )
        .expect("spawn plan");

        assert_eq!(
            session_source,
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 2,
                agent_path: Some(agent_path.clone()),
                agent_nickname: Some("Owner".to_string()),
                agent_role: Some("feature-owner".to_string()),
            })
        );
        assert_eq!(metadata.agent_path, Some(agent_path.clone()));
        assert_eq!(metadata.agent_nickname, Some("Owner".to_string()));
        assert_eq!(metadata.agent_role, Some("feature-owner".to_string()));
        assert_eq!(metadata.agent_mode, AgentMode::Management);
    }

    #[test]
    fn prepare_thread_spawn_plan_uses_preferred_nickname() {
        let registry = std::sync::Arc::new(crate::AgentRegistry::default());
        let mut reservation = registry.reserve_spawn_slot(None).expect("reservation");
        let parent_thread_id = ThreadId::new();
        let candidates = vec!["Owner".to_string()];

        let (_, metadata) = prepare_thread_spawn_plan(
            &mut reservation,
            ThreadSpawnPlanInput {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_role: None,
                agent_mode: AgentMode::Normal,
                configured_nickname_candidates: Some(&candidates),
                preferred_agent_nickname: Some("ResumedOwner"),
            },
        )
        .expect("spawn plan");

        assert_eq!(metadata.agent_nickname, Some("ResumedOwner".to_string()));
    }

    #[test]
    fn list_agents_plan_filters_sorts_and_excludes_root_for_descendant_prefix() {
        let root = AgentPath::root();
        let worker = AgentPath::try_from("/root/worker").expect("agent path");
        let reviewer = AgentPath::try_from("/root/worker/reviewer").expect("agent path");
        let sibling = AgentPath::try_from("/root/sibling").expect("agent path");
        let first_thread = ThreadId::new();
        let second_thread = ThreadId::new();
        let skipped_thread = ThreadId::new();

        let plan = list_agents_plan(
            &root,
            Some("worker"),
            vec![
                AgentMetadata {
                    agent_id: Some(skipped_thread),
                    agent_path: Some(sibling),
                    ..Default::default()
                },
                AgentMetadata {
                    agent_id: Some(second_thread),
                    agent_path: Some(reviewer),
                    last_task_message: Some("review".to_string()),
                    ..Default::default()
                },
                AgentMetadata {
                    agent_id: Some(first_thread),
                    agent_path: Some(worker),
                    last_task_message: Some("work".to_string()),
                    ..Default::default()
                },
                AgentMetadata {
                    agent_id: None,
                    agent_path: Some(AgentPath::try_from("/root/worker/pending").unwrap()),
                    ..Default::default()
                },
            ],
        )
        .expect("plan");

        assert!(!plan.include_root);
        assert_eq!(
            plan.candidates,
            vec![
                ListedAgentCandidate {
                    thread_id: first_thread,
                    agent_name: "/root/worker".to_string(),
                    last_task_message: Some("work".to_string()),
                },
                ListedAgentCandidate {
                    thread_id: second_thread,
                    agent_name: "/root/worker/reviewer".to_string(),
                    last_task_message: Some("review".to_string()),
                },
            ]
        );
    }

    #[test]
    fn list_agents_plan_root_prefix_does_not_match_project_root_paths() {
        let root = AgentPath::root();
        let root_worker = AgentPath::try_from("/root/worker").expect("root worker path");
        let project = AgentPath::try_from("/project").expect("project path");
        let root_thread = ThreadId::new();
        let project_thread = ThreadId::new();

        let root_plan = list_agents_plan(
            &project,
            Some("/root"),
            vec![
                AgentMetadata {
                    agent_id: Some(project_thread),
                    agent_path: Some(project.clone()),
                    ..Default::default()
                },
                AgentMetadata {
                    agent_id: Some(root_thread),
                    agent_path: Some(root_worker.clone()),
                    ..Default::default()
                },
            ],
        )
        .expect("root prefix plan");

        assert!(root_plan.include_root);
        assert_eq!(
            root_plan.candidates,
            vec![ListedAgentCandidate {
                thread_id: root_thread,
                agent_name: root_worker.to_string(),
                last_task_message: None,
            }]
        );

        let all_plan = list_agents_plan(
            &project,
            None,
            vec![AgentMetadata {
                agent_id: Some(project_thread),
                agent_path: Some(project.clone()),
                ..Default::default()
            }],
        )
        .expect("all agents plan");
        assert!(!all_plan.include_root);
        assert_eq!(
            all_plan.candidates,
            vec![ListedAgentCandidate {
                thread_id: project_thread,
                agent_name: project.to_string(),
                last_task_message: None,
            }]
        );

        let legacy_root_child = AgentPath::try_from("/root/worker").expect("root worker path");
        let legacy_plan = list_agents_plan(&legacy_root_child, None, Vec::new())
            .expect("legacy root plan");
        assert!(legacy_plan.include_root);
    }

    #[test]
    fn list_agents_plan_uses_thread_id_when_path_is_missing() {
        let thread_id = ThreadId::new();
        let plan = list_agents_plan(
            &AgentPath::root(),
            None,
            vec![AgentMetadata {
                agent_id: Some(thread_id),
                agent_path: None,
                ..Default::default()
            }],
        )
        .expect("plan");

        assert_eq!(
            plan.candidates,
            vec![ListedAgentCandidate {
                thread_id,
                agent_name: thread_id.to_string(),
                last_task_message: None,
            }]
        );
    }

    #[test]
    fn list_agents_plan_includes_root_without_prefix() {
        let plan = list_agents_plan(&AgentPath::root(), None, Vec::new()).expect("plan");

        assert!(plan.include_root);
        assert_eq!(plan.candidates, Vec::new());
    }

    #[test]
    fn build_thread_spawn_children_sorts_each_parent_by_path_then_thread_id() {
        let parent_thread_id = fixed_thread_id(1);
        let first_thread_id = fixed_thread_id(2);
        let second_thread_id = fixed_thread_id(3);
        let missing_path_thread_id = fixed_thread_id(4);
        let first_path = AgentPath::try_from("/root/a").expect("agent path");
        let second_path = AgentPath::try_from("/root/b").expect("agent path");

        let children = build_thread_spawn_children_by_parent(vec![
            ThreadSpawnChild {
                parent_thread_id,
                thread_id: second_thread_id,
                metadata: metadata_with_path(second_path.clone()),
            },
            ThreadSpawnChild {
                parent_thread_id,
                thread_id: first_thread_id,
                metadata: metadata_with_path(first_path.clone()),
            },
            ThreadSpawnChild {
                parent_thread_id,
                thread_id: missing_path_thread_id,
                metadata: AgentMetadata::default(),
            },
        ]);

        assert_eq!(
            children.get(&parent_thread_id),
            Some(&vec![
                (missing_path_thread_id, AgentMetadata::default()),
                (first_thread_id, metadata_with_path(first_path)),
                (second_thread_id, metadata_with_path(second_path)),
            ])
        );
    }

    #[test]
    fn thread_spawn_descendants_returns_depth_first_order_using_sorted_children() {
        let root = fixed_thread_id(1);
        let first_child = fixed_thread_id(2);
        let second_child = fixed_thread_id(3);
        let grandchild = fixed_thread_id(4);
        let children = build_thread_spawn_children_by_parent(vec![
            ThreadSpawnChild {
                parent_thread_id: root,
                thread_id: second_child,
                metadata: metadata_with_path(AgentPath::try_from("/root/b").unwrap()),
            },
            ThreadSpawnChild {
                parent_thread_id: first_child,
                thread_id: grandchild,
                metadata: metadata_with_path(AgentPath::try_from("/root/a/grandchild").unwrap()),
            },
            ThreadSpawnChild {
                parent_thread_id: root,
                thread_id: first_child,
                metadata: metadata_with_path(AgentPath::try_from("/root/a").unwrap()),
            },
        ]);

        assert_eq!(
            thread_spawn_descendants(root, children),
            vec![first_child, grandchild, second_child]
        );
    }

    #[test]
    fn should_register_session_root_only_for_non_thread_spawn_sources() {
        assert!(should_register_session_root(&SessionSource::Exec));
        assert!(!should_register_session_root(&SessionSource::SubAgent(
            SubAgentSource::ThreadSpawn {
                parent_thread_id: fixed_thread_id(1),
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            }
        )));
    }

    #[test]
    fn current_agent_path_prefers_source_then_metadata_then_root() {
        let source_path = AgentPath::try_from("/root/source").expect("agent path");
        let metadata_path = AgentPath::try_from("/root/metadata").expect("agent path");
        let source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: fixed_thread_id(1),
            depth: 1,
            agent_path: Some(source_path.clone()),
            agent_nickname: None,
            agent_role: None,
        });
        let no_path_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: fixed_thread_id(1),
            depth: 1,
            agent_path: None,
            agent_nickname: None,
            agent_role: None,
        });
        let metadata = metadata_with_path(metadata_path.clone());

        assert_eq!(
            current_agent_path_for_session(&source, Some(&metadata)),
            source_path
        );
        assert_eq!(
            current_agent_path_for_session(&no_path_source, Some(&metadata)),
            metadata_path
        );
        assert_eq!(
            current_agent_path_for_session(&SessionSource::Exec, None),
            AgentPath::root()
        );
    }

    #[test]
    fn resolve_agent_reference_path_resolves_relative_paths() {
        let current = AgentPath::try_from("/root/owner").expect("agent path");

        assert_eq!(
            resolve_agent_reference_path(&current, "reviewer").expect("resolved"),
            AgentPath::try_from("/root/owner/reviewer").expect("agent path")
        );
    }

    #[test]
    fn agent_subtree_thread_ids_keeps_root_first() {
        let root = fixed_thread_id(1);
        let first = fixed_thread_id(2);
        let second = fixed_thread_id(3);

        assert_eq!(
            agent_subtree_thread_ids(root, vec![first, second]),
            vec![root, first, second]
        );
    }

    #[test]
    fn direct_subagent_paths_from_children_filters_missing_paths() {
        let first = AgentPath::try_from("/root/first").expect("agent path");
        let second = AgentPath::try_from("/root/second").expect("agent path");

        assert_eq!(
            direct_subagent_paths_from_children(vec![
                (fixed_thread_id(1), metadata_with_path(first.clone())),
                (fixed_thread_id(2), AgentMetadata::default()),
                (fixed_thread_id(3), metadata_with_path(second.clone())),
            ]),
            vec![first, second]
        );
    }

    #[test]
    fn thread_request_error_policy_releases_only_internal_agent_died() {
        assert!(should_release_agent_after_thread_request_error(
            &CodexErr::InternalAgentDied
        ));
        assert!(!should_release_agent_after_thread_request_error(
            &CodexErr::UnsupportedOperation("nope".to_string())
        ));
    }

    #[test]
    fn live_agent_shutdown_action_uses_thread_presence_and_status() {
        assert_eq!(
            live_agent_shutdown_action(/*thread_found*/ false, None),
            LiveAgentShutdownAction::SubmitWithoutWait
        );
        assert_eq!(
            live_agent_shutdown_action(/*thread_found*/ true, Some(&AgentStatus::Shutdown)),
            LiveAgentShutdownAction::AlreadyShutdownWait
        );
        assert_eq!(
            live_agent_shutdown_action(/*thread_found*/ true, Some(&AgentStatus::Running)),
            LiveAgentShutdownAction::SubmitAndWait
        );
    }

    #[test]
    fn descendant_shutdown_error_policy_ignores_missing_or_dead_agents() {
        assert!(should_ignore_descendant_shutdown_error(
            &CodexErr::InternalAgentDied
        ));
        assert!(should_ignore_descendant_shutdown_error(
            &CodexErr::ThreadNotFound(fixed_thread_id(1))
        ));
        assert!(!should_ignore_descendant_shutdown_error(
            &CodexErr::UnsupportedOperation("blocked".to_string())
        ));
    }

    #[test]
    fn agent_thread_activity_uses_runtime_facts_in_order() {
        assert!(!agent_thread_is_active_from_inputs(
            AgentThreadActivityInputs::default()
        ));
        assert!(agent_thread_is_active_from_inputs(
            AgentThreadActivityInputs {
                manager_available: true,
                thread_found: true,
                active_event_subscription_count: 1,
                ..AgentThreadActivityInputs::default()
            }
        ));
        assert!(!agent_thread_is_active_from_inputs(
            AgentThreadActivityInputs {
                manager_available: true,
                thread_found: false,
                active_event_subscription_count: 1,
                ..AgentThreadActivityInputs::default()
            }
        ));
        assert!(!agent_thread_is_active_from_inputs(
            AgentThreadActivityInputs {
                manager_available: true,
                thread_found: false,
                status: Some(AgentStatus::Running),
                ..AgentThreadActivityInputs::default()
            }
        ));
        assert!(!agent_thread_is_active_from_inputs(
            AgentThreadActivityInputs {
                manager_available: true,
                thread_found: true,
                active_event_subscription_count: 1,
                status: Some(AgentStatus::Completed(None)),
                ..AgentThreadActivityInputs::default()
            }
        ));
        assert!(!agent_thread_is_active_from_inputs(
            AgentThreadActivityInputs {
                manager_available: true,
                thread_found: true,
                has_active_turn: true,
                status: Some(AgentStatus::Completed(None)),
                ..AgentThreadActivityInputs::default()
            }
        ));
        assert!(agent_thread_is_active_from_inputs(
            AgentThreadActivityInputs {
                manager_available: true,
                thread_found: true,
                status: Some(AgentStatus::Running),
                ..AgentThreadActivityInputs::default()
            }
        ));
        assert!(!agent_thread_is_active_from_inputs(
            AgentThreadActivityInputs {
                manager_available: true,
                thread_found: true,
                status: Some(AgentStatus::Completed(None)),
                ..AgentThreadActivityInputs::default()
            }
        ));
    }

    #[test]
    fn normalized_thread_lifecycle_projects_runtime_facts() {
        let lifecycle = normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
            manager_available: true,
            thread_found: true,
            live_agent_status: Some(AgentStatus::PendingInit),
            ..ThreadLifecycleInputs::default()
        });
        assert_eq!(lifecycle, ThreadLifecycleStatus::Initializing);
        assert!(thread_lifecycle_is_active(&lifecycle));

        let lifecycle = normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
            manager_available: true,
            thread_found: true,
            live_agent_status: Some(AgentStatus::PendingInit),
            persisted_final_agent_status: Some(AgentStatus::Completed(Some("done".to_string()))),
            ..ThreadLifecycleInputs::default()
        });
        assert_eq!(
            lifecycle,
            ThreadLifecycleStatus::completed(Some("done".to_string()))
        );
        assert!(!thread_lifecycle_is_active(&lifecycle));

        let lifecycle = normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
            manager_available: true,
            persisted_final_agent_status: Some(AgentStatus::Shutdown),
            ..ThreadLifecycleInputs::default()
        });
        assert_eq!(
            lifecycle,
            ThreadLifecycleStatus::Final {
                result: ThreadLifecycleFinalStatus::Shutdown
            }
        );
        assert!(!thread_lifecycle_is_active(&lifecycle));

        let lifecycle = normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
            manager_available: true,
            thread_found: true,
            active_event_subscription_count: 1,
            has_active_turn: true,
            live_agent_status: Some(AgentStatus::Interrupted),
            ..ThreadLifecycleInputs::default()
        });
        assert_eq!(
            lifecycle,
            ThreadLifecycleStatus::Final {
                result: ThreadLifecycleFinalStatus::Interrupted
            }
        );
        assert!(!thread_lifecycle_is_active(&lifecycle));

        let lifecycle = normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
            manager_available: true,
            thread_found: true,
            active_event_subscription_count: 1,
            live_agent_status: Some(AgentStatus::Running),
            ..ThreadLifecycleInputs::default()
        });
        assert_eq!(
            lifecycle,
            ThreadLifecycleStatus::Waiting {
                reason: ThreadLifecycleWaitReason::EventSubscription
            }
        );
        assert!(thread_lifecycle_is_active(&lifecycle));

        let lifecycle = normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
            manager_available: false,
            thread_found: true,
            live_agent_status: Some(AgentStatus::Running),
            ..ThreadLifecycleInputs::default()
        });
        assert_eq!(
            lifecycle,
            ThreadLifecycleStatus::SystemError {
                message: Some("thread manager unavailable".to_string())
            }
        );
        assert!(!thread_lifecycle_is_active(&lifecycle));
    }

    #[test]
    fn any_agent_thread_active_returns_true_for_any_active_flag() {
        assert!(any_agent_thread_active([false, true, false]));
        assert!(!any_agent_thread_active([false, false]));
    }

    #[test]
    fn render_input_preview_joins_text_items() {
        let op = Op::UserInput {
            items: vec![
                UserInput::Text {
                    text: "first".to_string(),
                    text_elements: Vec::new(),
                },
                UserInput::Image {
                    image_url: "https://example.com/image.png".to_string(),
                },
                UserInput::Text {
                    text: "second".to_string(),
                    text_elements: Vec::new(),
                },
            ],
            environments: None,
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        };

        assert_eq!(render_input_preview(&op), "first\n[image]\nsecond");
    }

    #[test]
    fn render_input_preview_ignores_non_message_ops() {
        let op = Op::Interrupt;

        assert_eq!(render_input_preview(&op), "");
    }

    fn fixed_thread_id(value: u128) -> ThreadId {
        ThreadId::from_string(&format!("{value:032x}")).expect("thread id")
    }

    fn metadata_with_path(agent_path: AgentPath) -> AgentMetadata {
        AgentMetadata {
            agent_path: Some(agent_path),
            ..Default::default()
        }
    }
}
