use std::future::Future;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;

use codex_config_state::ConfigLayerStack;
use codex_config_state::ConfigLayerStackOrdering;
use codex_config_types::ConfigLayerSource;
use codex_core::ExecPolicyLoadResult;
use codex_core::ExecPolicyLoader;
use codex_execpolicy::PolicyParser;
use codex_execpolicy_api::Policy;
use thiserror::Error;
use tokio::fs;

const RULES_DIR_NAME: &str = "rules";
const RULE_EXTENSION: &str = "rules";

#[derive(Debug, Error)]
pub enum ExecPolicyError {
    #[error("failed to read rules files from {dir}: {source}")]
    ReadDir {
        dir: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to read rules file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse rules file {path}: {source}")]
    ParsePolicy {
        path: String,
        source: codex_execpolicy::Error,
    },
}

pub struct StarlarkExecPolicyLoader;

impl ExecPolicyLoader for StarlarkExecPolicyLoader {
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

pub async fn check_execpolicy_for_warnings(
    config_stack: &ConfigLayerStack,
) -> Result<Option<ExecPolicyError>, ExecPolicyError> {
    let (_, warning) = load_exec_policy_with_warning(config_stack).await?;
    Ok(warning)
}

pub fn format_exec_policy_error_with_source(error: &ExecPolicyError) -> String {
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

async fn load_exec_policy_with_warning(
    config_stack: &ConfigLayerStack,
) -> Result<(Policy, Option<ExecPolicyError>), ExecPolicyError> {
    match load_exec_policy(config_stack).await {
        Ok(policy) => Ok((policy, None)),
        Err(err @ ExecPolicyError::ParsePolicy { .. }) => Ok((Policy::empty(), Some(err))),
        Err(err) => Err(err),
    }
}

pub async fn load_exec_policy(config_stack: &ConfigLayerStack) -> Result<Policy, ExecPolicyError> {
    let mut policy_paths = Vec::new();
    for layer in config_stack.get_layers(
        ConfigLayerStackOrdering::LowestPrecedenceFirst,
        /*include_disabled*/ false,
    ) {
        if config_stack.ignore_user_and_project_exec_policy_rules()
            && matches!(
                layer.name,
                ConfigLayerSource::User { .. } | ConfigLayerSource::Project { .. }
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
    tracing::trace!(policy_paths = ?policy_paths, "loaded exec policies");

    let mut parser = PolicyParser::new();
    for policy_path in &policy_paths {
        let contents =
            fs::read_to_string(policy_path)
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
    tracing::debug!("loaded rules from {} files", policy_paths.len());
    tracing::trace!(rules = ?policy, "exec policy rules loaded");

    let requirements = config_stack.requirements();
    let Some(requirements_policy) = requirements.exec_policy.as_deref() else {
        return Ok(policy);
    };

    Ok(policy.merge_overlay(requirements_policy.as_ref()))
}

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

async fn collect_policy_files(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>, ExecPolicyError> {
    let dir = dir.as_ref();
    let mut read_dir = match fs::read_dir(dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
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
            .is_some_and(|ext| ext == RULE_EXTENSION)
            && file_type.is_file()
        {
            policy_paths.push(path);
        }
    }

    policy_paths.sort();

    tracing::debug!(
        "loaded {} .rules files in {}",
        policy_paths.len(),
        dir.display()
    );

    Ok(policy_paths)
}
