#[cfg(test)]
use std::future::Future;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::pin::Pin;

use codex_config_state::ConfigLayerEntry;
#[cfg(test)]
use codex_config_state::ConfigLayerStack;
use codex_config_state::ConfigLayerStackOrdering;
#[cfg(test)]
use codex_execpolicy_api::AmendError;
#[cfg(test)]
use codex_execpolicy_api::Decision;
#[cfg(test)]
pub(crate) use codex_execpolicy_api::Evaluation;
#[cfg(test)]
use codex_execpolicy_api::MatchOptions;
#[cfg(test)]
use codex_execpolicy_api::Policy;
#[cfg(test)]
pub(crate) use codex_execpolicy_api::RuleMatch;
pub use codex_permissions_runtime::EmptyExecPolicyLoader;
pub(crate) use codex_permissions_runtime::ExecPolicyApprovalRequest as ExecApprovalRequest;
#[cfg(test)]
pub(crate) use codex_permissions_runtime::ExecPolicyCommandOrigin;
#[cfg(test)]
pub(crate) use codex_permissions_runtime::ExecPolicyCommands;
pub use codex_permissions_runtime::ExecPolicyLoadResult;
pub use codex_permissions_runtime::ExecPolicyLoader;
pub(crate) use codex_permissions_runtime::ExecPolicyManager;
pub(crate) use codex_permissions_runtime::ExecPolicyUpdateError;
#[cfg(test)]
pub(crate) use codex_permissions_runtime::UnmatchedCommandContext;
#[cfg(test)]
pub(crate) use codex_permissions_runtime::commands_for_exec_policy;
#[cfg(test)]
use codex_permissions_runtime::create_exec_approval_requirement_for_command;
#[cfg(test)]
pub(crate) use codex_permissions_runtime::default_policy_path;
#[cfg(test)]
pub(crate) use codex_permissions_runtime::derive_requested_execpolicy_amendment_from_prefix_rule;
#[cfg(test)]
use codex_permissions_runtime::is_policy_match;
#[cfg(test)]
pub(crate) use codex_permissions_runtime::profile_is_managed_read_only;
#[cfg(test)]
pub(crate) use codex_permissions_runtime::render_decision_for_unmatched_command;
#[cfg(test)]
use codex_protocol::approvals::ExecPolicyAmendment;
#[cfg(test)]
use codex_protocol::models::SandboxPermissions;
#[cfg(test)]
pub(crate) use codex_protocol::permissions::FileSystemSandboxPolicy;

use crate::config::Config;
#[cfg(test)]
use crate::tools::sandboxing::ExecApprovalRequirement;
use codex_utils_absolute_path::AbsolutePathBuf;

#[cfg(test)]
const RULES_DIR_NAME: &str = "rules";
#[cfg(test)]
const PROMPT_CONFLICT_REASON: &str =
    "approval required by policy, but AskForApproval is set to Never";
#[cfg(test)]
const REJECT_SANDBOX_APPROVAL_REASON: &str =
    "approval required by policy, but AskForApproval::Granular.sandbox_approval is false";
#[cfg(test)]
const REJECT_RULES_APPROVAL_REASON: &str =
    "approval required by policy rule, but AskForApproval::Granular.rules is false";

pub(crate) fn child_uses_parent_exec_policy(parent_config: &Config, child_config: &Config) -> bool {
    fn exec_policy_config_folders(config: &Config) -> Vec<AbsolutePathBuf> {
        config
            .config_layer_stack
            .get_layers(
                ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ false,
            )
            .into_iter()
            .filter_map(ConfigLayerEntry::config_folder)
            .collect()
    }

    exec_policy_config_folders(parent_config) == exec_policy_config_folders(child_config)
        && parent_config
            .config_layer_stack
            .ignore_user_and_project_exec_policy_rules()
            == child_config
                .config_layer_stack
                .ignore_user_and_project_exec_policy_rules()
        && parent_config.config_layer_stack.requirements().exec_policy
            == child_config.config_layer_stack.requirements().exec_policy
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) enum ExecPolicyError {
    ReadDir {
        dir: PathBuf,
        source: std::io::Error,
    },

    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    ParsePolicy {
        path: String,
        source: codex_execpolicy::Error,
    },
}

#[cfg(test)]
impl std::fmt::Display for ExecPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadDir { dir, source } => {
                write!(
                    f,
                    "failed to read rules files from {}: {source}",
                    dir.display()
                )
            }
            Self::ReadFile { path, source } => {
                write!(f, "failed to read rules file {}: {source}", path.display())
            }
            Self::ParsePolicy { path, source } => {
                write!(f, "failed to parse rules file {path}: {source}")
            }
        }
    }
}

#[cfg(test)]
impl std::error::Error for ExecPolicyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReadDir { source, .. } => Some(source),
            Self::ReadFile { source, .. } => Some(source),
            Self::ParsePolicy { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
pub(crate) struct TestExecPolicyLoader;

#[cfg(test)]
impl ExecPolicyLoader for TestExecPolicyLoader {
    fn load_exec_policy<'a>(
        &'a self,
        config_stack: &'a ConfigLayerStack,
    ) -> Pin<Box<dyn Future<Output = Result<ExecPolicyLoadResult, String>> + Send + 'a>> {
        Box::pin(async move {
            let (policy, warning) = load_exec_policy_with_warning(config_stack)
                .await
                .map_err(|err| err.to_string())?;
            Ok(ExecPolicyLoadResult {
                policy,
                warning: warning.as_ref().map(format_exec_policy_error_with_source),
            })
        })
    }
}

