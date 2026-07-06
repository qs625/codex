use super::*;

#[test]
fn command_execution_request_approval_rejects_relative_additional_permission_paths() {
    let err = serde_json::from_value::<CommandExecutionRequestApprovalParams>(json!({
        "threadId": "thr_123",
        "turnId": "turn_123",
        "itemId": "call_123",
        "startedAtMs": 1,
        "command": "cat file",
        "cwd": absolute_path_string("tmp"),
        "commandActions": null,
        "reason": null,
        "networkApprovalContext": null,
        "additionalPermissions": {
            "network": null,
            "fileSystem": {
                "read": ["relative/path"],
                "write": null
            }
        },
        "proposedExecpolicyAmendment": null,
        "proposedNetworkPolicyAmendments": null,
        "availableDecisions": null
    }))
    .expect_err("relative additional permission paths should fail");
    assert!(
        err.to_string()
            .contains("AbsolutePathBuf deserialized without a base path"),
        "unexpected error: {err}"
    );
}

#[test]
fn permissions_request_approval_uses_request_permission_profile() {
    let read_only_path = if cfg!(windows) {
        r"C:\tmp\read-only"
    } else {
        "/tmp/read-only"
    };
    let read_write_path = if cfg!(windows) {
        r"C:\tmp\read-write"
    } else {
        "/tmp/read-write"
    };
    let params = serde_json::from_value::<PermissionsRequestApprovalParams>(json!({
        "threadId": "thr_123",
        "turnId": "turn_123",
        "itemId": "call_123",
        "startedAtMs": 1,
        "cwd": absolute_path_string("repo"),
        "reason": "Select a workspace root",
        "permissions": {
            "network": {
                "enabled": true,
            },
            "fileSystem": {
                "read": [read_only_path],
                "write": [read_write_path],
            },
        },
    }))
    .expect("permissions request should deserialize");

    assert_eq!(params.cwd, absolute_path("repo"));
    assert_eq!(
        params.permissions,
        RequestPermissionProfile {
            network: Some(AdditionalNetworkPermissions {
                enabled: Some(true),
            }),
            file_system: Some(AdditionalFileSystemPermissions {
                read: Some(vec![
                    AbsolutePathBuf::try_from(PathBuf::from(read_only_path))
                        .expect("path must be absolute"),
                ]),
                write: Some(vec![
                    AbsolutePathBuf::try_from(PathBuf::from(read_write_path))
                        .expect("path must be absolute"),
                ]),
                glob_scan_max_depth: None,
                entries: None,
            }),
        }
    );

    assert_eq!(
        CoreRequestPermissionProfile::from(params.permissions),
        CoreRequestPermissionProfile {
            network: Some(CoreNetworkPermissions {
                enabled: Some(true),
            }),
            file_system: Some(CoreFileSystemPermissions::from_read_write_roots(
                Some(vec![
                    AbsolutePathBuf::try_from(PathBuf::from(read_only_path))
                        .expect("path must be absolute"),
                ]),
                Some(vec![
                    AbsolutePathBuf::try_from(PathBuf::from(read_write_path))
                        .expect("path must be absolute"),
                ]),
            )),
        }
    );
}

#[test]
fn permissions_request_approval_rejects_macos_permissions() {
    let err = serde_json::from_value::<PermissionsRequestApprovalParams>(json!({
        "threadId": "thr_123",
        "turnId": "turn_123",
        "itemId": "call_123",
        "startedAtMs": 1,
        "cwd": absolute_path_string("repo"),
        "reason": "Select a workspace root",
        "permissions": {
            "network": null,
            "fileSystem": null,
            "macos": {
                "preferences": "read_only",
                "automations": "none",
                "launchServices": false,
                "accessibility": false,
                "calendar": false,
                "reminders": false,
                "contacts": "none",
            },
        },
    }))
    .expect_err("permissions request should reject macos permissions");

    assert!(
        err.to_string().contains("unknown field `macos`"),
        "unexpected error: {err}"
    );
}

