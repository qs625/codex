fn stored_auth_mode_value(auth: &AuthDotJson) -> codex_auth_types::AuthMode {
    if let Some(mode) = auth.auth_mode {
        return mode;
    }
    if auth.openai_api_key.is_some() {
        codex_auth_types::AuthMode::ApiKey
    } else {
        codex_auth_types::AuthMode::Chatgpt
    }
}

fn stored_auth_issues(
    auth: &AuthDotJson,
    env_var_present: impl Fn(&str) -> bool,
) -> Vec<&'static str> {
    let mut issues = Vec::new();
    match stored_auth_mode_value(auth) {
        codex_auth_types::AuthMode::ApiKey => {
            let stored_key_present = auth
                .openai_api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty());
            let env_key_present =
                env_var_present(OPENAI_API_KEY_ENV_VAR) || env_var_present(CODEX_API_KEY_ENV_VAR);
            if !stored_key_present && !env_key_present {
                issues.push("API key auth is missing an API key");
            }
        }
        codex_auth_types::AuthMode::Chatgpt => {
            match auth.tokens.as_ref() {
                Some(tokens) => {
                    if tokens.access_token.trim().is_empty() {
                        issues.push("ChatGPT auth is missing an access token");
                    }
                    if tokens.refresh_token.trim().is_empty() {
                        issues.push("ChatGPT auth is missing a refresh token");
                    }
                }
                None => issues.push("ChatGPT auth is missing token data"),
            }
            if auth.last_refresh.is_none() {
                issues.push("ChatGPT auth is missing refresh metadata");
            }
        }
        codex_auth_types::AuthMode::ChatgptAuthTokens => {
            match auth.tokens.as_ref() {
                Some(tokens) => {
                    if tokens.access_token.trim().is_empty() {
                        issues.push("external ChatGPT auth is missing an access token");
                    }
                    if tokens.account_id.is_none() && tokens.id_token.chatgpt_account_id.is_none() {
                        issues.push("external ChatGPT auth is missing a ChatGPT account id");
                    }
                }
                None => issues.push("external ChatGPT auth is missing token data"),
            }
            if auth.last_refresh.is_none() {
                issues.push("external ChatGPT auth is missing refresh metadata");
            }
        }
        codex_auth_types::AuthMode::AgentIdentity => {
            if auth
                .agent_identity
                .as_deref()
                .is_none_or(|token| token.trim().is_empty())
            {
                issues.push("agent identity auth is missing an agent identity token");
            }
        }
    }
    issues
}

fn network_check() -> DoctorCheck {
    let mut details = Vec::new();
    push_proxy_env_details(&mut details);

    let mut status = CheckStatus::Ok;
    let mut summary = "network-related environment looks readable".to_string();
    for name in ["CODEX_CA_CERTIFICATE", "SSL_CERT_FILE"] {
        if let Some(raw) = env::var_os(name) {
            let path = PathBuf::from(raw);
            match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    if let Err(err) = read_probe_file(&path) {
                        status = CheckStatus::Warning;
                        summary = "custom CA env var points at an unreadable file".to_string();
                        details.push(format!("{name}: {} ({err})", path.display()));
                    } else {
                        details.push(format!("{name}: readable file {}", path.display()));
                    }
                }
                Ok(_) => {
                    status = CheckStatus::Warning;
                    summary = "custom CA env var does not point at a file".to_string();
                    details.push(format!("{name}: not a file {}", path.display()));
                }
                Err(err) => {
                    status = CheckStatus::Warning;
                    summary = "custom CA env var points at an unreadable path".to_string();
                    details.push(format!("{name}: {} ({err})", path.display()));
                }
            }
        }
    }

    DoctorCheck::new("network.env", "network", status, summary).details(details)
}

fn push_proxy_env_details(details: &mut Vec<String>) {
    let present_proxy_vars = PROXY_ENV_VARS
        .iter()
        .copied()
        .filter(|name| env_var_present(name))
        .collect::<Vec<_>>();
    if present_proxy_vars.is_empty() {
        details.push("proxy env vars: none".to_string());
    } else {
        details.push(format!(
            "proxy env vars present: {}",
            present_proxy_vars.join(", ")
        ));
    }
}

