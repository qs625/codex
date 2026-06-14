use super::*;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

fn windows_shell_guidance_description() -> String {
    format!("\n\n{}", windows_shell_guidance())
}

fn expected_exec_command_description() -> String {
    "Runs a command and creates a command session. If the command exits during the initial wait window, returns its output and exit code; otherwise returns a command_id for `command_wait` and `command_write_stdin`. Live output is streamed to clients independently from model notifications.".to_string()
}

#[test]
fn exec_command_tool_matches_expected_spec() {
    let tool = create_exec_command_tool(CommandToolOptions {
        allow_login_shell: true,
        exec_permission_approvals_enabled: false,
    });

    let description = if cfg!(windows) {
        format!(
            "{}{}",
            expected_exec_command_description(),
            windows_shell_guidance_description()
        )
    } else {
        expected_exec_command_description()
    };

    let mut properties = BTreeMap::from([
        (
            "cmd".to_string(),
            JsonSchema::string(Some("Shell command to execute.".to_string())),
        ),
        (
            "workdir".to_string(),
            JsonSchema::string(Some(
                    "Optional working directory to run the command in; defaults to the turn cwd."
                        .to_string(),
                )),
        ),
        (
            "shell".to_string(),
            JsonSchema::string(Some(
                    "Shell binary to launch. Defaults to the user's default shell.".to_string(),
                )),
        ),
        (
            "tty".to_string(),
            JsonSchema::boolean(Some(
                    "Whether to allocate a TTY for the command. Defaults to false (plain pipes); set to true to open a PTY and access TTY process."
                        .to_string(),
                )),
        ),
        (
            "initial_wait_ms".to_string(),
            JsonSchema::number(Some(
                    "How long to wait initially (in milliseconds) before returning a running command session. If omitted, uses yield_time_ms for compatibility.".to_string(),
                )),
        ),
        (
            "notify_on".to_string(),
            JsonSchema::string(Some(
                    "When the model should be notified after the initial response: \"output\" wakes on new output or exit; \"exit\" wakes only when the command exits. Defaults to \"exit\".".to_string(),
                )),
        ),
        (
            "yield_time_ms".to_string(),
            JsonSchema::number(Some(
                    "Compatibility alias for initial_wait_ms.".to_string(),
                )),
        ),
        (
            "max_output_tokens".to_string(),
            JsonSchema::number(Some(
                    "Maximum number of tokens to return. Excess output will be truncated."
                        .to_string(),
                )),
        ),
        (
            "login".to_string(),
            JsonSchema::boolean(Some(
                    "Whether to run the shell with -l/-i semantics. Defaults to true.".to_string(),
                )),
        ),
    ]);
    properties.extend(create_approval_parameters(
        /*exec_permission_approvals_enabled*/ false,
    ));

    assert_eq!(
        tool,
        ToolSpec::Function(ResponsesApiTool {
            name: "exec_command".to_string(),
            description,
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(
                properties,
                Some(vec!["cmd".to_string()]),
                Some(false.into())
            ),
            output_schema: Some(unified_exec_output_schema()),
        })
    );
}

#[test]
fn write_stdin_tool_matches_expected_spec() {
    let tool = create_write_stdin_tool();

    let properties = BTreeMap::from([
        (
            "command_id".to_string(),
            JsonSchema::number(Some(
                "Identifier of the running command session to send input to.".to_string(),
            )),
        ),
        (
            "chars".to_string(),
            JsonSchema::string(Some(
                "Non-empty bytes to write to stdin. Use this only to send real input to a running interactive PTY session; do not use it to read output, wait for completion, or refresh command status.".to_string(),
            )),
        ),
    ]);

    assert_eq!(
        tool,
        ToolSpec::Function(ResponsesApiTool {
            name: "command_write_stdin".to_string(),
            description: "Writes characters to an existing command session so you can interact with a running PTY-backed command. Use this to answer prompts, send confirmations, or provide interactive input. `chars` is required and must be non-empty; use `command_wait` for command completion or output notifications instead of polling.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(
                properties,
                Some(vec!["command_id".to_string(), "chars".to_string()]),
                Some(false.into())
            ),
            output_schema: Some(command_write_stdin_output_schema()),
        })
    );
}

#[test]
fn request_permissions_tool_includes_full_permission_schema() {
    let tool =
        create_request_permissions_tool("Request extra permissions for this turn.".to_string());

    let properties = BTreeMap::from([
        (
            "reason".to_string(),
            JsonSchema::string(Some(
                "Optional short explanation for why additional permissions are needed.".to_string(),
            )),
        ),
        ("permissions".to_string(), permission_profile_schema()),
    ]);

    assert_eq!(
        tool,
        ToolSpec::Function(ResponsesApiTool {
            name: "request_permissions".to_string(),
            description: "Request extra permissions for this turn.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(
                properties,
                Some(vec!["permissions".to_string()]),
                Some(false.into())
            ),
            output_schema: None,
        })
    );
}

#[test]
fn shell_command_tool_matches_expected_spec() {
    let tool = create_shell_command_tool(CommandToolOptions {
        allow_login_shell: true,
        exec_permission_approvals_enabled: false,
    });

    let description = if cfg!(windows) {
        r#"Runs a Powershell command (Windows) and returns its output.

Examples of valid command strings:

- ls -a (show hidden): "Get-ChildItem -Force"
- recursive find by name: "Get-ChildItem -Recurse -Filter *.py"
- recursive grep: "Get-ChildItem -Path C:\\myrepo -Recurse | Select-String -Pattern 'TODO' -CaseSensitive"
- ps aux | grep python: "Get-Process | Where-Object { $_.ProcessName -like '*python*' }"
- setting an env var: "$env:FOO='bar'; echo $env:FOO"
- running an inline Python script: "@'\\nprint('Hello, world!')\\n'@ | python -""#
            .to_string()
            + &windows_shell_guidance_description()
    } else {
        r#"Runs a shell command and returns its output.
- Always set the `workdir` param when using the shell_command function. Do not use `cd` unless absolutely necessary."#
            .to_string()
    };

    let mut properties = BTreeMap::from([
        (
            "command".to_string(),
            JsonSchema::string(Some(
                "The shell script to execute in the user's default shell".to_string(),
            )),
        ),
        (
            "workdir".to_string(),
            JsonSchema::string(Some(
                "The working directory to execute the command in".to_string(),
            )),
        ),
        (
            "timeout_ms".to_string(),
            JsonSchema::number(Some(
                "The timeout for the command in milliseconds".to_string(),
            )),
        ),
        (
            "login".to_string(),
            JsonSchema::boolean(Some(
                "Whether to run the shell with login shell semantics. Defaults to true."
                    .to_string(),
            )),
        ),
    ]);
    properties.extend(create_approval_parameters(
        /*exec_permission_approvals_enabled*/ false,
    ));

    assert_eq!(
        tool,
        ToolSpec::Function(ResponsesApiTool {
            name: "shell_command".to_string(),
            description,
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(
                properties,
                Some(vec!["command".to_string()]),
                Some(false.into())
            ),
            output_schema: None,
        })
    );
}
