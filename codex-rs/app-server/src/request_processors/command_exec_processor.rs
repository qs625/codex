use super::*;

#[derive(Clone)]
pub(crate) struct CommandExecRequestProcessor {
    arg0_paths: Arg0DispatchPaths,
    config: Arc<Config>,
    outgoing: Arc<OutgoingMessageSender>,
    command_exec_manager: CommandExecManager,
    sandbox_runtime: codex_sandboxing_api::SharedSandboxRuntime,
    thread_service: Arc<ThreadService>,
}

impl CommandExecRequestProcessor {
    pub(crate) fn new(
        arg0_paths: Arg0DispatchPaths,
        config: Arc<Config>,
        outgoing: Arc<OutgoingMessageSender>,
        sandbox_runtime: codex_sandboxing_api::SharedSandboxRuntime,
        thread_service: Arc<ThreadService>,
    ) -> Self {
        Self {
            arg0_paths,
            config,
            outgoing,
            command_exec_manager: CommandExecManager::default(),
            sandbox_runtime,
            thread_service,
        }
    }

    pub(crate) async fn one_off_command_exec(
        &self,
        request_id: &ConnectionRequestId,
        params: CommandExecParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.exec_one_off_command(request_id, params)
            .await
            .map(|()| None)
    }

    pub(crate) async fn command_exec_write(
        &self,
        request_id: ConnectionRequestId,
        params: CommandExecWriteParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.command_exec_manager
            .write(request_id, params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn command_exec_resize(
        &self,
        request_id: ConnectionRequestId,
        params: CommandExecResizeParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.command_exec_manager
            .resize(request_id, params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn command_exec_terminate(
        &self,
        request_id: ConnectionRequestId,
        params: CommandExecTerminateParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.command_exec_manager
            .terminate(request_id, params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn terminal_session_list(
        &self,
        request_id: ConnectionRequestId,
        params: TerminalSessionListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let mut data = self
            .command_exec_manager
            .list(request_id.connection_id, &params.user_resume_tokens)
            .await
            .into_iter()
            .map(|session| TerminalSessionDescriptor {
                session_id: format!("user:{}", session.process_id),
                generation: session.generation,
                origin: TerminalSessionOrigin::User,
                thread_id: None,
                command_item_id: None,
                process_id: session.process_id,
                title: session.command.join(" "),
                cwd: session.cwd,
                replay_base64: session.replay_base64,
                replay_truncated: session.replay_truncated,
                replay_through_sequence: session.replay_through_sequence,
                can_resize: true,
                can_write: true,
                can_terminate: true,
            })
            .collect::<Vec<_>>();
        if let Some(thread_id) = params.thread_id {
            let parsed_thread_id = protocol::ThreadId::from_string(&thread_id)
                .map_err(|err| invalid_params(format!("invalid threadId: {err}")))?;
            let commands = self
                .thread_service
                .live_terminal_commands(parsed_thread_id)
                .await
                .map_err(|err| invalid_request(err.to_string()))?;
            data.extend(commands.into_iter().map(|command| {
                let process_id = command.process_id.to_string();
                TerminalSessionDescriptor {
                    session_id: format!(
                        "model:{thread_id}:{}:{process_id}",
                        command.call_id
                    ),
                    generation: command.call_id.clone(),
                    origin: TerminalSessionOrigin::Model,
                    thread_id: Some(thread_id.clone()),
                    command_item_id: Some(command.call_id.clone()),
                    process_id,
                    title: command.command,
                    cwd: command.cwd.as_path().to_path_buf(),
                    replay_base64: (!command.latest_output_bytes.is_empty()).then(|| {
                        base64::engine::general_purpose::STANDARD
                            .encode(command.latest_output_bytes)
                    }),
                    replay_truncated: command.replay_truncated,
                    replay_through_sequence: command.replay_through_sequence,
                    can_resize: command.can_resize,
                    can_write: true,
                    can_terminate: true,
                }
            }));
        }
        Ok(Some(TerminalSessionListResponse { data }.into()))
    }

    pub(crate) async fn terminal_session_write(
        &self,
        request_id: ConnectionRequestId,
        params: TerminalSessionWriteParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let delta = base64::engine::general_purpose::STANDARD
            .decode(params.delta_base64)
            .map_err(|err| invalid_params(format!("invalid deltaBase64: {err}")))?;
        self.dispatch_terminal_control(
            request_id.connection_id,
            params.target,
            TerminalControl::Write(delta),
        )
        .await?;
        Ok(Some(TerminalSessionWriteResponse {}.into()))
    }

    pub(crate) async fn terminal_session_resize(
        &self,
        request_id: ConnectionRequestId,
        params: TerminalSessionResizeParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let size = crate::command_exec::terminal_size_from_protocol(params.size)?;
        self.dispatch_terminal_control(
            request_id.connection_id,
            params.target,
            TerminalControl::Resize(size),
        )
        .await?;
        Ok(Some(TerminalSessionResizeResponse {}.into()))
    }

    pub(crate) async fn terminal_session_terminate(
        &self,
        request_id: ConnectionRequestId,
        params: TerminalSessionTerminateParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.dispatch_terminal_control(
            request_id.connection_id,
            params.target,
            TerminalControl::Terminate,
        )
        .await?;
        Ok(Some(TerminalSessionTerminateResponse {}.into()))
    }

    pub(crate) async fn connection_closed(&self, connection_id: ConnectionId) {
        self.command_exec_manager
            .connection_closed(connection_id)
            .await;
    }

    async fn exec_one_off_command(
        &self,
        request_id: &ConnectionRequestId,
        params: CommandExecParams,
    ) -> Result<(), JSONRPCErrorError> {
        self.exec_one_off_command_inner(request_id.clone(), params)
            .await
    }

    async fn exec_one_off_command_inner(
        &self,
        request_id: ConnectionRequestId,
        params: CommandExecParams,
    ) -> Result<(), JSONRPCErrorError> {
        tracing::debug!("ExecOneOffCommand params: {params:?}");

        let request = request_id.clone();

        if params.command.is_empty() {
            return Err(invalid_request("command must not be empty"));
        }

        let CommandExecParams {
            command,
            process_id,
            tty,
            stream_stdin,
            stream_stdout_stderr,
            output_bytes_cap,
            disable_output_cap,
            disable_timeout,
            timeout_ms,
            cwd,
            env: env_overrides,
            size,
            sandbox_policy,
            permission_profile,
        } = params;
        if sandbox_policy.is_some() && permission_profile.is_some() {
            return Err(invalid_request(
                "`permissionProfile` cannot be combined with `sandboxPolicy`",
            ));
        }

        if size.is_some() && !tty {
            return Err(invalid_params("command/exec size requires tty: true"));
        }

        if disable_output_cap && output_bytes_cap.is_some() {
            return Err(invalid_params(
                "command/exec cannot set both outputBytesCap and disableOutputCap",
            ));
        }

        if disable_timeout && timeout_ms.is_some() {
            return Err(invalid_params(
                "command/exec cannot set both timeoutMs and disableTimeout",
            ));
        }

        let cwd = cwd.map_or_else(|| self.config.cwd.clone(), |cwd| self.config.cwd.join(cwd));
        let mut env = create_env(
            &self.config.permissions.shell_environment_policy,
            /*thread_id*/ None,
        );
        if let Some(env_overrides) = env_overrides {
            for (key, value) in env_overrides {
                match value {
                    Some(value) => {
                        env.insert(key, value);
                    }
                    None => {
                        env.remove(&key);
                    }
                }
            }
        }
        let timeout_ms = match timeout_ms {
            Some(timeout_ms) => match u64::try_from(timeout_ms) {
                Ok(timeout_ms) => Some(timeout_ms),
                Err(_) => {
                    return Err(invalid_params(format!(
                        "command/exec timeoutMs must be non-negative, got {timeout_ms}"
                    )));
                }
            },
            None => None,
        };
        let managed_network_requirements_enabled =
            self.config.managed_network_requirements_enabled();
        let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
        let started_network_proxy = match self.config.permissions.network.as_ref() {
            Some(spec) => match spec
                .start_proxy(
                    &network_proxy_runtime_factory,
                    self.config.permissions.permission_profile(),
                    /*policy_decider*/ None,
                    /*blocked_request_observer*/ None,
                    managed_network_requirements_enabled,
                    NetworkProxyAuditMetadata::default(),
                )
                .await
            {
                Ok(started) => Some(started),
                Err(err) => {
                    return Err(internal_error(format!(
                        "failed to start managed network proxy: {err}"
                    )));
                }
            },
            None => None,
        };
        let windows_sandbox_level = WindowsSandboxLevel::from_config(&self.config);
        let output_bytes_cap = if disable_output_cap {
            None
        } else {
            Some(output_bytes_cap.unwrap_or(DEFAULT_OUTPUT_BYTES_CAP))
        };
        let expiration = if disable_timeout {
            ExecExpiration::Cancellation(CancellationToken::new())
        } else {
            match timeout_ms {
                Some(timeout_ms) => timeout_ms.into(),
                None => ExecExpiration::DefaultTimeout,
            }
        };
        let capture_policy = if disable_output_cap {
            ExecCapturePolicy::FullBuffer
        } else {
            ExecCapturePolicy::ShellTool
        };
        let sandbox_cwd = if permission_profile.is_some() {
            cwd.clone()
        } else {
            self.config.cwd.clone()
        };
        let exec_params = ExecParams {
            command,
            cwd: cwd.clone(),
            expiration,
            capture_policy,
            env,
            network: started_network_proxy
                .as_ref()
                .map(|started_proxy| started_proxy.proxy()),
            sandbox_permissions: SandboxPermissions::UseDefault,
            windows_sandbox_level,
            windows_sandbox_private_desktop: self
                .config
                .permissions
                .windows_sandbox_private_desktop,
            justification: None,
            arg0: None,
        };

        let effective_permission_profile = if let Some(permission_profile) = permission_profile {
            let permission_profile = protocol::models::PermissionProfile::from(permission_profile);
            let (mut file_system_sandbox_policy, network_sandbox_policy) =
                permission_profile.to_runtime_permissions();
            let configured_file_system_sandbox_policy =
                self.config.permissions.file_system_sandbox_policy();
            Self::preserve_configured_deny_read_restrictions(
                &mut file_system_sandbox_policy,
                &configured_file_system_sandbox_policy,
            );
            let effective_permission_profile =
                protocol::models::PermissionProfile::from_runtime_permissions_with_enforcement(
                    permission_profile.enforcement(),
                    &file_system_sandbox_policy,
                    network_sandbox_policy,
                );
            self.config
                .permissions
                .can_set_permission_profile(&effective_permission_profile)
                .map_err(|err| invalid_request(format!("invalid permission profile: {err}")))?;
            effective_permission_profile
        } else if let Some(policy) = sandbox_policy.map(|policy| policy.to_core()) {
            self.config
                .permissions
                .can_set_legacy_sandbox_policy(&policy, &sandbox_cwd)
                .map_err(|err| invalid_request(format!("invalid sandbox policy: {err}")))?;
            let file_system_sandbox_policy =
                protocol::permissions::FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
                    &policy,
                    &sandbox_cwd,
                );
            let network_sandbox_policy = protocol::permissions::NetworkSandboxPolicy::from(&policy);
            let permission_profile =
                protocol::models::PermissionProfile::from_runtime_permissions_with_enforcement(
                    protocol::models::SandboxEnforcement::from_legacy_sandbox_policy(&policy),
                    &file_system_sandbox_policy,
                    network_sandbox_policy,
                );
            self.config
                .permissions
                .can_set_permission_profile(&permission_profile)
                .map_err(|err| invalid_request(format!("invalid sandbox policy: {err}")))?;
            permission_profile
        } else {
            self.config.permissions.effective_permission_profile()
        };

        let codex_linux_sandbox_exe = self.arg0_paths.codex_linux_sandbox_exe.clone();
        let outgoing = self.outgoing.clone();
        let request_for_task = request.clone();
        let started_network_proxy_for_task = started_network_proxy;
        let use_legacy_landlock = self.config.features.use_legacy_landlock();
        let size = match size.map(crate::command_exec::terminal_size_from_protocol) {
            Some(Ok(size)) => Some(size),
            Some(Err(error)) => return Err(error),
            None => None,
        };

        let exec_request = command_service::build_exec_request(
            exec_params,
            &effective_permission_profile,
            &sandbox_cwd,
            &codex_linux_sandbox_exe,
            use_legacy_landlock,
            self.sandbox_runtime.as_ref(),
        )
        .map_err(|err| internal_error(format!("exec failed: {err}")))?;
        self.command_exec_manager
            .start(StartCommandExecParams {
                outgoing,
                request_id: request_for_task,
                process_id,
                exec_request,
                started_network_proxy: started_network_proxy_for_task,
                tty,
                stream_stdin,
                stream_stdout_stderr,
                output_bytes_cap,
                size,
            })
            .await
    }

    async fn dispatch_terminal_control(
        &self,
        connection_id: ConnectionId,
        target: TerminalSessionRef,
        control: TerminalControl,
    ) -> Result<(), JSONRPCErrorError> {
        match target.origin {
            TerminalSessionOrigin::User => {
                if target.session_id != format!("user:{}", target.process_id) {
                    return Err(invalid_request("stale user terminal session identity"));
                }
                match control {
                    TerminalControl::Write(delta) => {
                        self.command_exec_manager
                            .write_terminal(
                                connection_id,
                                target.process_id,
                                &target.generation,
                                target.resume_token.as_deref(),
                                delta,
                            )
                            .await
                    }
                    TerminalControl::Resize(size) => {
                        self.command_exec_manager
                            .resize_terminal(
                                connection_id,
                                target.process_id,
                                &target.generation,
                                target.resume_token.as_deref(),
                                size,
                            )
                            .await
                    }
                    TerminalControl::Terminate => {
                        self.command_exec_manager
                            .terminate_terminal(
                                connection_id,
                                target.process_id,
                                &target.generation,
                                target.resume_token.as_deref(),
                            )
                            .await
                    }
                }
            }
            TerminalSessionOrigin::Model => {
                let thread_id = target
                    .thread_id
                    .ok_or_else(|| invalid_params("model terminal requires threadId"))?;
                let command_item_id = target
                    .command_item_id
                    .ok_or_else(|| invalid_params("model terminal requires commandItemId"))?;
                if target.generation != command_item_id {
                    return Err(invalid_request("stale model terminal generation"));
                }
                let parsed_thread_id = protocol::ThreadId::from_string(&thread_id)
                    .map_err(|err| invalid_params(format!("invalid threadId: {err}")))?;
                let process_id = target
                    .process_id
                    .parse::<i32>()
                    .map_err(|err| invalid_params(format!("invalid processId: {err}")))?;
                let result = match control {
                    TerminalControl::Write(delta) => {
                        self.thread_service
                            .write_live_terminal(
                                parsed_thread_id,
                                process_id,
                                &command_item_id,
                                delta,
                            )
                            .await
                    }
                    TerminalControl::Resize(size) => {
                        self.thread_service
                            .resize_live_terminal(
                                parsed_thread_id,
                                process_id,
                                &command_item_id,
                                size.rows,
                                size.cols,
                            )
                            .await
                    }
                    TerminalControl::Terminate => {
                        self.thread_service
                            .terminate_live_terminal(
                                parsed_thread_id,
                                process_id,
                                &command_item_id,
                            )
                            .await
                    }
                };
                result.map_err(|err| invalid_request(err.to_string()))
            }
        }
    }

    fn preserve_configured_deny_read_restrictions(
        file_system_sandbox_policy: &mut FileSystemSandboxPolicy,
        configured_file_system_sandbox_policy: &FileSystemSandboxPolicy,
    ) {
        file_system_sandbox_policy
            .preserve_deny_read_restrictions_from(configured_file_system_sandbox_policy);
    }
}

enum TerminalControl {
    Write(Vec<u8>),
    Resize(codex_utils_pty::TerminalSize),
    Terminate,
}

#[cfg(test)]
#[path = "command_exec_processor_tests.rs"]
mod command_exec_processor_tests;
