use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use serde::Deserialize;
#[cfg(test)]
use serde_json::Map;
use serde_json::Value;

use crate::function_tool::FunctionCallError;
#[cfg(test)]
use crate::sandboxing::SandboxPermissions;
#[cfg(test)]
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::session::turn_context::TurnEnvironment;
pub(crate) type CoreToolDomainHost = crate::apply_patch_tool_host::CoreApplyPatchHandlerHost;
#[cfg(test)]
use codex_protocol::models::AdditionalPermissionProfile;
#[cfg(test)]
pub(super) use codex_tool_handlers::EffectiveAdditionalPermissions;
#[cfg(test)]
pub(super) use codex_tool_handlers::implicit_granted_permissions;
#[cfg(test)]
pub(crate) use codex_tool_handlers::normalize_and_validate_additional_permissions;

pub(crate) fn core_tool_domain_host() -> CoreToolDomainHost {
    fn assert_tool_domain_host<Host: codex_tool_runtime_api::ToolDomainHost>(host: Host) -> Host {
        host
    }

    assert_tool_domain_host(crate::apply_patch_tool_host::CoreApplyPatchHandlerHost)
}

pub(crate) fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

#[cfg(test)]
fn updated_hook_command(updated_input: &Value) -> Result<&str, FunctionCallError> {
    updated_input
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "hook returned updatedInput without string field `command`".to_string(),
            )
        })
}

#[cfg(test)]
fn rewrite_function_arguments(
    arguments: &str,
    tool_name: &str,
    rewrite: impl FnOnce(&mut Map<String, Value>),
) -> Result<String, FunctionCallError> {
    let mut arguments: Value = parse_arguments(arguments)?;
    let Value::Object(arguments) = &mut arguments else {
        return Err(FunctionCallError::RespondToModel(format!(
            "{tool_name} arguments must be an object"
        )));
    };
    rewrite(arguments);
    serde_json::to_string(&arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to serialize rewritten {tool_name} arguments: {err}"
        ))
    })
}

#[cfg(test)]
fn rewrite_function_string_argument(
    arguments: &str,
    tool_name: &str,
    field_name: &str,
    value: &str,
) -> Result<String, FunctionCallError> {
    rewrite_function_arguments(arguments, tool_name, |arguments| {
        arguments.insert(field_name.to_string(), Value::String(value.to_string()));
    })
}

pub(crate) fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: &AbsolutePathBuf,
) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let _guard = AbsolutePathBufGuard::new(base_path);
    parse_arguments(arguments)
}

pub(crate) fn resolve_workdir_base_path(
    arguments: &str,
    default_cwd: &AbsolutePathBuf,
) -> Result<AbsolutePathBuf, FunctionCallError> {
    let arguments: Value = parse_arguments(arguments)?;
    Ok(arguments
        .get("workdir")
        .and_then(Value::as_str)
        .filter(|workdir| !workdir.is_empty())
        .map_or_else(|| default_cwd.clone(), |workdir| default_cwd.join(workdir)))
}

pub(crate) fn resolve_tool_environment<'a>(
    turn: &'a TurnContext,
    environment_id: Option<&str>,
) -> Result<Option<&'a TurnEnvironment>, FunctionCallError> {
    environment_id.map_or_else(
        || Ok(turn.environments.primary()),
        |environment_id| {
            turn.environments
                .turn_environments
                .iter()
                .find(|environment| environment.environment_id == environment_id)
                .map(Some)
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(format!(
                        "unknown turn environment id `{environment_id}`"
                    ))
                })
        },
    )
}

#[cfg(test)]
pub(super) async fn apply_granted_turn_permissions(
    session: &Session,
    cwd: &std::path::Path,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<AdditionalPermissionProfile>,
) -> EffectiveAdditionalPermissions {
    codex_tool_handlers::apply_granted_permissions_from_grants(
        codex_tool_runtime_api::ToolPermissionGrants {
            session: session.granted_session_permissions().await,
            turn: session.granted_turn_permissions().await,
        },
        cwd,
        sandbox_permissions,
        additional_permissions,
    )
}

#[cfg(test)]
mod tests {
    use super::EffectiveAdditionalPermissions;
    use super::implicit_granted_permissions;
    use super::normalize_and_validate_additional_permissions;
    use crate::sandboxing::SandboxPermissions;
    use codex_protocol::models::AdditionalPermissionProfile;
    use codex_protocol::models::FileSystemPermissions;
    use codex_protocol::models::NetworkPermissions;
    use codex_protocol::permissions::FileSystemAccessMode;
    use codex_protocol::permissions::FileSystemPath;
    use codex_protocol::permissions::FileSystemSandboxEntry;
    use codex_protocol::permissions::FileSystemSpecialPath;
    use codex_protocol::protocol::AskForApproval;
    use codex_protocol::protocol::GranularApprovalConfig;
    use codex_sandboxing_api::policy_transforms::intersect_permission_profiles;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    fn network_permissions() -> AdditionalPermissionProfile {
        AdditionalPermissionProfile {
            network: Some(NetworkPermissions {
                enabled: Some(true),
            }),
            ..Default::default()
        }
    }

