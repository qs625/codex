use super::*;

#[test]
fn fs_get_metadata_response_round_trips_minimal_fields() {
    let response = FsGetMetadataResponse {
        is_directory: false,
        is_file: true,
        is_symlink: false,
        created_at_ms: 123,
        modified_at_ms: 456,
    };

    let value = serde_json::to_value(&response).expect("serialize fs/getMetadata response");
    assert_eq!(
        value,
        json!({
            "isDirectory": false,
            "isFile": true,
            "isSymlink": false,
            "createdAtMs": 123,
            "modifiedAtMs": 456,
        })
    );

    let decoded = serde_json::from_value::<FsGetMetadataResponse>(value)
        .expect("deserialize fs/getMetadata response");
    assert_eq!(decoded, response);
}

#[test]
fn fs_read_file_response_round_trips_base64_data() {
    let response = FsReadFileResponse {
        data_base64: "aGVsbG8=".to_string(),
    };

    let value = serde_json::to_value(&response).expect("serialize fs/readFile response");
    assert_eq!(
        value,
        json!({
            "dataBase64": "aGVsbG8=",
        })
    );

    let decoded = serde_json::from_value::<FsReadFileResponse>(value)
        .expect("deserialize fs/readFile response");
    assert_eq!(decoded, response);
}

#[test]
fn fs_read_file_params_round_trip() {
    let params = FsReadFileParams {
        path: absolute_path("tmp/example.txt"),
    };

    let value = serde_json::to_value(&params).expect("serialize fs/readFile params");
    assert_eq!(
        value,
        json!({
            "path": absolute_path_string("tmp/example.txt"),
        })
    );

    let decoded =
        serde_json::from_value::<FsReadFileParams>(value).expect("deserialize fs/readFile params");
    assert_eq!(decoded, params);
}

#[test]
fn fs_create_directory_params_round_trip_with_default_recursive() {
    let params = FsCreateDirectoryParams {
        path: absolute_path("tmp/example"),
        recursive: None,
    };

    let value = serde_json::to_value(&params).expect("serialize fs/createDirectory params");
    assert_eq!(
        value,
        json!({
            "path": absolute_path_string("tmp/example"),
            "recursive": null,
        })
    );

    let decoded = serde_json::from_value::<FsCreateDirectoryParams>(value)
        .expect("deserialize fs/createDirectory params");
    assert_eq!(decoded, params);
}

#[test]
fn fs_write_file_params_round_trip_with_base64_data() {
    let params = FsWriteFileParams {
        path: absolute_path("tmp/example.bin"),
        data_base64: "AAE=".to_string(),
    };

    let value = serde_json::to_value(&params).expect("serialize fs/writeFile params");
    assert_eq!(
        value,
        json!({
            "path": absolute_path_string("tmp/example.bin"),
            "dataBase64": "AAE=",
        })
    );

    let decoded = serde_json::from_value::<FsWriteFileParams>(value)
        .expect("deserialize fs/writeFile params");
    assert_eq!(decoded, params);
}

#[test]
fn fs_copy_params_round_trip_with_recursive_directory_copy() {
    let params = FsCopyParams {
        source_path: absolute_path("tmp/source"),
        destination_path: absolute_path("tmp/destination"),
        recursive: true,
    };

    let value = serde_json::to_value(&params).expect("serialize fs/copy params");
    assert_eq!(
        value,
        json!({
            "sourcePath": absolute_path_string("tmp/source"),
            "destinationPath": absolute_path_string("tmp/destination"),
            "recursive": true,
        })
    );

    let decoded =
        serde_json::from_value::<FsCopyParams>(value).expect("deserialize fs/copy params");
    assert_eq!(decoded, params);
}

#[test]
fn thread_shell_command_params_round_trip() {
    let params = ThreadShellCommandParams {
        thread_id: "thr_123".to_string(),
        command: "printf 'hello world\\n'".to_string(),
    };

    let value = serde_json::to_value(&params).expect("serialize thread/shellCommand params");
    assert_eq!(
        value,
        json!({
            "threadId": "thr_123",
            "command": "printf 'hello world\\n'",
        })
    );

    let decoded = serde_json::from_value::<ThreadShellCommandParams>(value)
        .expect("deserialize thread/shellCommand params");
    assert_eq!(decoded, params);
}

