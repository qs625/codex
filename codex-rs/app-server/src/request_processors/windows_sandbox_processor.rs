use super::*;

#[derive(Clone)]
pub(crate) struct WindowsSandboxRequestProcessor {
    outgoing: Arc<OutgoingMessageSender>,
    config: Arc<Config>,
    config_manager: ConfigManager,
}

impl WindowsSandboxRequestProcessor {
    pub(crate) fn new(
        outgoing: Arc<OutgoingMessageSender>,
        config: Arc<Config>,
        config_manager: ConfigManager,
    ) -> Self {
        Self {
            outgoing,
            config,
            config_manager,
        }
    }

    pub(crate) async fn windows_sandbox_readiness(
        &self,
    ) -> Result<WindowsSandboxReadinessResponse, JSONRPCErrorError> {
        Ok(determine_windows_sandbox_readiness(&self.config))
    }

    pub(crate) async fn windows_sandbox_setup_start(
        &self,
        request_id: &ConnectionRequestId,
        params: WindowsSandboxSetupStartParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.windows_sandbox_setup_start_inner(request_id, params)
            .await
            .map(|()| None)
    }

    async fn windows_sandbox_setup_start_inner(
        &self,
        request_id: &ConnectionRequestId,
        params: WindowsSandboxSetupStartParams,
    ) -> Result<(), JSONRPCErrorError> {
        self.outgoing
            .send_response(
                request_id.clone(),
                WindowsSandboxSetupStartResponse { started: true },
            )
            .await;

        let setup_plan = WindowsSandboxSetupStartPlan::from_params(
            request_id,
            params,
            self.config.cwd.as_path(),
        );
        let config_manager = self.config_manager.clone();
        let outgoing = Arc::clone(&self.outgoing);

        tokio::spawn(async move {
            let derived_config = config_manager
                .load_for_cwd(
                    /*request_overrides*/ None,
                    ConfigOverrides {
                        cwd: Some(setup_plan.command_cwd.clone()),
                        ..Default::default()
                    },
                    Some(setup_plan.command_cwd.clone()),
                )
                .await;
            let setup_result = match derived_config {
                Ok(config) => {
                    let setup_request = setup_plan.setup_request(&config, std::env::vars());
                    codex_sandboxing::run_windows_sandbox_setup(setup_request).await
                }
                Err(err) => Err(err.into()),
            };
            let notification =
                setup_plan.completed_notification(setup_result.map_err(|err| err.to_string()));
            outgoing
                .send_server_notification_to_connections(
                    &[setup_plan.connection_id],
                    ServerNotification::WindowsSandboxSetupCompleted(notification),
                )
                .await;
        });
        Ok(())
    }
}

struct WindowsSandboxSetupStartPlan {
    mode: CoreWindowsSandboxSetupMode,
    command_cwd: PathBuf,
    connection_id: ConnectionId,
}

impl WindowsSandboxSetupStartPlan {
    fn from_params(
        request_id: &ConnectionRequestId,
        params: WindowsSandboxSetupStartParams,
        default_cwd: &Path,
    ) -> Self {
        Self {
            mode: core_windows_sandbox_setup_mode(params.mode),
            command_cwd: setup_command_cwd(params.cwd, default_cwd),
            connection_id: request_id.connection_id,
        }
    }