fn read_probe_file(path: &Path) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0_u8; 1];
    let _ = file.read(&mut buffer)?;
    Ok(())
}

async fn mcp_check(config: &Config) -> DoctorCheck {
    mcp_check_from_servers(config.mcp_servers.get()).await
}

async fn mcp_check_from_servers(servers: &HashMap<String, McpServerConfig>) -> DoctorCheck {
    if servers.is_empty() {
        return DoctorCheck::new(
            "mcp.config",
            "mcp",
            CheckStatus::Ok,
            "no MCP servers configured",
        );
    }

    let mut details = Vec::new();
    let mut transport_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut disabled = 0usize;
    let mut missing_env = Vec::new();
    let mut unreachable_required_http = Vec::new();
    let mut unreachable_optional_http = Vec::new();

    for (name, server) in servers {
        let disabled_server = !server.enabled || server.disabled_reason.is_some();
        if disabled_server {
            disabled += 1;
        }
        match &server.transport {
            McpServerTransportConfig::Stdio {
                command,
                env,
                env_vars,
                cwd,
                ..
            } => {
                *transport_counts.entry("stdio").or_default() += 1;
                if disabled_server {
                    continue;
                }
                if let Some(cwd) = cwd
                    && !cwd.exists()
                {
                    missing_env.push(format!("{name}: cwd does not exist ({})", cwd.display()));
                }
                if command.trim().is_empty() {
                    missing_env.push(format!("{name}: stdio command is empty"));
                } else if let Err(err) =
                    stdio_command_resolves(command, cwd.as_deref(), env.as_ref())
                {
                    missing_env.push(format!(
                        "{name}: stdio command {command:?} is not resolvable ({err})"
                    ));
                }
                if let Some(env) = env {
                    for key in env.keys().filter(|key| key.trim().is_empty()) {
                        missing_env.push(format!("{name}: empty env key {key}"));
                    }
                }
                for env_var in env_vars {
                    if env_var.is_remote_source() {
                        missing_env.push(format!(
                            "{name}: env_vars entry `{}` uses source `remote`, which requires remote MCP stdio",
                            env_var.name()
                        ));
                    } else if !env_var_present(env_var.name()) {
                        missing_env.push(format!("{name}: env var {} is not set", env_var.name()));
                    }
                }
            }
            McpServerTransportConfig::StreamableHttp {
                url,
                bearer_token_env_var,
                env_http_headers,
                ..
            } => {
                *transport_counts.entry("streamable_http").or_default() += 1;
                if disabled_server {
                    continue;
                }
                if let Some(env_var) = bearer_token_env_var
                    && !env_var_present(env_var)
                {
                    missing_env.push(format!("{name}: bearer token env var {env_var} is not set"));
                }
                if let Some(headers) = env_http_headers {
                    for env_var in headers.values() {
                        if !env_var_present(env_var) {
                            missing_env
                                .push(format!("{name}: header env var {env_var} is not set"));
                        }
                    }
                }
                if let Err(err) = mcp_http_probe_url(url).await {
                    let detail = format!("{name}: {url} ({err})");
                    if server.required {
                        unreachable_required_http.push(detail);
                    } else {
                        unreachable_optional_http.push(detail);
                    }
                }
            }
        }
    }

    details.push(format!("configured servers: {}", servers.len()));
    details.push(format!("disabled servers: {disabled}"));
    for (transport, count) in transport_counts {
        details.push(format!("{transport} servers: {count}"));
    }
    details.extend(missing_env.iter().cloned());
    details.extend(
        unreachable_required_http
            .iter()
            .map(|detail| format!("required reachability failed: {detail}")),
    );
    details.extend(
        unreachable_optional_http
            .iter()
            .map(|detail| format!("optional reachability failed: {detail}")),
    );

    let required_missing = servers.iter().any(|(name, server)| {
        server.required
            && missing_env
                .iter()
                .any(|missing| missing.starts_with(&format!("{name}:")))
    });
    let status = if required_missing || !unreachable_required_http.is_empty() {
        CheckStatus::Fail
    } else if !missing_env.is_empty() || !unreachable_optional_http.is_empty() {
        CheckStatus::Warning
    } else {
        CheckStatus::Ok
    };
    let summary = match status {
        CheckStatus::Ok => "MCP configuration is locally consistent",
        CheckStatus::Warning => "MCP configuration has optional issues",
        CheckStatus::Fail => "MCP configuration has failing required inputs or reachability",
    };

    let mut check = DoctorCheck::new("mcp.config", "mcp", status, summary).details(details);
    if status != CheckStatus::Ok {
        check = check.remediation("Set the missing MCP env vars or disable the affected server.");
    }
    check
}