#[test]
fn additional_file_system_permissions_preserves_canonical_entries() {
    let core_permissions = CoreFileSystemPermissions {
        entries: vec![
            CoreFileSystemSandboxEntry {
                path: CoreFileSystemPath::Special {
                    value: CoreFileSystemSpecialPath::Root,
                },
                access: CoreFileSystemAccessMode::Write,
            },
            CoreFileSystemSandboxEntry {
                path: CoreFileSystemPath::GlobPattern {
                    pattern: "**/*.env".to_string(),
                },
                access: CoreFileSystemAccessMode::None,
            },
        ],
        glob_scan_max_depth: NonZeroUsize::new(2),
    };

    let permissions = AdditionalFileSystemPermissions::from(core_permissions.clone());
    assert_eq!(
        permissions,
        AdditionalFileSystemPermissions {
            read: None,
            write: None,
            glob_scan_max_depth: NonZeroUsize::new(2),
            entries: Some(vec![
                FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::Root,
                    },
                    access: FileSystemAccessMode::Write,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::GlobPattern {
                        pattern: "**/*.env".to_string(),
                    },
                    access: FileSystemAccessMode::None,
                },
            ]),
        }
    );
    assert_eq!(
        CoreFileSystemPermissions::from(permissions),
        core_permissions
    );
}

#[test]
fn additional_file_system_permissions_populates_entries_for_legacy_roots() {
    let read_only_path = absolute_path("read-only");
    let read_write_path = absolute_path("read-write");
    let core_permissions = CoreFileSystemPermissions::from_read_write_roots(
        Some(vec![read_only_path.clone()]),
        Some(vec![read_write_path.clone()]),
    );

    let permissions = AdditionalFileSystemPermissions::from(core_permissions.clone());

    assert_eq!(
        permissions,
        AdditionalFileSystemPermissions {
            read: Some(vec![read_only_path.clone()]),
            write: Some(vec![read_write_path.clone()]),
            glob_scan_max_depth: None,
            entries: Some(vec![
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: read_only_path,
                    },
                    access: FileSystemAccessMode::Read,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: read_write_path,
                    },
                    access: FileSystemAccessMode::Write,
                },
            ]),
        }
    );
    assert_eq!(
        CoreFileSystemPermissions::from(permissions),
        core_permissions
    );
}

#[test]
fn additional_file_system_permissions_rejects_zero_glob_scan_depth() {
    serde_json::from_value::<AdditionalFileSystemPermissions>(json!({
        "read": null,
        "write": null,
        "globScanMaxDepth": 0,
        "entries": [],
    }))
    .expect_err("zero glob scan depth should fail deserialization");
}

#[test]
fn permission_profile_file_system_permissions_preserves_glob_scan_depth() {
    let core_permissions = CoreManagedFileSystemPermissions::Restricted {
        entries: vec![CoreFileSystemSandboxEntry {
            path: CoreFileSystemPath::GlobPattern {
                pattern: "**/*.env".to_string(),
            },
            access: CoreFileSystemAccessMode::None,
        }],
        glob_scan_max_depth: NonZeroUsize::new(2),
    };

    let permissions = PermissionProfileFileSystemPermissions::from(core_permissions.clone());

    assert_eq!(
        permissions,
        PermissionProfileFileSystemPermissions::Restricted {
            entries: vec![FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: "**/*.env".to_string(),
                },
                access: FileSystemAccessMode::None,
            }],
            glob_scan_max_depth: NonZeroUsize::new(2),
        }
    );
    assert_eq!(
        CoreManagedFileSystemPermissions::from(permissions),
        core_permissions
    );
}

#[test]
fn permission_profile_file_system_permissions_rejects_zero_glob_scan_depth() {
    serde_json::from_value::<PermissionProfileFileSystemPermissions>(json!({
        "type": "restricted",
        "entries": [],
        "globScanMaxDepth": 0,
    }))
    .expect_err("zero glob scan depth should fail deserialization");
}

#[test]
fn legacy_current_working_directory_special_path_deserializes_as_project_roots() {
    let special_path = serde_json::from_value::<FileSystemSpecialPath>(json!({
        "kind": "current_working_directory",
    }))
    .expect("legacy cwd special path should deserialize");

    assert_eq!(
        special_path,
        FileSystemSpecialPath::ProjectRoots { subpath: None }
    );
    assert_eq!(
        serde_json::to_value(&special_path).expect("serialize special path"),
        json!({
            "kind": "project_roots",
            "subpath": null,
        })
    );
}

