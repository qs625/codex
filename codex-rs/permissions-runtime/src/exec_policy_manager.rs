use codex_config_state::ConfigLayerStack;
use codex_execpolicy_api::AmendError;
use codex_execpolicy_api::Decision;
use codex_execpolicy_api::Error as ExecPolicyRuleError;
use codex_execpolicy_api::MatchOptions;
use codex_execpolicy_api::NetworkRuleProtocol;
use codex_execpolicy_api::Policy;
use codex_execpolicy_api::blocking_append_allow_prefix_rule;
use codex_execpolicy_api::blocking_append_network_rule;
use codex_protocol::approvals::ExecPolicyAmendment;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use tokio::sync::Semaphore;
use tokio::task::spawn_blocking;
use tracing::instrument;

use crate::ExecApprovalRequirement;
use crate::ExecPolicyApprovalRequest;
use crate::create_exec_approval_requirement_for_command;
use crate::is_policy_match;

const RULES_DIR_NAME: &str = "rules";
const DEFAULT_POLICY_FILE: &str = "default.rules";

pub struct ExecPolicyLoadResult {
    pub policy: Policy,
    pub warning: Option<String>,
}

/// Host-provided loader for Starlark-backed exec-policy rules.
///
/// Implementations read configured rule files and merge requirements policy into
/// the returned [`Policy`]. The runtime manager owns policy evaluation and
/// amendment updates, while parser implementations live in composition-root
/// crates so dependents do not need to pull in Starlark.
pub trait ExecPolicyLoader: Send + Sync {
    fn load_exec_policy<'a>(
        &'a self,
        config_stack: &'a ConfigLayerStack,
    ) -> Pin<Box<dyn Future<Output = Result<ExecPolicyLoadResult, String>> + Send + 'a>>;
}

pub struct EmptyExecPolicyLoader;

impl ExecPolicyLoader for EmptyExecPolicyLoader {
    fn load_exec_policy<'a>(
        &'a self,
        _config_stack: &'a ConfigLayerStack,
    ) -> Pin<Box<dyn Future<Output = Result<ExecPolicyLoadResult, String>> + Send + 'a>> {
        Box::pin(async {
            Ok(ExecPolicyLoadResult {
                policy: Policy::empty(),
                warning: None,
            })
        })
    }
}

#[derive(Debug)]
pub enum ExecPolicyUpdateError {
    AppendRule { path: PathBuf, source: AmendError },

    JoinBlockingTask { source: tokio::task::JoinError },

    AddRule { source: ExecPolicyRuleError },
}

impl std::fmt::Display for ExecPolicyUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AppendRule { path, source } => {
                write!(
                    f,
                    "failed to update rules file {}: {source}",
                    path.display()
                )
            }
            Self::JoinBlockingTask { source } => {
                write!(f, "failed to join blocking rules update task: {source}")
            }
            Self::AddRule { source } => write!(f, "failed to update in-memory rules: {source}"),
        }
    }
}

impl std::error::Error for ExecPolicyUpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AppendRule { source, .. } => Some(source),
            Self::JoinBlockingTask { source } => Some(source),
            Self::AddRule { source } => Some(source),
        }
    }
}

impl From<ExecPolicyRuleError> for ExecPolicyUpdateError {
    fn from(source: ExecPolicyRuleError) -> Self {
        Self::AddRule { source }
    }
}

pub struct ExecPolicyManager {
    policy: StdRwLock<Arc<Policy>>,
    update_lock: Semaphore,
}

impl ExecPolicyManager {
    pub fn new(policy: Arc<Policy>) -> Self {
        Self {
            policy: StdRwLock::new(policy),
            update_lock: Semaphore::new(/*permits*/ 1),
        }
    }

    #[instrument(level = "info", skip_all)]
    pub async fn load(
        config_stack: &ConfigLayerStack,
        loader: &dyn ExecPolicyLoader,
    ) -> Result<Self, String> {
        let result = loader.load_exec_policy(config_stack).await?;
        if let Some(warning) = result.warning.as_ref() {
            tracing::warn!("failed to parse rules: {warning}");
        }
        Ok(Self::new(Arc::new(result.policy)))
    }

