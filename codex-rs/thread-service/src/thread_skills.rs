use protocol::protocol::EventMsg;
use protocol::protocol::InitialHistory;
use protocol::protocol::RolloutItem;
use protocol::protocol::ThreadSkill;

pub fn merge_thread_skills(
    mut current: Vec<ThreadSkill>,
    additions: Vec<ThreadSkill>,
) -> Option<Vec<ThreadSkill>> {
    let mut changed = false;
    for addition in additions {
        if let Some(existing) = current.iter_mut().find(|skill| skill.path == addition.path) {
            let merged_kind = existing.kind.merge(addition.kind);
            if existing.kind != merged_kind {
                existing.kind = merged_kind;
                changed = true;
            }
            if existing.name != addition.name {
                existing.name = addition.name;
                changed = true;
            }
        } else {
            current.push(addition);
            changed = true;
        }
    }

    changed.then_some(current)
}

pub fn initial_thread_skills(initial_history: &InitialHistory) -> Vec<ThreadSkill> {
    initial_history
        .get_rollout_items()
        .into_iter()
        .rev()
        .find_map(|item| match item {
            RolloutItem::EventMsg(EventMsg::ThreadSkillsUpdated(event)) => Some(event.skills),
            RolloutItem::SessionMeta(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::ResponseItem(_)
            | RolloutItem::Compacted(_)
            | RolloutItem::EventMsg(_) => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::ThreadId;
    use protocol::protocol::ResumedHistory;
    use protocol::protocol::ThreadSkillKind;
    use protocol::protocol::ThreadSkillsUpdatedEvent;

    fn thread_skill(name: &str, path: &str, kind: ThreadSkillKind) -> ThreadSkill {
        ThreadSkill {
            name: name.to_string(),
            path: path.to_string(),
            kind,
        }
    }

    #[test]
    fn merge_thread_skills_adds_new_skills() {
        let current = vec![thread_skill(
            "old",
            "/tmp/old/SKILL.md",
            ThreadSkillKind::Explicit,
        )];
        let addition = thread_skill("new", "/tmp/new/SKILL.md", ThreadSkillKind::Implicit);

        assert_eq!(
            merge_thread_skills(current.clone(), vec![addition.clone()]),
            Some(vec![current[0].clone(), addition])
        );
    }

    #[test]
    fn merge_thread_skills_updates_name_and_kind_by_path() {
        let current = vec![thread_skill(
            "old-name",
            "/tmp/shared/SKILL.md",
            ThreadSkillKind::Explicit,
        )];
        let addition = thread_skill(
            "new-name",
            "/tmp/shared/SKILL.md",
            ThreadSkillKind::Implicit,
        );

        assert_eq!(
            merge_thread_skills(current, vec![addition]),
            Some(vec![thread_skill(
                "new-name",
                "/tmp/shared/SKILL.md",
                ThreadSkillKind::All
            )])
        );
    }

    #[test]
    fn merge_thread_skills_returns_none_without_changes() {
        let current = vec![thread_skill(
            "same",
            "/tmp/shared/SKILL.md",
            ThreadSkillKind::All,
        )];

        assert_eq!(merge_thread_skills(current.clone(), current), None);
    }

    #[test]
    fn initial_thread_skills_uses_latest_update() {
        let older = thread_skill("older", "/tmp/older/SKILL.md", ThreadSkillKind::Explicit);
        let newer = thread_skill("newer", "/tmp/newer/SKILL.md", ThreadSkillKind::All);
        let history = InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::new(),
            history: vec![
                RolloutItem::EventMsg(EventMsg::ThreadSkillsUpdated(ThreadSkillsUpdatedEvent {
                    skills: vec![older],
                })),
                RolloutItem::EventMsg(EventMsg::ThreadSkillsUpdated(ThreadSkillsUpdatedEvent {
                    skills: vec![newer.clone()],
                })),
            ],
            rollout_path: None,
        });

        assert_eq!(initial_thread_skills(&history), vec![newer]);
    }

    #[test]
    fn initial_thread_skills_defaults_to_empty_without_updates() {
        assert_eq!(
            initial_thread_skills(&InitialHistory::New),
            Vec::<ThreadSkill>::new()
        );
    }
}