    fn setup_request(
        &self,
        config: &Config,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> WindowsSandboxSetupRequest {
        WindowsSandboxSetupRequest {
            mode: self.mode,
            policy: config
                .permissions
                .legacy_sandbox_policy(config.cwd.as_path()),
            policy_cwd: config.cwd.to_path_buf(),
            command_cwd: self.command_cwd.clone(),
            env_map: env.into_iter().collect(),
            codex_home: config.codex_home.to_path_buf(),
            active_profile: config.active_profile.clone(),
        }
    }

    fn completed_notification(
        &self,
        setup_result: Result<(), String>,
    ) -> WindowsSandboxSetupCompletedNotification {
        WindowsSandboxSetupCompletedNotification {
            mode: api_windows_sandbox_setup_mode(self.mode),
            success: setup_result.is_ok(),
            error: setup_result.err(),
        }
    }
}

fn setup_command_cwd(cwd: Option<AbsolutePathBuf>, default_cwd: &Path) -> PathBuf {
    cwd.map(PathBuf::from)
        .unwrap_or_else(|| default_cwd.to_path_buf())
}

fn core_windows_sandbox_setup_mode(mode: WindowsSandboxSetupMode) -> CoreWindowsSandboxSetupMode {
    match mode {
        WindowsSandboxSetupMode::Elevated => CoreWindowsSandboxSetupMode::Elevated,
        WindowsSandboxSetupMode::Unelevated => CoreWindowsSandboxSetupMode::Unelevated,
    }
}

fn api_windows_sandbox_setup_mode(mode: CoreWindowsSandboxSetupMode) -> WindowsSandboxSetupMode {
    match mode {
        CoreWindowsSandboxSetupMode::Elevated => WindowsSandboxSetupMode::Elevated,
        CoreWindowsSandboxSetupMode::Unelevated => WindowsSandboxSetupMode::Unelevated,
    }
}

fn determine_windows_sandbox_readiness(config: &Config) -> WindowsSandboxReadinessResponse {
    if !cfg!(windows) {
        return WindowsSandboxReadinessResponse {
            status: WindowsSandboxReadiness::NotConfigured,
        };
    }

    determine_windows_sandbox_readiness_from_state(
        WindowsSandboxLevel::from_config(config),
        sandbox_setup_is_complete(config.codex_home.as_path()),
    )
}

fn determine_windows_sandbox_readiness_from_state(
    windows_sandbox_level: WindowsSandboxLevel,
    sandbox_setup_is_complete: bool,
) -> WindowsSandboxReadinessResponse {
    let status = match windows_sandbox_level {
        WindowsSandboxLevel::Disabled => WindowsSandboxReadiness::NotConfigured,
        WindowsSandboxLevel::RestrictedToken => WindowsSandboxReadiness::Ready,
        WindowsSandboxLevel::Elevated => {
            if sandbox_setup_is_complete {
                WindowsSandboxReadiness::Ready
            } else {
                WindowsSandboxReadiness::UpdateRequired
            }
        }
    };

    WindowsSandboxReadinessResponse { status }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determine_windows_sandbox_readiness_reports_not_configured_when_disabled() {
        let response = determine_windows_sandbox_readiness_from_state(
            WindowsSandboxLevel::Disabled,
            /*sandbox_setup_is_complete*/ false,
        );

        assert_eq!(response.status, WindowsSandboxReadiness::NotConfigured);
    }

    #[test]
    fn determine_windows_sandbox_readiness_reports_ready_for_unelevated_mode() {
        let response = determine_windows_sandbox_readiness_from_state(
            WindowsSandboxLevel::RestrictedToken,
            /*sandbox_setup_is_complete*/ false,
        );

        assert_eq!(response.status, WindowsSandboxReadiness::Ready);
    }

    #[test]
    fn determine_windows_sandbox_readiness_reports_ready_for_complete_elevated_mode() {
        let response = determine_windows_sandbox_readiness_from_state(
            WindowsSandboxLevel::Elevated,
            /*sandbox_setup_is_complete*/ true,
        );

        assert_eq!(response.status, WindowsSandboxReadiness::Ready);
    }

    #[test]
    fn determine_windows_sandbox_readiness_reports_update_required_when_elevated_setup_is_stale() {
        let response = determine_windows_sandbox_readiness_from_state(
            WindowsSandboxLevel::Elevated,
            /*sandbox_setup_is_complete*/ false,
        );

        assert_eq!(response.status, WindowsSandboxReadiness::UpdateRequired);
    }

    #[test]
    fn setup_command_cwd_uses_explicit_or_default_cwd() {
        let default_cwd = if cfg!(windows) {
            PathBuf::from("C:\\workspace")
        } else {
            PathBuf::from("/workspace")
        };
        let explicit_cwd = if cfg!(windows) {
            AbsolutePathBuf::try_from(PathBuf::from("C:\\other-workspace")).expect("absolute path")
        } else {
            AbsolutePathBuf::try_from(PathBuf::from("/other-workspace")).expect("absolute path")
        };

        assert_eq!(
            setup_command_cwd(Some(explicit_cwd.clone()), default_cwd.as_path()),
            explicit_cwd.to_path_buf()
        );
        assert_eq!(setup_command_cwd(None, default_cwd.as_path()), default_cwd);
    }

    #[test]
    fn setup_mode_projection_round_trips_api_and_core_modes() {
        assert_eq!(
            api_windows_sandbox_setup_mode(core_windows_sandbox_setup_mode(
                WindowsSandboxSetupMode::Elevated,
            )),
            WindowsSandboxSetupMode::Elevated
        );
        assert_eq!(
            api_windows_sandbox_setup_mode(core_windows_sandbox_setup_mode(
                WindowsSandboxSetupMode::Unelevated,
            )),
            WindowsSandboxSetupMode::Unelevated
        );
    }

    #[test]
    fn setup_plan_projects_completion_notifications() {
        let request_id = ConnectionRequestId {
            connection_id: ConnectionId(7),
            request_id: RequestId::Integer(1),
        };
        let plan = WindowsSandboxSetupStartPlan::from_params(
            &request_id,
            WindowsSandboxSetupStartParams {
                mode: WindowsSandboxSetupMode::Unelevated,
                cwd: None,
            },
            Path::new(if cfg!(windows) {
                "C:\\workspace"
            } else {
                "/workspace"
            }),
        );

        assert_eq!(plan.connection_id, ConnectionId(7));
        let success = plan.completed_notification(Ok(()));
        assert_eq!(success.mode, WindowsSandboxSetupMode::Unelevated);
        assert!(success.success);
        assert_eq!(success.error, None);

        let failed = plan.completed_notification(Err("setup failed".to_string()));
        assert_eq!(failed.mode, WindowsSandboxSetupMode::Unelevated);
        assert!(!failed.success);
        assert_eq!(failed.error, Some("setup failed".to_string()));
    }
}