    pub fn current(&self) -> Arc<Policy> {
        Arc::clone(
            &self
                .policy
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    pub async fn create_exec_approval_requirement_for_command(
        &self,
        req: ExecPolicyApprovalRequest<'_>,
    ) -> ExecApprovalRequirement {
        let exec_policy = self.current();
        create_exec_approval_requirement_for_command(exec_policy.as_ref(), req)
    }

    pub async fn append_amendment_and_update(
        &self,
        codex_home: &Path,
        amendment: &ExecPolicyAmendment,
    ) -> Result<(), ExecPolicyUpdateError> {
        let _update_guard =
            self.update_lock
                .acquire()
                .await
                .map_err(|_| ExecPolicyUpdateError::AddRule {
                    source: ExecPolicyRuleError::InvalidRule(
                        "exec policy update semaphore closed".to_string(),
                    ),
                })?;
        let policy_path = default_policy_path(codex_home);
        spawn_blocking({
            let policy_path = policy_path.clone();
            let prefix = amendment.command.clone();
            move || blocking_append_allow_prefix_rule(&policy_path, &prefix)
        })
        .await
        .map_err(|source| ExecPolicyUpdateError::JoinBlockingTask { source })?
        .map_err(|source| ExecPolicyUpdateError::AppendRule {
            path: policy_path,
            source,
        })?;

        let current_policy = self.current();
        let match_options = MatchOptions {
            resolve_host_executables: true,
        };
        let existing_evaluation = current_policy.check_multiple_with_options(
            [&amendment.command],
            &|_| Decision::Forbidden,
            &match_options,
        );
        let already_allowed = existing_evaluation.decision == Decision::Allow
            && existing_evaluation.matched_rules.iter().any(|rule_match| {
                is_policy_match(rule_match) && rule_match.decision() == Decision::Allow
            });
        if already_allowed {
            return Ok(());
        }

        let mut updated_policy = current_policy.as_ref().clone();
        updated_policy
            .add_prefix_rule(&amendment.command, Decision::Allow)
            .map_err(ExecPolicyRuleError::from)?;
        *self
            .policy
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(updated_policy);
        Ok(())
    }

    pub async fn append_network_rule_and_update(
        &self,
        codex_home: &Path,
        host: &str,
        protocol: NetworkRuleProtocol,
        decision: Decision,
        justification: Option<String>,
    ) -> Result<(), ExecPolicyUpdateError> {
        let _update_guard =
            self.update_lock
                .acquire()
                .await
                .map_err(|_| ExecPolicyUpdateError::AddRule {
                    source: ExecPolicyRuleError::InvalidRule(
                        "exec policy update semaphore closed".to_string(),
                    ),
                })?;
        let policy_path = default_policy_path(codex_home);
        let host = host.to_string();
        spawn_blocking({
            let policy_path = policy_path.clone();
            let host = host.clone();
            let justification = justification.clone();
            move || {
                blocking_append_network_rule(
                    &policy_path,
                    &host,
                    protocol,
                    decision,
                    justification.as_deref(),
                )
            }
        })
        .await
        .map_err(|source| ExecPolicyUpdateError::JoinBlockingTask { source })?
        .map_err(|source| ExecPolicyUpdateError::AppendRule {
            path: policy_path,
            source,
        })?;

        let mut updated_policy = self.current().as_ref().clone();
        updated_policy
            .add_network_rule(&host, protocol, decision, justification)
            .map_err(ExecPolicyRuleError::from)?;
        *self
            .policy
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(updated_policy);
        Ok(())
    }
}

impl Default for ExecPolicyManager {
    fn default() -> Self {
        Self::new(Arc::new(Policy::empty()))
    }
}

#[doc(hidden)]
pub fn default_policy_path(codex_home: &Path) -> PathBuf {
    codex_home.join(RULES_DIR_NAME).join(DEFAULT_POLICY_FILE)
}
