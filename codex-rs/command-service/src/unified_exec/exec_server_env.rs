use std::collections::HashMap;
use std::path::PathBuf;

use codex_protocol::config_types::ShellEnvironmentPolicy;

#[derive(Clone, Debug)]
pub struct ExecServerEnvConfig {
    pub policy: codex_exec_server_protocol::ExecEnvPolicy,
    pub local_policy_env: HashMap<String, String>,
}

const UNIFIED_EXEC_ENV: [(&str, &str); 10] = [
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("LANG", "C.UTF-8"),
    ("LC_CTYPE", "C.UTF-8"),
    ("LC_ALL", "C.UTF-8"),
    ("COLORTERM", ""),
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GH_PAGER", "cat"),
    ("CODEX_CI", "1"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecServerSpawnRequest {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub arg0: Option<String>,
}

pub fn apply_unified_exec_env(mut env: HashMap<String, String>) -> HashMap<String, String> {
    for (key, value) in UNIFIED_EXEC_ENV {
        env.insert(key.to_string(), value.to_string());
    }
    env
}

pub fn exec_env_policy_from_shell_policy(
    policy: &ShellEnvironmentPolicy,
) -> codex_exec_server_protocol::ExecEnvPolicy {
    codex_exec_server_protocol::ExecEnvPolicy {
        inherit: policy.inherit.clone(),
        ignore_default_excludes: policy.ignore_default_excludes,
        exclude: policy
            .exclude
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        r#set: policy.r#set.clone(),
        include_only: policy
            .include_only
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
    }
}

pub fn env_overlay_for_exec_server(
    request_env: &HashMap<String, String>,
    local_policy_env: &HashMap<String, String>,
) -> HashMap<String, String> {
    request_env
        .iter()
        .filter(|(key, value)| local_policy_env.get(*key) != Some(*value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub fn exec_server_process_id(process_id: i32) -> String {
    process_id.to_string()
}

pub fn exec_server_spawn_params(
    process_id: i32,
    request: ExecServerSpawnRequest,
    exec_server_env_config: Option<&ExecServerEnvConfig>,
    tty: bool,
) -> codex_exec_server_protocol::ExecParams {
    let (env_policy, env) = if let Some(exec_server_env_config) = exec_server_env_config {
        (
            Some(exec_server_env_config.policy.clone()),
            env_overlay_for_exec_server(&request.env, &exec_server_env_config.local_policy_env),
        )
    } else {
        (None, request.env)
    };

    codex_exec_server_protocol::ExecParams {
        process_id: exec_server_process_id(process_id).into(),
        argv: request.command,
        cwd: request.cwd,
        env_policy,
        env,
        tty,
        pipe_stdin: false,
        arg0: request.arg0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn unified_exec_env_injects_defaults() {
        let env = apply_unified_exec_env(HashMap::new());
        let expected = HashMap::from([
            ("NO_COLOR".to_string(), "1".to_string()),
            ("TERM".to_string(), "dumb".to_string()),
            ("LANG".to_string(), "C.UTF-8".to_string()),
            ("LC_CTYPE".to_string(), "C.UTF-8".to_string()),
            ("LC_ALL".to_string(), "C.UTF-8".to_string()),
            ("COLORTERM".to_string(), String::new()),
            ("PAGER".to_string(), "cat".to_string()),
            ("GIT_PAGER".to_string(), "cat".to_string()),
            ("GH_PAGER".to_string(), "cat".to_string()),
            ("CODEX_CI".to_string(), "1".to_string()),
        ]);

        assert_eq!(env, expected);
    }

    #[test]
    fn unified_exec_env_overrides_existing_values() {
        let mut base = HashMap::new();
        base.insert("NO_COLOR".to_string(), "0".to_string());
        base.insert("PATH".to_string(), "/usr/bin".to_string());

        let env = apply_unified_exec_env(base);

        assert_eq!(env.get("NO_COLOR"), Some(&"1".to_string()));
        assert_eq!(env.get("PATH"), Some(&"/usr/bin".to_string()));
    }

    #[test]
    fn env_overlay_for_exec_server_keeps_runtime_changes_only() {
        let local_policy_env = HashMap::from([
            ("HOME".to_string(), "/client-home".to_string()),
            ("PATH".to_string(), "/client-path".to_string()),
            ("SHELL_SET".to_string(), "policy".to_string()),
        ]);
        let request_env = HashMap::from([
            ("HOME".to_string(), "/client-home".to_string()),
            ("PATH".to_string(), "/sandbox-path".to_string()),
            ("SHELL_SET".to_string(), "policy".to_string()),
            ("CODEX_THREAD_ID".to_string(), "thread-1".to_string()),
            (
                "CODEX_SANDBOX_NETWORK_DISABLED".to_string(),
                "1".to_string(),
            ),
        ]);

        assert_eq!(
            env_overlay_for_exec_server(&request_env, &local_policy_env),
            HashMap::from([
                ("PATH".to_string(), "/sandbox-path".to_string()),
                ("CODEX_THREAD_ID".to_string(), "thread-1".to_string()),
                (
                    "CODEX_SANDBOX_NETWORK_DISABLED".to_string(),
                    "1".to_string()
                ),
            ])
        );
    }

    #[test]
    fn exec_server_params_use_env_policy_overlay_contract() {
        let exec_server_env_config = ExecServerEnvConfig {
            policy: codex_exec_server_protocol::ExecEnvPolicy {
                inherit: codex_protocol::config_types::ShellEnvironmentPolicyInherit::Core,
                ignore_default_excludes: false,
                exclude: Vec::new(),
                r#set: HashMap::new(),
                include_only: Vec::new(),
            },
            local_policy_env: HashMap::from([
                ("HOME".to_string(), "/client-home".to_string()),
                ("PATH".to_string(), "/client-path".to_string()),
            ]),
        };
        let request = ExecServerSpawnRequest {
            command: vec!["bash".to_string(), "-lc".to_string(), "true".to_string()],
            cwd: std::env::current_dir().expect("current dir"),
            env: HashMap::from([
                ("HOME".to_string(), "/client-home".to_string()),
                ("PATH".to_string(), "/sandbox-path".to_string()),
                ("CODEX_THREAD_ID".to_string(), "thread-1".to_string()),
            ]),
            arg0: None,
        };

        let params = exec_server_spawn_params(
            /*process_id*/ 123,
            request,
            Some(&exec_server_env_config),
            /*tty*/ true,
        );

        assert_eq!(params.process_id.as_str(), "123");
        assert!(params.env_policy.is_some());
        assert_eq!(
            params.env,
            HashMap::from([
                ("PATH".to_string(), "/sandbox-path".to_string()),
                ("CODEX_THREAD_ID".to_string(), "thread-1".to_string()),
            ])
        );
    }

    #[test]
    fn exec_server_process_id_matches_unified_exec_process_id() {
        assert_eq!(exec_server_process_id(/*process_id*/ 4321), "4321");
    }
}
