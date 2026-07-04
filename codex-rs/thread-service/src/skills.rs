use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::sync::Arc;

use crate::config::Config;
use crate::config::skill_config_layer_stack_from_config_layer_stack;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_analytics_api::InvocationType;
use codex_analytics_api::SkillInvocation;
use codex_analytics_api::build_track_events_context;
use codex_utils_absolute_path::AbsolutePathBuf;
use plugin_service_api::PluginSkillRoot;
use protocol::protocol::EventMsg;
use protocol::protocol::SkillScope;
use protocol::protocol::ThreadSkill;
use protocol::protocol::ThreadSkillKind;
use protocol::protocol::ThreadSkillsUpdatedEvent;
use protocol::request_user_input::RequestUserInputArgs;
use protocol::request_user_input::RequestUserInputQuestion;
use protocol::request_user_input::RequestUserInputResponse;
use skill_service_api::SkillDependencyInfo;
use skill_service_api::SkillsLoadInput;
use skill_service_api::detect_implicit_skill_invocation_for_command;
use tracing::warn;

pub(crate) fn build_skill_service_input_from_config(
    config: &Config,
    effective_skill_roots: Vec<PluginSkillRoot>,
) -> SkillsLoadInput {
    SkillsLoadInput::new(
        config.cwd.clone(),
        effective_skill_roots,
        skill_config_layer_stack_from_config_layer_stack(&config.config_layer_stack),
        config.bundled_skills_enabled(),
    )
    .with_allowlist_patterns(config.agent_skill_patterns.clone())
}

pub(crate) async fn resolve_skill_dependencies_for_turn(
    sess: &Arc<Session>,
    turn_context: &Arc<TurnContext>,
    dependencies: &[SkillDependencyInfo],
) {
    if dependencies.is_empty() {
        return;
    }

    let existing_env = sess.dependency_env().await;
    let mut loaded_values = HashMap::new();
    let mut missing = Vec::new();
    let mut seen_names = HashSet::new();

    for dependency in dependencies {
        let name = dependency.name.clone();
        if !seen_names.insert(name.clone()) || existing_env.contains_key(&name) {
            continue;
        }
        match env::var(&name) {
            Ok(value) => {
                loaded_values.insert(name.clone(), value);
            }
            Err(env::VarError::NotPresent) => {
                missing.push(dependency.clone());
            }
            Err(err) => {
                warn!("failed to read env var {name}: {err}");
                missing.push(dependency.clone());
            }
        }
    }

    if !loaded_values.is_empty() {
        sess.set_dependency_env(loaded_values).await;
    }

    if !missing.is_empty() {
        request_skill_dependencies(sess, turn_context, &missing).await;
    }
}

async fn request_skill_dependencies(
    sess: &Arc<Session>,
    turn_context: &Arc<TurnContext>,
    dependencies: &[SkillDependencyInfo],
) {
    let questions = dependencies
        .iter()
        .map(|dependency| {
            let requirement = dependency.description.as_ref().map_or_else(
                || {
                    format!(
                        "The skill \"{}\" requires \"{}\" to be set.",
                        dependency.skill_name, dependency.name
                    )
                },
                |description| {
                    format!(
                        "The skill \"{}\" requires \"{}\" to be set ({}).",
                        dependency.skill_name, dependency.name, description
                    )
                },
            );
            RequestUserInputQuestion {
                id: dependency.name.clone(),
                header: "Skill requires environment variable".to_string(),
                question: format!(
                    "{requirement} This is an experimental internal feature. The value is stored in memory for this session only."
                ),
                is_other: false,
                is_secret: true,
                options: None,
            }
        })
        .collect::<Vec<_>>();
    if questions.is_empty() {
        return;
    }

    let response = sess
        .request_user_input(
            turn_context,
            format!("skill-deps-{}", turn_context.sub_id),
            RequestUserInputArgs { questions },
        )
        .await
        .unwrap_or_else(|| RequestUserInputResponse {
            answers: HashMap::new(),
        });
    if response.answers.is_empty() {
        return;
    }

    let mut values = HashMap::new();
    for (name, answer) in response.answers {
        let mut user_note = None;
        for entry in &answer.answers {
            if let Some(note) = entry.strip_prefix("user_note: ")
                && !note.trim().is_empty()
            {
                user_note = Some(note.trim().to_string());
            }
        }
        if let Some(value) = user_note {
            values.insert(name, value);
        }
    }
    if values.is_empty() {
        return;
    }

    sess.set_dependency_env(values).await;
}

pub(crate) async fn maybe_emit_implicit_skill_invocation(
    sess: &Session,
    turn_context: &TurnContext,
    command: &str,
    workdir: &AbsolutePathBuf,
) {
    let Some(candidate) = detect_implicit_skill_invocation_for_command(
        turn_context.turn_skills.outcome.as_ref(),
        command,
        workdir,
    ) else {
        return;
    };
    let invocation = SkillInvocation {
        skill_name: candidate.name,
        skill_scope: candidate.scope,
        skill_path: candidate.path_to_skills_md.to_path_buf(),
        plugin_id: candidate.plugin_id,
        invocation_type: InvocationType::Implicit,
    };
    let skill_scope = match invocation.skill_scope {
        SkillScope::User => "user",
        SkillScope::Repo => "repo",
        SkillScope::System => "system",
        SkillScope::Admin => "admin",
    };
    let skill_path = invocation.skill_path.to_string_lossy();
    let skill_name = invocation.skill_name.clone();
    let seen_key = format!("{skill_scope}:{skill_path}:{skill_name}");
    let inserted = {
        let mut seen_skills = turn_context
            .turn_skills
            .implicit_invocation_seen_skills
            .lock()
            .await;
        seen_skills.insert(seen_key)
    };
    if !inserted {
        return;
    }

    turn_context.session_telemetry.counter(
        "codex.skill.injected",
        /*inc*/ 1,
        &[
            ("status", "ok"),
            ("skill", skill_name.as_str()),
            ("invoke_type", "implicit"),
        ],
    );
    sess.services
        .analytics_events_client
        .track_skill_invocations(
            build_track_events_context(
                turn_context.model_info.slug.clone(),
                sess.conversation_id.to_string(),
                turn_context.sub_id.clone(),
            ),
            vec![invocation.clone()],
        );
    emit_thread_skills_update(sess, turn_context, &[invocation]).await;
}

pub(crate) async fn emit_thread_skills_update(
    sess: &Session,
    turn_context: &TurnContext,
    invocations: &[SkillInvocation],
) {
    if invocations.is_empty() {
        return;
    }

    let next_skills = sess
        .merge_thread_skills(
            invocations
                .iter()
                .map(|invocation| ThreadSkill {
                    name: invocation.skill_name.clone(),
                    path: invocation.skill_path.to_string_lossy().into_owned(),
                    kind: match invocation.invocation_type {
                        InvocationType::Explicit => ThreadSkillKind::Explicit,
                        InvocationType::Implicit => ThreadSkillKind::Implicit,
                    },
                })
                .collect(),
        )
        .await;

    let Some(skills) = next_skills else {
        return;
    };

    sess.send_event(
        turn_context,
        EventMsg::ThreadSkillsUpdated(ThreadSkillsUpdatedEvent { skills }),
    )
    .await;
}
