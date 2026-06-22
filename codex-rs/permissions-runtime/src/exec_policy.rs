use std::path::Path;

use codex_execpolicy_api::Decision;
use codex_execpolicy_api::Evaluation;
use codex_execpolicy_api::MatchOptions;
use codex_execpolicy_api::Policy;
use codex_execpolicy_api::RuleMatch;
use codex_protocol::approvals::ExecPolicyAmendment;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxKind;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_shell_command::bash::parse_shell_lc_plain_commands;
use codex_shell_command::bash::parse_shell_lc_single_command_prefix;
use codex_shell_command::is_dangerous_command::command_might_be_dangerous;
use codex_shell_command::is_safe_command::is_known_safe_command;
use codex_shell_utils::shlex_join;
use codex_utils_absolute_path::AbsolutePathBuf;

use crate::ExecApprovalRequirement;

const PROMPT_CONFLICT_REASON: &str =
    "approval required by policy, but AskForApproval is set to Never";
const REJECT_SANDBOX_APPROVAL_REASON: &str =
    "approval required by policy, but AskForApproval::Granular.sandbox_approval is false";
const REJECT_RULES_APPROVAL_REASON: &str =
    "approval required by policy rule, but AskForApproval::Granular.rules is false";
static BANNED_PREFIX_SUGGESTIONS: &[&[&str]] = &[
    &["python3"],
    &["python3", "-"],
    &["python3", "-c"],
    &["python"],
    &["python", "-"],
    &["python", "-c"],
    &["py"],
    &["py", "-3"],
    &["pythonw"],
    &["pyw"],
    &["pypy"],
    &["pypy3"],
    &["git"],
    &["bash"],
    &["bash", "-lc"],
    &["sh"],
    &["sh", "-c"],
    &["sh", "-lc"],
    &["zsh"],
    &["zsh", "-lc"],
    &["/bin/zsh"],
    &["/bin/zsh", "-lc"],
    &["/bin/bash"],
    &["/bin/bash", "-lc"],
    &["pwsh"],
    &["pwsh", "-Command"],
    &["pwsh", "-c"],
    &["powershell"],
    &["powershell", "-Command"],
    &["powershell", "-c"],
    &["powershell.exe"],
    &["powershell.exe", "-Command"],
    &["powershell.exe", "-c"],
    &["env"],
    &["sudo"],
    &["node"],
    &["node", "-e"],
    &["perl"],
    &["perl", "-e"],
    &["ruby"],
    &["ruby", "-e"],
    &["php"],
    &["php", "-r"],
    &["lua"],
    &["lua", "-e"],
    &["osascript"],
];

/// Describes which unmatched-command heuristics should classify the command
/// words being evaluated by exec-policy.
///
/// The command tokens may be the original argv or a shell-specific lowering of
/// a wrapper such as `bash -lc ...` or `powershell.exe -Command ...`. We only
/// need to distinguish the PowerShell case because its safelist and dangerous
/// heuristics operate on PowerShell-flavored inner command words rather than
/// the generic command classifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecPolicyCommandOrigin {
    /// Use the generic unmatched-command heuristics.
    Generic,
    #[cfg(windows)]
    /// The command words came from the `-Command` body of a top-level
    /// PowerShell wrapper, so use PowerShell-specific unmatched-command
    /// heuristics for the lowered words.
    PowerShell,
}

#[derive(Clone, Copy)]
pub struct UnmatchedCommandContext<'a> {
    pub approval_policy: AskForApproval,
    pub permission_profile: &'a PermissionProfile,
    pub file_system_sandbox_policy: &'a FileSystemSandboxPolicy,
    pub sandbox_cwd: &'a Path,
    pub sandbox_permissions: SandboxPermissions,
    pub used_complex_parsing: bool,
    pub command_origin: ExecPolicyCommandOrigin,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ExecPolicyCommands {
    pub commands: Vec<Vec<String>>,
    pub used_complex_parsing: bool,
    pub command_origin: ExecPolicyCommandOrigin,
}