fn sandbox_check(config: &Config, arg0_paths: &Arg0DispatchPaths) -> DoctorCheck {
    let mut details = Vec::new();
    details.push(format!(
        "approval policy: {:?}",
        config.permissions.approval_policy.value()
    ));
    let file_system_sandbox = config.permissions.file_system_sandbox_policy();
    details.push(format!("filesystem sandbox: {}", file_system_sandbox.kind));
    details.push(format!(
        "network sandbox: {}",
        config.permissions.network_sandbox_policy()
    ));
    push_path_detail(
        &mut details,
        "codex-linux-sandbox helper",
        arg0_paths.codex_linux_sandbox_exe.as_deref(),
    );
    push_path_detail(
        &mut details,
        "execve wrapper helper",
        arg0_paths.main_execve_wrapper_exe.as_deref(),
    );

    let mut status = CheckStatus::Ok;
    let mut summary = "sandbox configuration is readable".to_string();
    if let Some(helper) = arg0_paths.codex_linux_sandbox_exe.as_deref()
        && !helper.exists()
    {
        status = CheckStatus::Warning;
        summary = "Linux sandbox helper path does not exist".to_string();
    }

    DoctorCheck::new("sandbox.helpers", "sandbox", status, summary).details(details)
}

#[derive(Clone, Debug)]
struct TerminalCheckInputs {
    info: TerminalInfo,
    env: BTreeMap<String, String>,
    present_env: BTreeSet<String>,
    no_color_flag: bool,
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    stderr_is_terminal: bool,
    stream_supports_color: bool,
    terminal_size: Result<(u16, u16), String>,
    tmux_details: Vec<String>,
}

impl TerminalCheckInputs {
    fn detect(no_color_flag: bool) -> Self {
        let names = terminal_env_names();
        let (env, present_env) = collect_env_snapshot(&names);
        let terminal_size = crossterm::terminal::size().map_err(|err| err.to_string());
        let info = terminal_info();
        let tmux_details = if matches!(info.multiplexer, Some(Multiplexer::Tmux { .. })) {
            tmux_diagnostic_details()
        } else {
            Vec::new()
        };
        Self {
            info,
            env,
            present_env,
            no_color_flag,
            stdin_is_terminal: std::io::stdin().is_terminal(),
            stdout_is_terminal: std::io::stdout().is_terminal(),
            stderr_is_terminal: std::io::stderr().is_terminal(),
            stream_supports_color: supports_color::on(Stream::Stdout).is_some(),
            terminal_size,
            tmux_details,
        }
    }

    fn env_value(&self, name: &str) -> Option<&str> {
        self.env.get(name).map(String::as_str)
    }

    fn env_present(&self, name: &str) -> bool {
        self.present_env.contains(name)
    }
}

fn terminal_check(no_color_flag: bool) -> DoctorCheck {
    terminal_check_from_inputs(TerminalCheckInputs::detect(no_color_flag))
}