#[cfg(test)]
pub(crate) async fn check_execpolicy_for_warnings(
    config_stack: &ConfigLayerStack,
) -> Result<Option<ExecPolicyError>, ExecPolicyError> {
    let (_, warning) = load_exec_policy_with_warning(config_stack).await?;
    Ok(warning)
}

#[cfg(test)]
fn exec_policy_message_for_display(source: &codex_execpolicy::Error) -> String {
    let message = source.to_string();
    if let Some(line) = message
        .lines()
        .find(|line| line.trim_start().starts_with("error: "))
    {
        return line.to_owned();
    }
    if let Some(first_line) = message.lines().next()
        && let Some((_, detail)) = first_line.rsplit_once(": starlark error: ")
    {
        return detail.trim().to_string();
    }

    message
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(test)]
fn parse_starlark_line_from_message(message: &str) -> Option<(PathBuf, usize)> {
    let first_line = message.lines().next()?.trim();
    let (path_and_position, _) = first_line.rsplit_once(": starlark error:")?;

    let mut parts = path_and_position.rsplitn(3, ':');
    let _column = parts.next()?.parse::<usize>().ok()?;
    let line = parts.next()?.parse::<usize>().ok()?;
    let path = PathBuf::from(parts.next()?);

    if line == 0 {
        return None;
    }

    Some((path, line))
}

#[cfg(test)]
pub(crate) fn format_exec_policy_error_with_source(error: &ExecPolicyError) -> String {
    match error {
        ExecPolicyError::ParsePolicy { path, source } => {
            let rendered_source = source.to_string();
            let structured_location = source
                .location()
                .map(|location| (PathBuf::from(location.path), location.range.start.line));
            let parsed_location = parse_starlark_line_from_message(&rendered_source);
            let location = match (structured_location, parsed_location) {
                (Some((_, 1)), Some((parsed_path, parsed_line))) if parsed_line > 1 => {
                    Some((parsed_path, parsed_line))
                }
                (Some(structured), _) => Some(structured),
                (None, parsed) => parsed,
            };
            let message = exec_policy_message_for_display(source);
            match location {
                Some((path, line)) => {
                    format!(
                        "{}:{}: {} (problem is on or around line {})",
                        path.display(),
                        line,
                        message,
                        line
                    )
                }
                None => format!("{path}: {message}"),
            }
        }
        _ => error.to_string(),
    }
}

#[cfg(test)]
async fn load_exec_policy_with_warning(
    config_stack: &ConfigLayerStack,
) -> Result<(Policy, Option<ExecPolicyError>), ExecPolicyError> {
    match load_exec_policy(config_stack).await {
        Ok(policy) => Ok((policy, None)),
        Err(err @ ExecPolicyError::ParsePolicy { .. }) => Ok((Policy::empty(), Some(err))),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
pub(crate) async fn load_exec_policy(
    config_stack: &ConfigLayerStack,
) -> Result<Policy, ExecPolicyError> {
    let mut policy_paths = Vec::new();
    for layer in config_stack.get_layers(
        ConfigLayerStackOrdering::LowestPrecedenceFirst,
        /*include_disabled*/ false,
    ) {
        if config_stack.ignore_user_and_project_exec_policy_rules()
            && matches!(
                layer.name,
                codex_config_types::ConfigLayerSource::User { .. }
                    | codex_config_types::ConfigLayerSource::Project { .. }
            )
        {
            continue;
        }
        if let Some(config_folder) = layer.config_folder() {
            let policy_dir = config_folder.join(RULES_DIR_NAME);
            let layer_policy_paths = collect_policy_files(&policy_dir).await?;
            policy_paths.extend(layer_policy_paths);
        }
    }

    let mut parser = codex_execpolicy::PolicyParser::new();
    for policy_path in &policy_paths {
        let contents = tokio::fs::read_to_string(policy_path)
            .await
            .map_err(|source| ExecPolicyError::ReadFile {
                path: policy_path.clone(),
                source,
            })?;
        let identifier = policy_path.to_string_lossy().to_string();
        parser
            .parse(&identifier, &contents)
            .map_err(|source| ExecPolicyError::ParsePolicy {
                path: identifier,
                source,
            })?;
    }

    let policy = parser.build();
    let Some(requirements_policy) = config_stack.requirements().exec_policy.as_deref() else {
        return Ok(policy);
    };

    Ok(policy.merge_overlay(requirements_policy.as_ref()))
}

#[cfg(test)]
async fn collect_policy_files(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>, ExecPolicyError> {
    let dir = dir.as_ref();
    let mut read_dir = match tokio::fs::read_dir(dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(ExecPolicyError::ReadDir {
                dir: dir.to_path_buf(),
                source,
            });
        }
    };

    let mut policy_paths = Vec::new();
    while let Some(entry) =
        read_dir
            .next_entry()
            .await
            .map_err(|source| ExecPolicyError::ReadDir {
                dir: dir.to_path_buf(),
                source,
            })?
    {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .await
            .map_err(|source| ExecPolicyError::ReadDir {
                dir: dir.to_path_buf(),
                source,
            })?;

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "rules")
            && file_type.is_file()
        {
            policy_paths.push(path);
        }
    }

    policy_paths.sort();
    Ok(policy_paths)
}

#[cfg(test)]
#[path = "exec_policy_tests.rs"]
mod tests;