    fn file_system_permissions(path: &std::path::Path) -> AdditionalPermissionProfile {
        AdditionalPermissionProfile {
            file_system: Some(FileSystemPermissions::from_read_write_roots(
                /*read*/ None,
                Some(vec![
                    AbsolutePathBuf::from_absolute_path(path).expect("absolute path"),
                ]),
            )),
            ..Default::default()
        }
    }

    #[test]
    fn preapproved_permissions_work_when_request_permissions_tool_is_enabled_without_exec_permission_approvals_feature()
     {
        let cwd = tempdir().expect("tempdir");

        let normalized = normalize_and_validate_additional_permissions(
            /*additional_permissions_allowed*/ false,
            AskForApproval::Granular(GranularApprovalConfig {
                sandbox_approval: true,
                rules: true,
                skill_approval: true,
                request_permissions: false,
                mcp_elicitations: true,
            }),
            SandboxPermissions::WithAdditionalPermissions,
            Some(network_permissions()),
            /*permissions_preapproved*/ true,
            cwd.path(),
        )
        .expect("preapproved permissions should be allowed");

        assert_eq!(normalized, Some(network_permissions()));
    }

    #[test]
    fn fresh_additional_permissions_still_require_exec_permission_approvals_feature() {
        let cwd = tempdir().expect("tempdir");

        let err = normalize_and_validate_additional_permissions(
            /*additional_permissions_allowed*/ false,
            AskForApproval::OnRequest,
            SandboxPermissions::WithAdditionalPermissions,
            Some(network_permissions()),
            /*permissions_preapproved*/ false,
            cwd.path(),
        )
        .expect_err("fresh inline permission requests should remain disabled");

        assert_eq!(
            err,
            "additional permissions are disabled; enable `features.exec_permission_approvals` before using `with_additional_permissions`"
        );
    }

    #[test]
    fn implicit_sticky_grants_bypass_inline_permission_validation() {
        let cwd = tempdir().expect("tempdir");
        let granted_permissions = file_system_permissions(cwd.path());
        let implicit_permissions = implicit_granted_permissions(
            SandboxPermissions::UseDefault,
            /*additional_permissions*/ None,
            &EffectiveAdditionalPermissions {
                sandbox_permissions: SandboxPermissions::WithAdditionalPermissions,
                additional_permissions: Some(granted_permissions.clone()),
                permissions_preapproved: false,
            },
        );

        assert_eq!(implicit_permissions, Some(granted_permissions));
    }

    #[test]
    fn explicit_inline_permissions_do_not_use_implicit_sticky_grant_path() {
        let cwd = tempdir().expect("tempdir");
        let requested_permissions = file_system_permissions(cwd.path());
        let implicit_permissions = implicit_granted_permissions(
            SandboxPermissions::WithAdditionalPermissions,
            Some(&requested_permissions),
            &EffectiveAdditionalPermissions {
                sandbox_permissions: SandboxPermissions::WithAdditionalPermissions,
                additional_permissions: Some(requested_permissions.clone()),
                permissions_preapproved: false,
            },
        );

        assert_eq!(implicit_permissions, None);
    }

    #[test]
    fn relative_deny_glob_grants_remain_preapproved_after_materialization() {
        let cwd = tempdir().expect("tempdir");
        let requested_permissions = AdditionalPermissionProfile {
            file_system: Some(FileSystemPermissions {
                entries: vec![
                    FileSystemSandboxEntry {
                        path: FileSystemPath::Special {
                            value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                        },
                        access: FileSystemAccessMode::Write,
                    },
                    FileSystemSandboxEntry {
                        path: FileSystemPath::GlobPattern {
                            pattern: "**/*.env".to_string(),
                        },
                        access: FileSystemAccessMode::None,
                    },
                ],
                glob_scan_max_depth: None,
            }),
            ..Default::default()
        };
        let stored_grant = intersect_permission_profiles(
            requested_permissions.clone(),
            requested_permissions.clone(),
            cwd.path(),
        );
        let effective_permissions = codex_tool_handlers::apply_granted_permissions_from_grants(
            codex_tool_runtime_api::ToolPermissionGrants {
                session: None,
                turn: Some(stored_grant),
            },
            cwd.path(),
            SandboxPermissions::UseDefault,
            Some(requested_permissions),
        );

        assert!(effective_permissions.permissions_preapproved);
    }
}