fn terminal_check_from_inputs(inputs: TerminalCheckInputs) -> DoctorCheck {
    let info = &inputs.info;
    let name = info.name;
    let mut details = vec![format!("terminal: {}", terminal_name(info))];
    if let Some(term_program) = info.term_program.as_deref() {
        details.push(format!("TERM_PROGRAM: {term_program}"));
    }
    if let Some(version) = info.version.as_deref() {
        details.push(format!("terminal version: {version}"));
    }
    if let Some(term) = info.term.as_deref() {
        details.push(format!("TERM: {term}"));
    }
    if let Some(multiplexer) = info.multiplexer.as_ref() {
        details.push(format!("multiplexer: {}", multiplexer_name(multiplexer)));
    }
    details.push(format!("stdin is terminal: {}", inputs.stdin_is_terminal));
    details.push(format!("stdout is terminal: {}", inputs.stdout_is_terminal));
    details.push(format!("stderr is terminal: {}", inputs.stderr_is_terminal));
    match &inputs.terminal_size {
        Ok((columns, rows)) => details.push(format!("terminal size: {columns}x{rows}")),
        Err(err) => details.push(format!("terminal size: unavailable ({err})")),
    }
    push_terminal_env_values(&mut details, &inputs, TERMINAL_DIMENSION_ENV_VARS);
    details.push(format!("color output: {}", color_output_summary(&inputs)));
    push_terminal_env_values(&mut details, &inputs, COLOR_ENV_VARS);
    let terminfo_warning = push_terminfo_details(&mut details, &inputs);
    let locale = effective_locale(&inputs);
    if let Some(locale) = locale.as_ref() {
        details.push(format!("effective locale: {locale}"));
    }
    push_presence_env_values(&mut details, &inputs, REMOTE_TERMINAL_ENV_VARS);
    details.extend(inputs.tmux_details.iter().cloned());

    let locale_warning = locale.as_deref().is_some_and(is_non_utf8_locale);
    let mut issues = Vec::new();
    if matches!(name, TerminalName::Dumb) {
        issues.push(
            DoctorIssue::new(
                CheckStatus::Fail,
                "TERM=dumb - colors and cursor control are disabled",
            )
            .measured("TERM=dumb")
            .expected("TERM=xterm-256color or another real terminal type")
            .remedy("set TERM to a real value, for example xterm-256color")
            .field("TERM"),
        );
    }
    if locale_warning {
        let measured = locale.unwrap_or_else(|| "unknown".to_string());
        issues.push(
            DoctorIssue::new(
                CheckStatus::Warning,
                "locale is not UTF-8 - unicode glyphs may render incorrectly",
            )
            .measured(measured)
            .expected("UTF-8 locale, for example en_US.UTF-8")
            .remedy("export LANG=en_US.UTF-8 or another UTF-8 locale")
            .field("effective locale"),
        );
    }
    if terminfo_warning {
        issues.push(
            DoctorIssue::new(
                CheckStatus::Fail,
                "TERMINFO unreadable - terminal capabilities are unknown",
            )
            .expected("readable terminfo file or directory")
            .remedy("check that $TERMINFO points to a readable directory")
            .field("TERMINFO")
            .field("TERMINFO_DIRS entry"),
        );
    }
    issues.extend(terminal_size_issues(&inputs));

    let status = issues
        .iter()
        .map(|issue| issue.severity)
        .max()
        .unwrap_or(CheckStatus::Ok);
    let summary = issues
        .first()
        .map(|issue| issue.cause.as_str())
        .unwrap_or("terminal metadata was detected");
    let mut check = DoctorCheck::new("terminal.env", "terminal", status, summary).details(details);
    for issue in issues {
        check = check.issue(issue);
    }
    check
}

fn terminal_name(info: &TerminalInfo) -> &'static str {
    match info.name {
        TerminalName::AppleTerminal => "Apple Terminal",
        TerminalName::Ghostty => "Ghostty",
        TerminalName::Iterm2 => "iTerm2",
        TerminalName::WarpTerminal => "Warp",
        TerminalName::VsCode => "VS Code",
        TerminalName::WezTerm => "WezTerm",
        TerminalName::Kitty => "kitty",
        TerminalName::Alacritty => "Alacritty",
        TerminalName::Konsole => "Konsole",
        TerminalName::GnomeTerminal => "GNOME Terminal",
        TerminalName::Vte => "VTE",
        TerminalName::WindowsTerminal => "Windows Terminal",
        TerminalName::Dumb => "dumb",
        TerminalName::Unknown => "unknown",
    }
}

