use codex_context_manager::EnvironmentContext;
use codex_context_manager::EnvironmentContextEnvironment;
use codex_context_manager::NetworkContext;

use crate::runtime_shell_model::Shell;
use crate::session::turn_context::TurnContext;
use crate::session::turn_context::TurnEnvironment;

pub(crate) fn environment_context_from_turn_context(
    turn_context: &TurnContext,
    shell: &Shell,
) -> EnvironmentContext {
    EnvironmentContext::new(
        environment_context_environments_from_turn_context(
            &turn_context.environments.turn_environments,
            shell,
        ),
        turn_context.current_date.clone(),
        turn_context.timezone.clone(),
        network_context_from_turn_context(turn_context),
    )
}

fn environment_context_environments_from_turn_context(
    environments: &[TurnEnvironment],
    shell: &Shell,
) -> Vec<EnvironmentContextEnvironment> {
    environments
        .iter()
        .map(|environment| {
            EnvironmentContextEnvironment::new(
                environment.environment_id.clone(),
                environment.cwd.clone(),
                environment
                    .shell
                    .clone()
                    .unwrap_or_else(|| shell.name().to_string()),
            )
        })
        .collect()
}

fn network_context_from_turn_context(turn_context: &TurnContext) -> Option<NetworkContext> {
    let network = turn_context
        .config
        .config_layer_stack
        .requirements()
        .network
        .as_ref()?;

    Some(NetworkContext::new(
        network
            .domains
            .as_ref()
            .and_then(config_service::NetworkDomainPermissionsToml::allowed_domains)
            .unwrap_or_default(),
        network
            .domains
            .as_ref()
            .and_then(config_service::NetworkDomainPermissionsToml::denied_domains)
            .unwrap_or_default(),
    ))
}