/// Command approval request evaluated against the current exec-policy.
pub struct ExecPolicyApprovalRequest<'a> {
    pub command: &'a [String],
    pub approval_policy: AskForApproval,
    pub permission_profile: PermissionProfile,
    pub file_system_sandbox_policy: &'a FileSystemSandboxPolicy,
    pub sandbox_cwd: &'a Path,
    pub sandbox_permissions: SandboxPermissions,
    pub prefix_rule: Option<Vec<String>>,
}

pub fn is_policy_match(rule_match: &RuleMatch) -> bool {
    match rule_match {
        RuleMatch::PrefixRuleMatch { .. } => true,
        RuleMatch::HeuristicsRuleMatch { .. } => false,
    }
}

/// Returns a rejection reason when `approval_policy` disallows surfacing the
/// current prompt to the user.
///
/// `prompt_is_rule` distinguishes policy-rule prompts from sandbox/escalation
/// prompts so granular `rules` and `sandbox_approval` settings are honored
/// independently. When both are present, policy-rule prompts take precedence.
pub fn prompt_is_rejected_by_policy(
    approval_policy: AskForApproval,
    prompt_is_rule: bool,
) -> Option<&'static str> {
    match approval_policy {
        AskForApproval::Never => Some(PROMPT_CONFLICT_REASON),
        AskForApproval::OnFailure => None,
        AskForApproval::OnRequest => None,
        AskForApproval::UnlessTrusted => None,
        AskForApproval::Granular(granular_config) => {
            if prompt_is_rule {
                if !granular_config.allows_rules_approval() {
                    Some(REJECT_RULES_APPROVAL_REASON)
                } else {
                    None
                }
            } else if !granular_config.allows_sandbox_approval() {
                Some(REJECT_SANDBOX_APPROVAL_REASON)
            } else {
                None
            }
        }
    }
}

pub fn create_exec_approval_requirement_for_command(
    exec_policy: &Policy,
    req: ExecPolicyApprovalRequest<'_>,
) -> ExecApprovalRequirement {
    let ExecPolicyApprovalRequest {
        command,
        approval_policy,
        permission_profile,
        file_system_sandbox_policy,
        sandbox_cwd,
        sandbox_permissions,
        prefix_rule,
    } = req;
    let ExecPolicyCommands {
        commands,
        used_complex_parsing,
        command_origin,
    } = commands_for_exec_policy(command);
    // Keep heredoc prefix parsing for rule evaluation so existing
    // allow/prompt/forbidden rules still apply, but avoid auto-derived
    // amendments when only the heredoc fallback parser matched.
    let auto_amendment_allowed = !used_complex_parsing;
    let exec_policy_fallback = |cmd: &[String]| {
        render_decision_for_unmatched_command(
            cmd,
            UnmatchedCommandContext {
                approval_policy,
                permission_profile: &permission_profile,
                file_system_sandbox_policy,
                sandbox_cwd,
                sandbox_permissions,
                used_complex_parsing,
                command_origin,
            },
        )
    };
    let match_options = MatchOptions {
        resolve_host_executables: true,
    };
    let evaluation = exec_policy.check_multiple_with_options(
        commands.iter(),
        &exec_policy_fallback,
        &match_options,
    );

    let requested_amendment = if auto_amendment_allowed {
        derive_requested_execpolicy_amendment_from_prefix_rule(
            prefix_rule.as_ref(),
            &evaluation.matched_rules,
            exec_policy,
            &commands,
            &exec_policy_fallback,
            &match_options,
        )
    } else {
        None
    };

    match evaluation.decision {
        Decision::Forbidden => ExecApprovalRequirement::Forbidden {
            reason: derive_forbidden_reason(command, &evaluation),
        },
        Decision::Prompt => {
            let prompt_is_rule = evaluation.matched_rules.iter().any(|rule_match| {
                is_policy_match(rule_match) && rule_match.decision() == Decision::Prompt
            });
            match prompt_is_rejected_by_policy(approval_policy, prompt_is_rule) {
                Some(reason) => ExecApprovalRequirement::Forbidden {
                    reason: reason.to_string(),
                },
                None => ExecApprovalRequirement::NeedsApproval {
                    reason: derive_prompt_reason(command, &evaluation),
                    proposed_execpolicy_amendment: requested_amendment.or_else(|| {
                        if auto_amendment_allowed {
                            try_derive_execpolicy_amendment_for_prompt_rules(
                                &evaluation.matched_rules,
                            )
                        } else {
                            None
                        }
                    }),
                },
            }
        }
        Decision::Allow => ExecApprovalRequirement::Skip {
            // Bypass sandbox only when every parsed command segment is
            // explicitly allowed by execpolicy.
            bypass_sandbox: commands.iter().all(|command| {
                exec_policy
                    .matches_for_command_with_options(
                        command,
                        /*heuristics_fallback*/ None,
                        &match_options,
                    )
                    .iter()
                    .any(|rule_match| {
                        is_policy_match(rule_match) && rule_match.decision() == Decision::Allow
                    })
            }),
            proposed_execpolicy_amendment: if auto_amendment_allowed {
                try_derive_execpolicy_amendment_for_allow_rules(&evaluation.matched_rules)
            } else {
                None
            },
        },
    }
}

