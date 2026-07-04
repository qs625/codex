use std::future::Future;
use std::pin::Pin;

use anyhow::anyhow;
use permissions_service_api::Decision;
use permissions_service_api::Policy;
use permissions_service_api::RuleMatch;
use permissions_service::InterceptedExecPolicyContext;
use permissions_service::evaluate_intercepted_exec_policy;
use protocol::error::CodexErr;
use protocol::error::SandboxErr;
use protocol::exec_output::ExecToolCallOutput;
use protocol::exec_output::StreamOutput;
use protocol::models::AdditionalPermissionProfile;
use protocol::models::PermissionProfile;
use protocol::models::SandboxPermissions;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::protocol::AskForApproval;
use protocol::protocol::NetworkPolicyRuleAction;
use protocol::protocol::ReviewDecision;
use protocol::approvals::EscalationPermissions;
use protocol::approvals::ResolvedPermissionProfile;
use codex_utils_absolute_path::AbsolutePathBuf;

use crate::SandboxType;
use crate::shell_escalation_stopwatch::Stopwatch;

const PROMPT_CONFLICT_REASON: &str =
    "approval required by policy, but AskForApproval is set to Never";
const REJECT_SANDBOX_APPROVAL_REASON: &str =
    "approval required by policy, but AskForApproval::Granular.sandbox_approval is false";
const REJECT_RULES_APPROVAL_REASON: &str =
    "approval required by policy rule, but AskForApproval::Granular.rules is false";
const LINUX_SIGSYS_CODE: i32 = 31;
const EXIT_CODE_SIGNAL_BASE: i32 = 128;

/// Whether an execve decision came from an explicit rule or from the fallback
/// sandbox heuristic.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionSource {
    PrefixRule,
    /// Often, this is `is_safe_command()`.
    UnmatchedCommandFallback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EscalationDecision {
    Run,
    Escalate(EscalationExecution),
    Deny { reason: Option<String> },
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EscalationExecution {
    Unsandboxed,
    TurnDefault,
    Permissions(EscalationPermissions),
}

impl EscalationDecision {
    pub fn run() -> Self {
        Self::Run
    }

    pub fn escalate(execution: EscalationExecution) -> Self {
        Self::Escalate(execution)
    }

    pub fn deny(reason: Option<String>) -> Self {
        Self::Deny { reason }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub output: String,
    pub duration: std::time::Duration,
    pub timed_out: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ParsedShellCommand {
    pub program: String,
    pub script: String,
    pub login: bool,
}

pub struct EscalationPromptRequest<'a> {
    pub program: &'a AbsolutePathBuf,
    pub argv: &'a [String],
    pub workdir: &'a AbsolutePathBuf,
    pub stopwatch: &'a Stopwatch,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
}

pub struct EscalationPromptDecision {
    pub decision: ReviewDecision,
    pub rejection_message: Option<String>,
}

pub type EscalationPromptFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<EscalationPromptDecision>> + Send + 'a>>;

/// Handles the interactive or automated prompt step required by a shell
/// escalation decision.
pub trait EscalationPromptHandler: Send + Sync {
    fn prompt<'a>(&'a self, request: EscalationPromptRequest<'a>) -> EscalationPromptFuture<'a>;

    fn timeout_message(&self) -> String;
}

pub struct EscalationPolicyDecisionParams<'a> {
    pub policy: &'a Policy,
    pub approval_policy: AskForApproval,
    pub permission_profile: &'a PermissionProfile,
    pub file_system_sandbox_policy: &'a FileSystemSandboxPolicy,
    pub sandbox_policy_cwd: &'a AbsolutePathBuf,
    pub sandbox_permissions: SandboxPermissions,
    pub approval_sandbox_permissions: SandboxPermissions,
    pub prompt_permissions: Option<AdditionalPermissionProfile>,
    pub stopwatch: &'a Stopwatch,
    pub enable_shell_wrapper_parsing: bool,
}

pub fn approval_sandbox_permissions(
    sandbox_permissions: SandboxPermissions,
    additional_permissions_preapproved: bool,
) -> SandboxPermissions {
    if additional_permissions_preapproved
        && matches!(
            sandbox_permissions,
            SandboxPermissions::WithAdditionalPermissions
        )
    {
        SandboxPermissions::UseDefault
    } else {
        sandbox_permissions
    }
}

pub fn execve_prompt_is_rejected_by_policy(
    approval_policy: AskForApproval,
    decision_source: DecisionSource,
) -> Option<&'static str> {
    match (approval_policy, decision_source) {
        (AskForApproval::Never, _) => Some(PROMPT_CONFLICT_REASON),
        (AskForApproval::Granular(granular_config), DecisionSource::PrefixRule)
            if !granular_config.allows_rules_approval() =>
        {
            Some(REJECT_RULES_APPROVAL_REASON)
        }
        (AskForApproval::Granular(granular_config), DecisionSource::UnmatchedCommandFallback)
            if !granular_config.allows_sandbox_approval() =>
        {
            Some(REJECT_SANDBOX_APPROVAL_REASON)
        }
        _ => None,
    }
}