fn multiplexer_name(multiplexer: &Multiplexer) -> String {
    match multiplexer {
        Multiplexer::Tmux { version } => match version {
            Some(version) => format!("tmux {version}"),
            None => "tmux".to_string(),
        },
        Multiplexer::Zellij { version } => match version {
            Some(version) => format!("zellij {version}"),
            None => "zellij".to_string(),
        },
    }
}

fn terminal_env_names() -> BTreeSet<&'static str> {
    let mut names = BTreeSet::from(["TERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"]);
    names.extend(COLOR_ENV_VARS.iter().copied());
    names.extend(TERMINAL_DIMENSION_ENV_VARS.iter().copied());
    names.extend(TERMINFO_ENV_VARS.iter().copied());
    names.extend(LOCALE_ENV_VARS.iter().copied());
    names.extend(REMOTE_TERMINAL_ENV_VARS.iter().copied());
    names
}

fn collect_env_snapshot(
    names: &BTreeSet<&'static str>,
) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut values = BTreeMap::new();
    let mut present = BTreeSet::new();
    for name in names {
        if let Some(raw) = env::var_os(name) {
            present.insert((*name).to_string());
            let value = raw.to_string_lossy().trim().to_string();
            if !value.is_empty() {
                values.insert((*name).to_string(), value);
            }
        }
    }
    (values, present)
}

fn push_terminal_env_values(
    details: &mut Vec<String>,
    inputs: &TerminalCheckInputs,
    names: &[&str],
) {
    for name in names {
        if let Some(value) = inputs.env_value(name) {
            details.push(format!("{name}: {value}"));
        } else if inputs.env_present(name) {
            details.push(format!("{name}: present"));
        }
    }
}

fn push_presence_env_values(
    details: &mut Vec<String>,
    inputs: &TerminalCheckInputs,
    names: &[&str],
) {
    for name in names {
        if inputs.env_present(name) {
            details.push(format!("{name}: present"));
        }
    }
}

fn color_output_summary(inputs: &TerminalCheckInputs) -> String {
    if should_enable_color(
        inputs.no_color_flag,
        inputs.env_present("NO_COLOR"),
        inputs.env_value("TERM"),
        inputs.stdout_is_terminal,
        inputs.stream_supports_color,
    ) {
        return "enabled".to_string();
    }

    let reason = if inputs.no_color_flag {
        "--no-color"
    } else if inputs.env_present("NO_COLOR") {
        "NO_COLOR"
    } else if inputs.env_value("TERM") == Some("dumb") {
        "TERM=dumb"
    } else if !inputs.stdout_is_terminal {
        "stdout is not a terminal"
    } else if !inputs.stream_supports_color {
        "terminal color support not detected"
    } else {
        "disabled"
    };
    format!("disabled ({reason})")
}

fn push_terminfo_details(details: &mut Vec<String>, inputs: &TerminalCheckInputs) -> bool {
    let mut has_warning = false;
    if let Some(raw) = inputs.env_value("TERMINFO") {
        let path = PathBuf::from(raw);
        let (status, warning) = terminal_path_readiness(&path);
        details.push(format!("TERMINFO: {} ({status})", path.display()));
        has_warning |= warning;
    }
    if let Some(raw) = inputs.env_value("TERMINFO_DIRS") {
        for path in env::split_paths(raw).filter(|path| !path.as_os_str().is_empty()) {
            let (status, warning) = terminal_path_readiness(&path);
            details.push(format!(
                "TERMINFO_DIRS entry: {} ({status})",
                path.display()
            ));
            has_warning |= warning;
        }
    } else if inputs.env_present("TERMINFO_DIRS") {
        details.push("TERMINFO_DIRS: present".to_string());
    }
    has_warning
}

fn terminal_path_readiness(path: &Path) -> (String, bool) {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => match std::fs::read_dir(path) {
            Ok(_) => ("dir".to_string(), false),
            Err(err) => (format!("dir unreadable: {err}"), true),
        },
        Ok(metadata) if metadata.is_file() => match read_probe_file(path) {
            Ok(_) => ("file".to_string(), false),
            Err(err) => (format!("file unreadable: {err}"), true),
        },
        Ok(_) => ("not a file or directory".to_string(), true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => ("missing".to_string(), true),
        Err(err) => (err.to_string(), true),
    }
}
