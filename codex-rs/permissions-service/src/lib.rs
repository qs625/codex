mod exec_policy;
mod exec_policy_loader;
mod exec_policy_manager;
mod exec_policy_parser;
mod execpolicy_parser_error;
mod network_approval;
mod request_permissions;

pub use exec_policy::ExecPolicyApprovalRequest;
pub use exec_policy::ExecPolicyCommandOrigin;
pub use exec_policy::ExecPolicyCommands;
pub use exec_policy::InterceptedExecPolicyCommands;
pub use exec_policy::InterceptedExecPolicyContext;
pub use exec_policy::UnmatchedCommandContext;
pub use exec_policy::commands_for_exec_policy;
pub use exec_policy::commands_for_intercepted_exec_policy;
pub use exec_policy::create_exec_approval_requirement_for_command;
pub use exec_policy::default_exec_approval_requirement;
pub use exec_policy::derive_requested_execpolicy_amendment_from_prefix_rule;
pub use exec_policy::evaluate_intercepted_exec_policy;
pub use exec_policy::is_policy_match;
pub use exec_policy::join_program_and_argv;
pub use exec_policy::profile_is_managed_read_only;
pub use exec_policy::prompt_is_rejected_by_policy;
pub use exec_policy::render_decision_for_unmatched_command;
pub use exec_policy_loader::ExecPolicyError;
pub use exec_policy_loader::StarlarkExecPolicyLoader;
pub use exec_policy_loader::check_execpolicy_for_warnings;
pub use exec_policy_loader::format_exec_policy_error_with_source;
pub use exec_policy_manager::EmptyExecPolicyLoader;
pub use exec_policy_manager::ExecPolicyLoadResult;
pub use exec_policy_manager::ExecPolicyLoader;
pub use exec_policy_manager::ExecPolicyManager;
pub use exec_policy_manager::ExecPolicyUpdateError;
pub use exec_policy_manager::default_policy_path;
pub use exec_policy_parser::PolicyParser;
pub use execpolicy_parser_error::Error;
pub use execpolicy_parser_error::ErrorLocation;
pub use permissions_service_api::ExecApprovalRequirement;
pub use execpolicy_parser_error::Result;
pub use execpolicy_parser_error::TextPosition;
pub use execpolicy_parser_error::TextRange;
pub use network_approval::ActiveNetworkApprovalCall;
pub use network_approval::HostApprovalKey;
pub use network_approval::NetworkApprovalOutcome;
pub use network_approval::NetworkApprovalRuntime;
pub use network_approval::PendingApprovalDecision;
pub use network_approval::PendingHostApproval;
pub use network_approval::allows_network_approval_flow;
pub use network_approval::denied_network_policy_message;
pub use network_approval::permission_profile_allows_network_approval_flow;
pub use permissions_service_api::Decision;
pub use permissions_service_api::MatchOptions;
pub use permissions_service_api::NetworkRuleProtocol;
pub use permissions_service_api::Policy;
pub use exec_policy::ReadDenyMatcher;
pub use request_permissions::validate_network_policy_amendment_host;
use permissions_service_api::PermissionsServiceApi;
use permissions_service_api::PermissionsServiceFuture;

#[derive(Default)]
pub struct PermissionsService;

impl PermissionsServiceApi for PermissionsService {
    fn create_exec_approval_requirement<'a>(
        &'a self,
        exec_policy: &'a Policy,
        request: permissions_service_api::ExecPolicyApprovalRequest<'a>,
    ) -> PermissionsServiceFuture<'a, permissions_service_api::ExecApprovalRequirement> {
        Box::pin(async move {
            create_exec_approval_requirement_for_command(
                exec_policy,
                ExecPolicyApprovalRequest {
                    command: request.command,
                    approval_policy: request.approval_policy,
                    permission_profile: request.permission_profile,
                    file_system_sandbox_policy: request.file_system_sandbox_policy,
                    sandbox_cwd: request.sandbox_cwd,
                    sandbox_permissions: request.sandbox_permissions,
                    prefix_rule: request.prefix_rule,
                },
            )
        })
    }
}