/// If a command is not matched by any execpolicy rule, derive a [`Decision`].
pub fn render_decision_for_unmatched_command(
    command: &[String],
    context: UnmatchedCommandContext<'_>,
) -> Decision {
    let UnmatchedCommandContext {
        approval_policy,
        permission_profile,
        file_system_sandbox_policy,
        sandbox_cwd,
        sandbox_permissions,
        used_complex_parsing,
        command_origin,
    } = context;
    let is_known_safe = match command_origin {
        ExecPolicyCommandOrigin::Generic => is_known_safe_command(command),
        #[cfg(windows)]
        ExecPolicyCommandOrigin::PowerShell => {
            codex_shell_command::is_safe_command::is_safe_powershell_words(command)
        }
    };

    // On Windows, ReadOnly sandbox is not a real sandbox, so special-case it
    // here.
    let environment_lacks_sandbox_protections = cfg!(windows)
        && profile_is_managed_read_only(
            permission_profile,
            file_system_sandbox_policy,
            sandbox_cwd,
        );

    if is_known_safe
        && !used_complex_parsing
        && (approval_policy == AskForApproval::UnlessTrusted
            || environment_lacks_sandbox_protections)
    {
        return Decision::Allow;
    }

    // If the command is flagged as dangerous or we have no sandbox protection,
    // we should never allow it to run without approval.
    //
    // We prefer to prompt the user rather than outright forbid the command,
    // but if the user has explicitly disabled prompts, we must
    // forbid the command.
    let command_is_dangerous = match command_origin {
        ExecPolicyCommandOrigin::Generic => command_might_be_dangerous(command),
        #[cfg(windows)]
        ExecPolicyCommandOrigin::PowerShell => {
            codex_shell_command::is_dangerous_command::is_dangerous_powershell_words(command)
        }
    };
    if command_is_dangerous || environment_lacks_sandbox_protections {
        return match approval_policy {
            AskForApproval::Never => {
                let sandbox_is_explicitly_disabled = matches!(
                    permission_profile,
                    PermissionProfile::Disabled | PermissionProfile::External { .. }
                );
                if sandbox_is_explicitly_disabled {
                    // If the sandbox is explicitly disabled, we should allow the command to run
                    Decision::Allow
                } else {
                    Decision::Forbidden
                }
            }
            AskForApproval::OnFailure
            | AskForApproval::OnRequest
            | AskForApproval::UnlessTrusted
            | AskForApproval::Granular(_) => Decision::Prompt,
        };
    }

    match approval_policy {
        AskForApproval::Never | AskForApproval::OnFailure => {
            // We allow the command to run, relying on the sandbox for
            // protection.
            Decision::Allow
        }
        AskForApproval::UnlessTrusted => {
            // We already checked the unmatched-command safelist and it
            // returned false, so we must prompt.
            Decision::Prompt
        }
        AskForApproval::OnRequest => match file_system_sandbox_policy.kind {
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
                // The user has indicated we should "just run" commands
                // in their unrestricted environment, so we do so since the
                // command has not been flagged as dangerous.
                Decision::Allow
            }
            FileSystemSandboxKind::Restricted => {
                // In restricted sandboxes, do not prompt for non-escalated,
                // non-dangerous commands; let the sandbox enforce
                // restrictions without a user prompt.
                if sandbox_permissions.requests_sandbox_override() {
                    Decision::Prompt
                } else {
                    Decision::Allow
                }
            }
        },
        AskForApproval::Granular(_) => match file_system_sandbox_policy.kind {
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
                // Mirror on-request behavior for unmatched commands; prompt-vs-reject is handled
                // by `prompt_is_rejected_by_policy`.
                Decision::Allow
            }
            FileSystemSandboxKind::Restricted => {
                if sandbox_permissions.requests_sandbox_override() {
                    Decision::Prompt
                } else {
                    Decision::Allow
                }
            }
        },
    }
}