#[test]
fn thread_shell_command_response_round_trip() {
    let response = ThreadShellCommandResponse {};

    let value = serde_json::to_value(&response).expect("serialize thread/shellCommand response");
    assert_eq!(value, json!({}));

    let decoded = serde_json::from_value::<ThreadShellCommandResponse>(value)
        .expect("deserialize thread/shellCommand response");
    assert_eq!(decoded, response);
}

#[test]
fn fs_changed_notification_round_trips() {
    let notification = FsChangedNotification {
        watch_id: "0195ec6b-1d6f-7c2e-8c7a-56f2c4a8b9d1".to_string(),
        changed_paths: vec![
            absolute_path("tmp/repo/.git/HEAD"),
            absolute_path("tmp/repo/.git/FETCH_HEAD"),
        ],
    };

    let value = serde_json::to_value(&notification).expect("serialize fs/changed notification");
    assert_eq!(
        value,
        json!({
            "watchId": "0195ec6b-1d6f-7c2e-8c7a-56f2c4a8b9d1",
            "changedPaths": [
                absolute_path_string("tmp/repo/.git/HEAD"),
                absolute_path_string("tmp/repo/.git/FETCH_HEAD"),
            ],
        })
    );

    let decoded = serde_json::from_value::<FsChangedNotification>(value)
        .expect("deserialize fs/changed notification");
    assert_eq!(decoded, notification);
}

#[test]
fn command_exec_params_default_optional_streaming_flags() {
    let params = serde_json::from_value::<CommandExecParams>(json!({
        "command": ["ls", "-la"],
        "timeoutMs": 1000,
        "cwd": "/tmp"
    }))
    .expect("command/exec payload should deserialize");

    assert_eq!(
        params,
        CommandExecParams {
            command: vec!["ls".to_string(), "-la".to_string()],
            process_id: None,
            tty: false,
            stream_stdin: false,
            stream_stdout_stderr: false,
            output_bytes_cap: None,
            disable_output_cap: false,
            disable_timeout: false,
            timeout_ms: Some(1000),
            cwd: Some(PathBuf::from("/tmp")),
            env: None,
            size: None,
            sandbox_policy: None,
            permission_profile: None,
        }
    );
}