pub fn decision_driven_by_policy(matched_rules: &[RuleMatch], decision: Decision) -> bool {
    matched_rules.iter().any(|rule_match| {
        !matches!(rule_match, RuleMatch::HeuristicsRuleMatch { .. })
            && rule_match.decision() == decision
    })
}

pub fn shell_request_escalation_execution(
    sandbox_permissions: SandboxPermissions,
    permission_profile: &PermissionProfile,
    additional_permissions: Option<&AdditionalPermissionProfile>,
) -> EscalationExecution {
    match sandbox_permissions {
        SandboxPermissions::UseDefault => EscalationExecution::TurnDefault,
        SandboxPermissions::RequireEscalated => EscalationExecution::Unsandboxed,
        SandboxPermissions::WithAdditionalPermissions => additional_permissions
            .map(|_| {
                // Shell request additional permissions were already normalized and
                // merged into the first-attempt sandbox policy.
                EscalationExecution::Permissions(EscalationPermissions::ResolvedPermissionProfile(
                    ResolvedPermissionProfile {
                        permission_profile: permission_profile.clone(),
                    },
                ))
            })
            .unwrap_or(EscalationExecution::TurnDefault),
    }
}

pub async fn determine_escalation_action(
    params: EscalationPolicyDecisionParams<'_>,
    program: &AbsolutePathBuf,
    argv: &[String],
    workdir: &AbsolutePathBuf,
    prompt_handler: &(dyn EscalationPromptHandler + '_),
) -> anyhow::Result<EscalationDecision> {
    tracing::debug!(
        "Determining escalation action for command {program:?} with args {argv:?} in {workdir:?}"
    );

    let evaluation = evaluate_intercepted_exec_policy(
        params.policy,
        program,
        argv,
        InterceptedExecPolicyContext {
            approval_policy: params.approval_policy,
            permission_profile: params.permission_profile.clone(),
            file_system_sandbox_policy: params.file_system_sandbox_policy,
            sandbox_cwd: params.sandbox_policy_cwd.as_path(),
            sandbox_permissions: params.approval_sandbox_permissions,
            enable_shell_wrapper_parsing: params.enable_shell_wrapper_parsing,
        },
    );
    let driven_by_policy =
        decision_driven_by_policy(&evaluation.matched_rules, evaluation.decision);
    let needs_escalation =
        params.sandbox_permissions.requires_escalated_permissions() || driven_by_policy;

    let decision_source = if driven_by_policy {
        DecisionSource::PrefixRule
    } else {
        DecisionSource::UnmatchedCommandFallback
    };
    let escalation_execution = match decision_source {
        DecisionSource::PrefixRule => EscalationExecution::Unsandboxed,
        DecisionSource::UnmatchedCommandFallback => shell_request_escalation_execution(
            params.sandbox_permissions,
            params.permission_profile,
            params.prompt_permissions.as_ref(),
        ),
    };
    process_decision(
        evaluation.decision,
        needs_escalation,
        program,
        argv,
        workdir,
        params.prompt_permissions,
        escalation_execution,
        decision_source,
        params.approval_policy,
        params.stopwatch,
        prompt_handler,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn process_decision(
    decision: Decision,
    needs_escalation: bool,
    program: &AbsolutePathBuf,
    argv: &[String],
    workdir: &AbsolutePathBuf,
    prompt_permissions: Option<AdditionalPermissionProfile>,
    escalation_execution: EscalationExecution,
    decision_source: DecisionSource,
    approval_policy: AskForApproval,
    stopwatch: &Stopwatch,
    prompt_handler: &(dyn EscalationPromptHandler + '_),
) -> anyhow::Result<EscalationDecision> {
    let action = match decision {
        Decision::Forbidden => {
            EscalationDecision::deny(Some("Execution forbidden by policy".to_string()))
        }
        Decision::Prompt => {
            if execve_prompt_is_rejected_by_policy(approval_policy, decision_source).is_some() {
                EscalationDecision::deny(Some("Execution forbidden by policy".to_string()))
            } else {
                let prompt_decision = prompt_handler
                    .prompt(EscalationPromptRequest {
                        program,
                        argv,
                        workdir,
                        stopwatch,
                        additional_permissions: prompt_permissions,
                    })
                    .await?;
                match prompt_decision.decision {
                    ReviewDecision::Approved
                    | ReviewDecision::ApprovedForSession
                    | ReviewDecision::ApprovedExecpolicyAmendment { .. } => {
                        if needs_escalation {
                            EscalationDecision::escalate(escalation_execution.clone())
                        } else {
                            EscalationDecision::run()
                        }
                    }
                    ReviewDecision::NetworkPolicyAmendment {
                        network_policy_amendment,
                    } => match network_policy_amendment.action {
                        NetworkPolicyRuleAction::Allow => {
                            if needs_escalation {
                                EscalationDecision::escalate(escalation_execution.clone())
                            } else {
                                EscalationDecision::run()
                            }
                        }
                        NetworkPolicyRuleAction::Deny => {
                            EscalationDecision::deny(Some("User denied execution".to_string()))
                        }
                    },
                    ReviewDecision::Denied => EscalationDecision::deny(Some(
                        prompt_decision
                            .rejection_message
                            .unwrap_or_else(|| "User denied execution".to_string()),
                    )),
                    ReviewDecision::TimedOut => {
                        EscalationDecision::deny(Some(prompt_handler.timeout_message()))
                    }
                    ReviewDecision::Abort => {
                        EscalationDecision::deny(Some("User cancelled execution".to_string()))
                    }
                }
            }
        }
        Decision::Allow => {
            if needs_escalation {
                EscalationDecision::escalate(escalation_execution)
            } else {
                EscalationDecision::run()
            }
        }
    };
    tracing::debug!(
        "Policy decision for command {program:?} is {decision:?}, leading to escalation action {action:?}",
    );
    Ok(action)
}

pub fn extract_shell_script(command: &[String]) -> anyhow::Result<ParsedShellCommand> {
    // Commands reaching zsh-fork can be wrapped by environment/sandbox helpers,
    // so we search for the first `-c`/`-lc` triple anywhere in the argv rather
    // than assuming it is the first positional form.
    if let Some((program, script, login)) = command.windows(3).find_map(|parts| match parts {
        [program, flag, script] if flag == "-c" => {
            Some((program.to_owned(), script.to_owned(), false))
        }
        [program, flag, script] if flag == "-lc" => {
            Some((program.to_owned(), script.to_owned(), true))
        }
        _ => None,
    }) {
        return Ok(ParsedShellCommand {
            program,
            script,
            login,
        });
    }

    Err(anyhow!(
        "unexpected shell command format for zsh-fork execution"
    ))
}

pub fn map_exec_result(
    sandbox: SandboxType,
    result: ExecResult,
) -> Result<ExecToolCallOutput, CodexErr> {
    let output = ExecToolCallOutput {
        exit_code: result.exit_code,
        stdout: StreamOutput::new(result.stdout.clone()),
        stderr: StreamOutput::new(result.stderr.clone()),
        aggregated_output: StreamOutput::new(result.output.clone()),
        duration: result.duration,
        timed_out: result.timed_out,
    };

    if result.timed_out {
        return Err(CodexErr::Sandbox(SandboxErr::Timeout {
            output: Box::new(output),
        }));
    }

    if is_likely_sandbox_denied(sandbox, &output) {
        return Err(CodexErr::Sandbox(SandboxErr::Denied {
            output: Box::new(output),
            network_policy_decision: None,
        }));
    }

    Ok(output)
}

fn is_likely_sandbox_denied(sandbox_type: SandboxType, exec_output: &ExecToolCallOutput) -> bool {
    if sandbox_type == SandboxType::None || exec_output.exit_code == 0 {
        return false;
    }

    const SANDBOX_DENIED_KEYWORDS: [&str; 7] = [
        "operation not permitted",
        "permission denied",
        "read-only file system",
        "seccomp",
        "sandbox",
        "landlock",
        "failed to write file",
    ];

    let has_sandbox_keyword = [
        &exec_output.stderr.text,
        &exec_output.stdout.text,
        &exec_output.aggregated_output.text,
    ]
    .into_iter()
    .any(|section| {
        let lower = section.to_lowercase();
        SANDBOX_DENIED_KEYWORDS
            .iter()
            .any(|needle| lower.contains(needle))
    });

    if has_sandbox_keyword {
        return true;
    }

    const QUICK_REJECT_EXIT_CODES: [i32; 3] = [2, 126, 127];
    if QUICK_REJECT_EXIT_CODES.contains(&exec_output.exit_code) {
        return false;
    }

    sandbox_type == SandboxType::LinuxSeccomp
        && exec_output.exit_code == EXIT_CODE_SIGNAL_BASE + LINUX_SIGSYS_CODE
}

#[cfg(test)]
#[path = "policy_engine_tests.rs"]
mod tests;