#[doc(hidden)]
pub fn profile_is_managed_read_only(
    permission_profile: &PermissionProfile,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    sandbox_cwd: &Path,
) -> bool {
    matches!(permission_profile, PermissionProfile::Managed { .. })
        && matches!(
            file_system_sandbox_policy.kind,
            FileSystemSandboxKind::Restricted
        )
        && !file_system_sandbox_policy.has_full_disk_write_access()
        && file_system_sandbox_policy
            .get_writable_roots_with_cwd(sandbox_cwd)
            .is_empty()
}

pub fn commands_for_exec_policy(command: &[String]) -> ExecPolicyCommands {
    if let Some(commands) = parse_shell_lc_plain_commands(command)
        && !commands.is_empty()
    {
        return ExecPolicyCommands {
            commands,
            used_complex_parsing: false,
            command_origin: ExecPolicyCommandOrigin::Generic,
        };
    }

    #[cfg(windows)]
    {
        if let Some(commands) =
            codex_shell_command::powershell::parse_powershell_command_into_plain_commands(command)
            && !commands.is_empty()
        {
            return ExecPolicyCommands {
                commands,
                used_complex_parsing: false,
                command_origin: ExecPolicyCommandOrigin::PowerShell,
            };
        }
    }

    if let Some(single_command) = parse_shell_lc_single_command_prefix(command) {
        return ExecPolicyCommands {
            commands: vec![single_command],
            used_complex_parsing: true,
            command_origin: ExecPolicyCommandOrigin::Generic,
        };
    }

    ExecPolicyCommands {
        commands: vec![command.to_vec()],
        used_complex_parsing: false,
        command_origin: ExecPolicyCommandOrigin::Generic,
    }
}

/// Derive a proposed execpolicy amendment when a command requires user approval
/// - If any execpolicy rule prompts, return None, because an amendment would not skip that policy requirement.
/// - Otherwise return the first heuristics Prompt.
/// - Examples:
/// - execpolicy: empty. Command: `["python"]`. Heuristics prompt -> `Some(vec!["python"])`.
/// - execpolicy: empty. Command: `["bash", "-c", "cd /some/folder && prog1 --option1 arg1 && prog2 --option2 arg2"]`.
///   Parsed commands include `cd /some/folder`, `prog1 --option1 arg1`, and `prog2 --option2 arg2`. If heuristics allow `cd` but prompt
///   on `prog1`, we return `Some(vec!["prog1", "--option1", "arg1"])`.
/// - execpolicy: contains a `prompt for prefix ["prog2"]` rule. For the same command as above,
///   we return `None` because an execpolicy prompt still applies even if we amend execpolicy to allow ["prog1", "--option1", "arg1"].
fn try_derive_execpolicy_amendment_for_prompt_rules(
    matched_rules: &[RuleMatch],
) -> Option<ExecPolicyAmendment> {
    if matched_rules
        .iter()
        .any(|rule_match| is_policy_match(rule_match) && rule_match.decision() == Decision::Prompt)
    {
        return None;
    }

    matched_rules
        .iter()
        .find_map(|rule_match| match rule_match {
            RuleMatch::HeuristicsRuleMatch {
                command,
                decision: Decision::Prompt,
            } => Some(ExecPolicyAmendment::from(command.clone())),
            _ => None,
        })
}