#[test]
fn command_exec_params_round_trips_disable_timeout() {
    let params = CommandExecParams {
        command: vec!["sleep".to_string(), "30".to_string()],
        process_id: Some("sleep-1".to_string()),
        tty: false,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: None,
        disable_output_cap: false,
        disable_timeout: true,
        timeout_ms: None,
        cwd: None,
        env: None,
        size: None,
        sandbox_policy: None,
        permission_profile: None,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec params");
    assert_eq!(
        value,
        json!({
            "command": ["sleep", "30"],
            "processId": "sleep-1",
            "disableTimeout": true,
            "timeoutMs": null,
            "cwd": null,
            "env": null,
            "size": null,
            "sandboxPolicy": null,
            "permissionProfile": null,
            "outputBytesCap": null,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn process_spawn_params_round_trips_without_sandbox_policy() {
    let params = ProcessSpawnParams {
        command: vec!["sleep".to_string(), "30".to_string()],
        process_handle: "sleep-1".to_string(),
        cwd: test_absolute_path(),
        tty: false,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: None,
        timeout_ms: None,
        env: None,
        size: None,
    };

    let value = serde_json::to_value(&params).expect("serialize process/spawn params");
    assert_eq!(
        value,
        json!({
            "command": ["sleep", "30"],
            "processHandle": "sleep-1",
            "cwd": absolute_path_string("readable"),
            "env": null,
            "size": null,
        })
    );

    let decoded =
        serde_json::from_value::<ProcessSpawnParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn process_spawn_params_distinguish_omitted_null_and_value_limits() {
    let base = json!({
        "command": ["sleep", "30"],
        "processHandle": "sleep-1",
        "cwd": absolute_path_string("readable"),
    });

    let expected_omitted = ProcessSpawnParams {
        command: vec!["sleep".to_string(), "30".to_string()],
        process_handle: "sleep-1".to_string(),
        cwd: test_absolute_path(),
        tty: false,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: None,
        timeout_ms: None,
        env: None,
        size: None,
    };
    let decoded =
        serde_json::from_value::<ProcessSpawnParams>(base).expect("deserialize omitted limits");
    assert_eq!(decoded, expected_omitted);

    let decoded = serde_json::from_value::<ProcessSpawnParams>(json!({
        "command": ["sleep", "30"],
        "processHandle": "sleep-1",
        "cwd": absolute_path_string("readable"),
        "outputBytesCap": null,
        "timeoutMs": null,
    }))
    .expect("deserialize disabled limits");
    assert_eq!(
        decoded,
        ProcessSpawnParams {
            output_bytes_cap: Some(None),
            timeout_ms: Some(None),
            ..expected_omitted.clone()
        }
    );

    let decoded = serde_json::from_value::<ProcessSpawnParams>(json!({
        "command": ["sleep", "30"],
        "processHandle": "sleep-1",
        "cwd": absolute_path_string("readable"),
        "outputBytesCap": 123,
        "timeoutMs": 456,
    }))
    .expect("deserialize explicit limits");
    assert_eq!(
        decoded,
        ProcessSpawnParams {
            output_bytes_cap: Some(Some(123)),
            timeout_ms: Some(Some(456)),
            ..expected_omitted
        }
    );
}

#[test]
fn command_exec_params_round_trips_disable_output_cap() {
    let params = CommandExecParams {
        command: vec!["yes".to_string()],
        process_id: Some("yes-1".to_string()),
        tty: false,
        stream_stdin: false,
        stream_stdout_stderr: true,
        output_bytes_cap: None,
        disable_output_cap: true,
        disable_timeout: false,
        timeout_ms: None,
        cwd: None,
        env: None,
        size: None,
        sandbox_policy: None,
        permission_profile: None,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec params");
    assert_eq!(
        value,
        json!({
            "command": ["yes"],
            "processId": "yes-1",
            "streamStdoutStderr": true,
            "outputBytesCap": null,
            "disableOutputCap": true,
            "timeoutMs": null,
            "cwd": null,
            "env": null,
            "size": null,
            "sandboxPolicy": null,
            "permissionProfile": null,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_params_round_trips_env_overrides_and_unsets() {
    let params = CommandExecParams {
        command: vec!["printenv".to_string(), "FOO".to_string()],
        process_id: Some("env-1".to_string()),
        tty: false,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: None,
        disable_output_cap: false,
        disable_timeout: false,
        timeout_ms: None,
        cwd: None,
        env: Some(HashMap::from([
            ("FOO".to_string(), Some("override".to_string())),
            ("BAR".to_string(), Some("added".to_string())),
            ("BAZ".to_string(), None),
        ])),
        size: None,
        sandbox_policy: None,
        permission_profile: None,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec params");
    assert_eq!(
        value,
        json!({
            "command": ["printenv", "FOO"],
            "processId": "env-1",
            "outputBytesCap": null,
            "timeoutMs": null,
            "cwd": null,
            "env": {
                "FOO": "override",
                "BAR": "added",
                "BAZ": null,
            },
            "size": null,
            "sandboxPolicy": null,
            "permissionProfile": null,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_write_round_trips_close_only_payload() {
    let params = CommandExecWriteParams {
        process_id: "proc-7".to_string(),
        delta_base64: None,
        close_stdin: true,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec/write params");
    assert_eq!(
        value,
        json!({
            "processId": "proc-7",
            "deltaBase64": null,
            "closeStdin": true,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecWriteParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_terminate_round_trips() {
    let params = CommandExecTerminateParams {
        process_id: "proc-8".to_string(),
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec/terminate params");
    assert_eq!(
        value,
        json!({
            "processId": "proc-8",
        })
    );

    let decoded = serde_json::from_value::<CommandExecTerminateParams>(value)
        .expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_params_round_trip_with_size() {
    let params = CommandExecParams {
        command: vec!["top".to_string()],
        process_id: Some("pty-1".to_string()),
        tty: true,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: None,
        disable_output_cap: false,
        disable_timeout: false,
        timeout_ms: None,
        cwd: None,
        env: None,
        size: Some(CommandExecTerminalSize {
            rows: 40,
            cols: 120,
        }),
        sandbox_policy: None,
        permission_profile: None,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec params");
    assert_eq!(
        value,
        json!({
            "command": ["top"],
            "processId": "pty-1",
            "tty": true,
            "outputBytesCap": null,
            "timeoutMs": null,
            "cwd": null,
            "env": null,
            "size": {
                "rows": 40,
                "cols": 120,
            },
            "sandboxPolicy": null,
            "permissionProfile": null,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_resize_round_trips() {
    let params = CommandExecResizeParams {
        process_id: "proc-9".to_string(),
        size: CommandExecTerminalSize {
            rows: 50,
            cols: 160,
        },
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec/resize params");
    assert_eq!(
        value,
        json!({
            "processId": "proc-9",
            "size": {
                "rows": 50,
                "cols": 160,
            },
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecResizeParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_output_delta_round_trips() {
    let notification = CommandExecOutputDeltaNotification {
        process_id: "proc-1".to_string(),
        stream: CommandExecOutputStream::Stdout,
        delta_base64: "AQI=".to_string(),
        cap_reached: false,
    };

    let value = serde_json::to_value(&notification)
        .expect("serialize command/exec/outputDelta notification");
    assert_eq!(
        value,
        json!({
            "processId": "proc-1",
            "stream": "stdout",
            "deltaBase64": "AQI=",
            "capReached": false,
        })
    );

    let decoded = serde_json::from_value::<CommandExecOutputDeltaNotification>(value)
        .expect("deserialize round-trip");
    assert_eq!(decoded, notification);
}

#[test]
fn process_control_params_round_trip() {
    let write = ProcessWriteStdinParams {
        process_handle: "proc-7".to_string(),
        delta_base64: None,
        close_stdin: true,
    };
    let value = serde_json::to_value(&write).expect("serialize process/writeStdin params");
    assert_eq!(
        value,
        json!({
            "processHandle": "proc-7",
            "deltaBase64": null,
            "closeStdin": true,
        })
    );
    let decoded = serde_json::from_value::<ProcessWriteStdinParams>(value)
        .expect("deserialize process/writeStdin params");
    assert_eq!(decoded, write);

    let resize = ProcessResizePtyParams {
        process_handle: "proc-7".to_string(),
        size: ProcessTerminalSize {
            rows: 50,
            cols: 160,
        },
    };
    let value = serde_json::to_value(&resize).expect("serialize process/resizePty params");
    assert_eq!(
        value,
        json!({
            "processHandle": "proc-7",
            "size": {
                "rows": 50,
                "cols": 160,
            },
        })
    );
    let decoded = serde_json::from_value::<ProcessResizePtyParams>(value)
        .expect("deserialize process/resizePty params");
    assert_eq!(decoded, resize);

    let kill = ProcessKillParams {
        process_handle: "proc-7".to_string(),
    };
    let value = serde_json::to_value(&kill).expect("serialize process/kill params");
    assert_eq!(
        value,
        json!({
            "processHandle": "proc-7",
        })
    );
    let decoded =
        serde_json::from_value::<ProcessKillParams>(value).expect("deserialize process/kill");
    assert_eq!(decoded, kill);
}

#[test]
fn process_notifications_round_trip() {
    let delta = ProcessOutputDeltaNotification {
        process_handle: "proc-1".to_string(),
        stream: ProcessOutputStream::Stdout,
        delta_base64: "AQI=".to_string(),
        cap_reached: false,
    };
    let value = serde_json::to_value(&delta).expect("serialize process/outputDelta");
    assert_eq!(
        value,
        json!({
            "processHandle": "proc-1",
            "stream": "stdout",
            "deltaBase64": "AQI=",
            "capReached": false,
        })
    );
    let decoded = serde_json::from_value::<ProcessOutputDeltaNotification>(value)
        .expect("deserialize process/outputDelta");
    assert_eq!(decoded, delta);

    let exited = ProcessExitedNotification {
        process_handle: "proc-1".to_string(),
        exit_code: 0,
        stdout: "out".to_string(),
        stdout_cap_reached: false,
        stderr: "err".to_string(),
        stderr_cap_reached: true,
    };
    let value = serde_json::to_value(&exited).expect("serialize process/exited");
    assert_eq!(
        value,
        json!({
            "processHandle": "proc-1",
            "exitCode": 0,
            "stdout": "out",
            "stdoutCapReached": false,
            "stderr": "err",
            "stderrCapReached": true,
        })
    );
    let decoded = serde_json::from_value::<ProcessExitedNotification>(value)
        .expect("deserialize process/exited");
    assert_eq!(decoded, exited);
}

#[test]
fn command_execution_output_delta_round_trips() {
    let notification = CommandExecutionOutputDeltaNotification {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        item_id: "item-1".to_string(),
        delta: "\u{fffd}a\n".to_string(),
    };

    let value = serde_json::to_value(&notification)
        .expect("serialize item/commandExecution/outputDelta notification");
    assert_eq!(
        value,
        json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "item-1",
            "delta": "\u{fffd}a\n",
        })
    );

    let decoded = serde_json::from_value::<CommandExecutionOutputDeltaNotification>(value)
        .expect("deserialize round-trip");
    assert_eq!(decoded, notification);
}
