use super::*;

    #[test]
    fn file_change_accept_for_session_maps_to_approved_for_session() {
        let decision =
            map_file_change_approval_decision(FileChangeApprovalDecision::AcceptForSession);
        assert_eq!(decision, ReviewDecision::ApprovedForSession);
    }

    #[test]
    fn mcp_server_elicitation_turn_transition_error_maps_to_cancel() {
        let error = JSONRPCErrorError {
            code: -1,
            message: "client request resolved because the turn state was changed".to_string(),
            data: Some(serde_json::json!({ "reason": "turnTransition" })),
        };

        let response = mcp_server_elicitation_response_from_client_result(Ok(Err(error)));

        assert_eq!(
            response,
            McpServerElicitationRequestResponse {
                action: McpServerElicitationAction::Cancel,
                content: None,
                meta: None,
            }
        );
    }

    #[test]
    fn request_permissions_turn_transition_error_is_ignored() {
        let error = JSONRPCErrorError {
            code: -1,
            message: "client request resolved because the turn state was changed".to_string(),
            data: Some(serde_json::json!({ "reason": "turnTransition" })),
        };

        let response = request_permissions_response_from_client_result(
            CoreRequestPermissionProfile::default(),
            Ok(Err(error)),
            std::env::current_dir().expect("current dir").as_path(),
        );

        assert_eq!(response, None);
    }

    #[test]
    fn request_permissions_response_accepts_partial_network_and_file_system_grants() {
        let input_path = if cfg!(target_os = "windows") {
            r"C:\tmp\input"
        } else {
            "/tmp/input"
        };
        let output_path = if cfg!(target_os = "windows") {
            r"C:\tmp\output"
        } else {
            "/tmp/output"
        };
        let ignored_path = if cfg!(target_os = "windows") {
            r"C:\tmp\ignored"
        } else {
            "/tmp/ignored"
        };
        let absolute_path = |path: &str| {
            AbsolutePathBuf::try_from(std::path::PathBuf::from(path)).expect("absolute path")
        };
        let requested_permissions = CoreRequestPermissionProfile {
            network: Some(CoreNetworkPermissions {
                enabled: Some(true),
            }),
            file_system: Some(CoreFileSystemPermissions::from_read_write_roots(
                Some(vec![absolute_path(input_path)]),
                Some(vec![absolute_path(output_path)]),
            )),
        };
        let cases = vec![
            (
                serde_json::json!({}),
                CoreRequestPermissionProfile::default(),
            ),
            (
                serde_json::json!({
                    "network": {
                        "enabled": true,
                    },
                }),
                CoreRequestPermissionProfile {
                    network: Some(CoreNetworkPermissions {
                        enabled: Some(true),
                    }),
                    ..CoreRequestPermissionProfile::default()
                },
            ),
            (
                serde_json::json!({
                    "fileSystem": {
                        "write": [output_path],
                    },
                }),
                CoreRequestPermissionProfile {
                    file_system: Some(CoreFileSystemPermissions::from_read_write_roots(
                        /*read*/ None,
                        Some(vec![absolute_path(output_path)]),
                    )),
                    ..CoreRequestPermissionProfile::default()
                },
            ),
            (
                serde_json::json!({
                    "fileSystem": {
                        "read": [input_path],
                        "write": [output_path, ignored_path],
                    },
                    "macos": {
                        "calendar": true,
                    },
                }),
                CoreRequestPermissionProfile {
                    file_system: Some(CoreFileSystemPermissions::from_read_write_roots(
                        Some(vec![absolute_path(input_path)]),
                        Some(vec![absolute_path(output_path)]),
                    )),
                    ..CoreRequestPermissionProfile::default()
                },
            ),
        ];

        let cwd = std::env::current_dir().expect("current dir");
        for (granted_permissions, expected_permissions) in cases {
            let response = request_permissions_response_from_client_result(
                requested_permissions.clone(),
                Ok(Ok(serde_json::json!({
                    "permissions": granted_permissions,
                }))),
                cwd.as_path(),
            )
            .expect("response should be accepted");

            assert_eq!(
                response,
                CoreRequestPermissionsResponse {
                    permissions: expected_permissions,
                    scope: CorePermissionGrantScope::Turn,
                    strict_auto_review: false,
                }
            );
        }
    }

    #[test]
    fn request_permissions_response_preserves_session_scope() {
        let response = request_permissions_response_from_client_result(
            CoreRequestPermissionProfile::default(),
            Ok(Ok(serde_json::json!({
                "scope": "session",
                "permissions": {},
            }))),
            std::env::current_dir().expect("current dir").as_path(),
        )
        .expect("response should be accepted");

        assert_eq!(
            response,
            CoreRequestPermissionsResponse {
                permissions: CoreRequestPermissionProfile::default(),
                scope: CorePermissionGrantScope::Session,
                strict_auto_review: false,
            }
        );
    }

    #[test]
    fn request_permissions_response_rejects_session_scoped_strict_auto_review() {
        let response = request_permissions_response_from_client_result(
            CoreRequestPermissionProfile::default(),
            Ok(Ok(serde_json::json!({
                "scope": "session",
                "strictAutoReview": true,
                "permissions": {
                    "network": {
                        "enabled": true,
                    },
                },
            }))),
            std::env::current_dir().expect("current dir").as_path(),
        )
        .expect("response should be accepted");

        assert_eq!(
            response,
            CoreRequestPermissionsResponse {
                permissions: CoreRequestPermissionProfile::default(),
                scope: CorePermissionGrantScope::Turn,
                strict_auto_review: false,
            }
        );
    }

    #[test]
    fn request_permissions_response_preserves_turn_scoped_strict_auto_review() {
        let response = request_permissions_response_from_client_result(
            CoreRequestPermissionProfile {
                network: Some(protocol::models::NetworkPermissions {
                    enabled: Some(true),
                }),
                ..Default::default()
            },
            Ok(Ok(serde_json::json!({
                "strictAutoReview": true,
                "permissions": {
                    "network": {
                        "enabled": true,
                    },
                },
            }))),
            std::env::current_dir().expect("current dir").as_path(),
        )
        .expect("response should be accepted");

        assert_eq!(response.scope, CorePermissionGrantScope::Turn);
        assert!(response.strict_auto_review);
    }

    #[test]
    fn request_permissions_response_accepts_explicit_child_grant_for_requested_cwd_scope() {
        let temp_dir = TempDir::new().expect("temp dir");
        let cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path()).expect("absolute cwd");
        let child = cwd.join("child");
        let requested_permissions = CoreRequestPermissionProfile {
            file_system: Some(CoreFileSystemPermissions {
                entries: vec![FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                    },
                    access: FileSystemAccessMode::Write,
                }],
                glob_scan_max_depth: None,
            }),
            ..Default::default()
        };

        let response = request_permissions_response_from_client_result(
            requested_permissions,
            Ok(Ok(serde_json::json!({
                "permissions": {
                    "fileSystem": {
                        "write": [child],
                    },
                },
            }))),
            cwd.as_path(),
        )
        .expect("response should be accepted");

        assert_eq!(
            response.permissions,
            CoreRequestPermissionProfile {
                file_system: Some(CoreFileSystemPermissions::from_read_write_roots(
                    /*read*/ None,
                    Some(vec![child]),
                )),
                ..Default::default()
            }
        );
    }

    #[test]
    fn request_permissions_response_rejects_child_grant_outside_requested_cwd_scope() {
        let temp_dir = TempDir::new().expect("temp dir");
        let request_cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path().join("request-cwd"))
            .expect("absolute request cwd");
        let later_cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path().join("later-cwd"))
            .expect("absolute later cwd");
        let later_child = later_cwd.join("child");
        let requested_permissions = CoreRequestPermissionProfile {
            file_system: Some(CoreFileSystemPermissions {
                entries: vec![FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                    },
                    access: FileSystemAccessMode::Write,
                }],
                glob_scan_max_depth: None,
            }),
            ..Default::default()
        };

        let response = request_permissions_response_from_client_result(
            requested_permissions,
            Ok(Ok(serde_json::json!({
                "permissions": {
                    "fileSystem": {
                        "write": [later_child],
                    },
                },
            }))),
            request_cwd.as_path(),
        )
        .expect("response should be accepted");

        assert_eq!(
            response.permissions,
            CoreRequestPermissionProfile::default()
        );
    }

    #[test]
    fn request_permissions_response_ignores_broader_cwd_grant_for_requested_child_path() {
        let temp_dir = TempDir::new().expect("temp dir");
        let cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path()).expect("absolute cwd");
        let child = cwd.join("child");
        let requested_permissions = CoreRequestPermissionProfile {
            file_system: Some(CoreFileSystemPermissions::from_read_write_roots(
                /*read*/ None,
                Some(vec![child]),
            )),
            ..Default::default()
        };

        let response = request_permissions_response_from_client_result(
            requested_permissions,
            Ok(Ok(serde_json::json!({
                "permissions": {
                    "fileSystem": {
                        "entries": [{
                            "path": {
                                "type": "special",
                                "value": {
                                    "kind": "project_roots",
                                    "subpath": null
                                }
                            },
                            "access": "write"
                        }],
                    },
                },
            }))),
            cwd.as_path(),
        )
        .expect("response should be accepted");

        assert_eq!(
            response.permissions,
            CoreRequestPermissionProfile::default()
        );
    }

