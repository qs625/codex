use protocol::ThreadId;
use protocol::config_types::ShellEnvironmentPolicy;

pub use protocol::shell_environment::CODEX_THREAD_ID_ENV_VAR;

pub fn create_env(
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<ThreadId>,
) -> std::collections::HashMap<String, String> {
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    protocol::shell_environment::create_env(policy, thread_id.as_deref())
}