#[test]
fn permissions_request_approval_response_uses_granted_permission_profile_without_macos() {
    let read_only_path = if cfg!(windows) {
        r"C:\tmp\read-only"
    } else {
        "/tmp/read-only"
    };
    let read_write_path = if cfg!(windows) {
        r"C:\tmp\read-write"
    } else {
        "/tmp/read-write"
    };
    let response = serde_json::from_value::<PermissionsRequestApprovalResponse>(json!({
        "permissions": {
            "network": {
                "enabled": true,
            },
            "fileSystem": {
                "read": [read_only_path],
                "write": [read_write_path],
            },
        },
    }))
    .expect("permissions response should deserialize");

    assert_eq!(
        response.permissions,
        GrantedPermissionProfile {
            network: Some(AdditionalNetworkPermissions {
                enabled: Some(true),
            }),
            file_system: Some(AdditionalFileSystemPermissions {
                read: Some(vec![
                    AbsolutePathBuf::try_from(PathBuf::from(read_only_path))
                        .expect("path must be absolute"),
                ]),
                write: Some(vec![
                    AbsolutePathBuf::try_from(PathBuf::from(read_write_path))
                        .expect("path must be absolute"),
                ]),
                glob_scan_max_depth: None,
                entries: None,
            }),
        }
    );

    assert_eq!(
        CoreAdditionalPermissionProfile::from(response.permissions),
        CoreAdditionalPermissionProfile {
            network: Some(CoreNetworkPermissions {
                enabled: Some(true),
            }),
            file_system: Some(CoreFileSystemPermissions::from_read_write_roots(
                Some(vec![
                    AbsolutePathBuf::try_from(PathBuf::from(read_only_path))
                        .expect("path must be absolute"),
                ]),
                Some(vec![
                    AbsolutePathBuf::try_from(PathBuf::from(read_write_path))
                        .expect("path must be absolute"),
                ]),
            )),
        }
    );
}

#[test]
fn permissions_request_approval_response_defaults_scope_to_turn() {
    let response = serde_json::from_value::<PermissionsRequestApprovalResponse>(json!({
        "permissions": {},
    }))
    .expect("response should deserialize");

    assert_eq!(response.scope, PermissionGrantScope::Turn);
    assert_eq!(response.strict_auto_review, None);
}

#[test]
fn permissions_request_approval_response_accepts_strict_auto_review() {
    let response = serde_json::from_value::<PermissionsRequestApprovalResponse>(json!({
        "permissions": {},
        "strictAutoReview": true,
    }))
    .expect("response should deserialize");

    assert_eq!(response.strict_auto_review, Some(true));
}

#[test]
fn permission_profile_selection_accepts_legacy_object_shape() {
    let additional_root = absolute_path("additional-root");
    let params = json!({
        "permissions": {
            "type": "profile",
            "id": ":workspace",
            "modifications": [
                {
                    "type": "additionalWritableRoot",
                    "path": additional_root,
                }
            ],
        },
    });

    let start: ThreadStartParams =
        serde_json::from_value(params.clone()).expect("thread/start params deserialize");
    assert_legacy_permission_profile_selection(start.permissions, &additional_root);

    let resume: ThreadResumeParams = serde_json::from_value(json!({
        "threadId": "thread-1",
        "permissions": params["permissions"].clone(),
    }))
    .expect("thread/resume params deserialize");
    assert_legacy_permission_profile_selection(resume.permissions, &additional_root);

    let fork: ThreadForkParams = serde_json::from_value(json!({
        "threadId": "thread-1",
        "permissions": params["permissions"].clone(),
    }))
    .expect("thread/fork params deserialize");
    assert_legacy_permission_profile_selection(fork.permissions, &additional_root);

    let turn: TurnStartParams = serde_json::from_value(json!({
        "threadId": "thread-1",
        "input": [],
        "permissions": params["permissions"].clone(),
    }))
    .expect("turn/start params deserialize");
    assert_legacy_permission_profile_selection(turn.permissions, &additional_root);
}

fn assert_legacy_permission_profile_selection(
    selection: Option<PermissionProfileSelectionParams>,
    additional_root: &AbsolutePathBuf,
) {
    let selection = selection.expect("permissions should be present");
    assert_eq!(selection.id(), ":workspace");
    assert_eq!(
        selection.legacy_additional_writable_roots(),
        std::slice::from_ref(additional_root)
    );
}