/// - Note: we only use this amendment when the command fails to run in sandbox and codex prompts the user to run outside the sandbox
/// - The purpose of this amendment is to bypass sandbox for similar commands in the future
/// - If any execpolicy rule matches, return None, because we would already be running command outside the sandbox
fn try_derive_execpolicy_amendment_for_allow_rules(
    matched_rules: &[RuleMatch],
) -> Option<ExecPolicyAmendment> {
    if matched_rules.iter().any(is_policy_match) {
        return None;
    }

    matched_rules
        .iter()
        .find_map(|rule_match| match rule_match {
            RuleMatch::HeuristicsRuleMatch {
                command,
                decision: Decision::Allow,
            } => Some(ExecPolicyAmendment::from(command.clone())),
            _ => None,
        })
}

#[doc(hidden)]
pub fn derive_requested_execpolicy_amendment_from_prefix_rule(
    prefix_rule: Option<&Vec<String>>,
    matched_rules: &[RuleMatch],
    exec_policy: &Policy,
    commands: &[Vec<String>],
    exec_policy_fallback: &impl Fn(&[String]) -> Decision,
    match_options: &MatchOptions,
) -> Option<ExecPolicyAmendment> {
    let prefix_rule = prefix_rule?;
    if prefix_rule.is_empty() {
        return None;
    }
    if BANNED_PREFIX_SUGGESTIONS.iter().any(|banned| {
        prefix_rule.len() == banned.len()
            && prefix_rule
                .iter()
                .map(String::as_str)
                .eq(banned.iter().copied())
    }) {
        return None;
    }

    // if any policy rule already matches, don't suggest an additional rule that might conflict or not apply
    if matched_rules.iter().any(is_policy_match) {
        return None;
    }

    let amendment = ExecPolicyAmendment::new(prefix_rule.clone());
    if prefix_rule_would_approve_all_commands(
        exec_policy,
        &amendment.command,
        commands,
        exec_policy_fallback,
        match_options,
    ) {
        Some(amendment)
    } else {
        None
    }
}

fn prefix_rule_would_approve_all_commands(
    exec_policy: &Policy,
    prefix_rule: &[String],
    commands: &[Vec<String>],
    exec_policy_fallback: &impl Fn(&[String]) -> Decision,
    match_options: &MatchOptions,
) -> bool {
    let mut policy_with_prefix_rule = exec_policy.clone();
    if policy_with_prefix_rule
        .add_prefix_rule(prefix_rule, Decision::Allow)
        .is_err()
    {
        return false;
    }

    commands.iter().all(|command| {
        policy_with_prefix_rule
            .check_with_options(command, exec_policy_fallback, match_options)
            .decision
            == Decision::Allow
    })
}

/// Only return a reason when a policy rule drove the prompt decision.
fn derive_prompt_reason(command_args: &[String], evaluation: &Evaluation) -> Option<String> {
    let command = render_shlex_command(command_args);

    let most_specific_prompt = evaluation
        .matched_rules
        .iter()
        .filter_map(|rule_match| match rule_match {
            RuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision: Decision::Prompt,
                justification,
                ..
            } => Some((matched_prefix.len(), justification.as_deref())),
            _ => None,
        })
        .max_by_key(|(matched_prefix_len, _)| *matched_prefix_len);

    match most_specific_prompt {
        Some((_matched_prefix_len, Some(justification))) => {
            Some(format!("`{command}` requires approval: {justification}"))
        }
        Some((_matched_prefix_len, None)) => {
            Some(format!("`{command}` requires approval by policy"))
        }
        None => None,
    }
}

fn render_shlex_command(args: &[String]) -> String {
    shlex_join(args)
}

/// Derive a string explaining why the command was forbidden. If `justification`
/// is set by the user, this can contain instructions with recommended
/// alternatives, for example.
fn derive_forbidden_reason(command_args: &[String], evaluation: &Evaluation) -> String {
    let command = render_shlex_command(command_args);

    let most_specific_forbidden = evaluation
        .matched_rules
        .iter()
        .filter_map(|rule_match| match rule_match {
            RuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision: Decision::Forbidden,
                justification,
                ..
            } => Some((matched_prefix, justification.as_deref())),
            _ => None,
        })
        .max_by_key(|(matched_prefix, _)| matched_prefix.len());

    match most_specific_forbidden {
        Some((_matched_prefix, Some(justification))) => {
            format!("`{command}` rejected: {justification}")
        }
        Some((matched_prefix, None)) => {
            let prefix = render_shlex_command(matched_prefix);
            format!("`{command}` rejected: policy forbids commands starting with `{prefix}`")
        }
        None => format!("`{command}` rejected: blocked by policy"),
    }
}

#[derive(Clone)]
pub struct InterceptedExecPolicyContext<'a> {
    pub approval_policy: AskForApproval,
    pub permission_profile: PermissionProfile,
    pub file_system_sandbox_policy: &'a FileSystemSandboxPolicy,
    pub sandbox_cwd: &'a Path,
    pub sandbox_permissions: SandboxPermissions,
    pub enable_shell_wrapper_parsing: bool,
}

pub fn evaluate_intercepted_exec_policy(
    policy: &Policy,
    program: &AbsolutePathBuf,
    argv: &[String],
    context: InterceptedExecPolicyContext<'_>,
) -> Evaluation {
    let InterceptedExecPolicyContext {
        approval_policy,
        permission_profile,
        file_system_sandbox_policy,
        sandbox_cwd,
        sandbox_permissions,
        enable_shell_wrapper_parsing,
    } = context;
    let InterceptedExecPolicyCommands {
        commands,
        used_complex_parsing,
    } = if enable_shell_wrapper_parsing {
        // In this codepath, the first argument in `commands` could be a bare
        // name like `find` instead of an absolute path like `/usr/bin/find`.
        // It could also be a shell built-in like `echo`.
        commands_for_intercepted_exec_policy(program, argv)
    } else {
        // In this codepath, `commands` has a single entry where the program
        // is always an absolute path.
        InterceptedExecPolicyCommands {
            commands: vec![join_program_and_argv(program, argv)],
            used_complex_parsing: false,
        }
    };

    let fallback = |cmd: &[String]| {
        render_decision_for_unmatched_command(
            cmd,
            UnmatchedCommandContext {
                approval_policy,
                permission_profile: &permission_profile,
                file_system_sandbox_policy,
                sandbox_cwd,
                sandbox_permissions,
                used_complex_parsing,
                command_origin: ExecPolicyCommandOrigin::Generic,
            },
        )
    };

    policy.check_multiple_with_options(
        commands.iter(),
        &fallback,
        &MatchOptions {
            resolve_host_executables: true,
        },
    )
}

#[derive(Debug, Eq, PartialEq)]
pub struct InterceptedExecPolicyCommands {
    pub commands: Vec<Vec<String>>,
    pub used_complex_parsing: bool,
}

pub fn commands_for_intercepted_exec_policy(
    program: &AbsolutePathBuf,
    argv: &[String],
) -> InterceptedExecPolicyCommands {
    if let [_, flag, script] = argv {
        let shell_command = [
            program.to_string_lossy().to_string(),
            flag.clone(),
            script.clone(),
        ];
        if let Some(commands) = parse_shell_lc_plain_commands(&shell_command) {
            return InterceptedExecPolicyCommands {
                commands,
                used_complex_parsing: false,
            };
        }
        if let Some(single_command) = parse_shell_lc_single_command_prefix(&shell_command) {
            return InterceptedExecPolicyCommands {
                commands: vec![single_command],
                used_complex_parsing: true,
            };
        }
    }

    InterceptedExecPolicyCommands {
        commands: vec![join_program_and_argv(program, argv)],
        used_complex_parsing: false,
    }
}

/// Convert an intercepted exec `(program, argv)` into a command vector suitable
/// for display and policy parsing.
///
/// The intercepted `argv` includes `argv[0]`, but once we have normalized the
/// executable path in `program`, we should replace the original `argv[0]`
/// rather than duplicating it as an apparent user argument.
pub fn join_program_and_argv(program: &AbsolutePathBuf, argv: &[String]) -> Vec<String> {
    std::iter::once(program.to_string_lossy().to_string())
        .chain(argv.iter().skip(1).cloned())
        .collect::<Vec<_>>()
}
