fn effective_locale(inputs: &TerminalCheckInputs) -> Option<String> {
    LOCALE_ENV_VARS
        .iter()
        .find_map(|name| inputs.env_value(name).map(ToString::to_string))
}

fn is_non_utf8_locale(locale: &str) -> bool {
    let locale = locale.to_ascii_lowercase();
    !(locale.contains("utf-8") || locale.contains("utf8"))
}

fn terminal_size_issues(inputs: &TerminalCheckInputs) -> Vec<DoctorIssue> {
    let mut issues = Vec::new();
    if let Ok((columns, rows)) = inputs.terminal_size {
        if columns > 0 && columns < NARROW_TERMINAL_COLUMNS {
            issues.push(
                DoctorIssue::new(
                    CheckStatus::Warning,
                    format!("width {columns} cols - output may wrap (recommended >=80)"),
                )
                .measured(format!("{columns} x {rows}"))
                .expected(format!(">= {NARROW_TERMINAL_COLUMNS} columns"))
                .remedy("resize the window to at least 80 columns")
                .field("terminal size"),
            );
        }
        if rows > 0 && rows < NARROW_TERMINAL_ROWS {
            issues.push(
                DoctorIssue::new(
                    CheckStatus::Warning,
                    format!("height {rows} rows - content may scroll off (recommended >=24)"),
                )
                .measured(format!("{columns} x {rows}"))
                .expected(format!(">= {NARROW_TERMINAL_ROWS} rows"))
                .remedy("resize the window to at least 24 rows")
                .field("terminal size"),
            );
        }
    }

    if let Some(columns) = inputs
        .env_value("COLUMNS")
        .and_then(|columns| columns.parse::<u16>().ok())
        && columns > 0
        && columns < NARROW_TERMINAL_COLUMNS
    {
        issues.push(
            DoctorIssue::new(
                CheckStatus::Warning,
                format!("COLUMNS={columns} - output may wrap (recommended >=80)"),
            )
            .measured(format!("{columns} columns"))
            .expected(format!(">= {NARROW_TERMINAL_COLUMNS} columns"))
            .remedy("resize the window to at least 80 columns")
            .field("COLUMNS"),
        );
    }
    if let Some(rows) = inputs
        .env_value("LINES")
        .and_then(|rows| rows.parse::<u16>().ok())
        && rows > 0
        && rows < NARROW_TERMINAL_ROWS
    {
        issues.push(
            DoctorIssue::new(
                CheckStatus::Warning,
                format!("LINES={rows} - content may scroll off (recommended >=24)"),
            )
            .measured(format!("{rows} rows"))
            .expected(format!(">= {NARROW_TERMINAL_ROWS} rows"))
            .remedy("resize the window to at least 24 rows")
            .field("LINES"),
        );
    }

    issues
}

fn tmux_diagnostic_details() -> Vec<String> {
    let mut details = Vec::new();
    push_tmux_display_detail(&mut details, "tmux client termtype", "#{client_termtype}");
    push_tmux_display_detail(&mut details, "tmux client termname", "#{client_termname}");
    for option in TMUX_OPTION_NAMES {
        let value = tmux_option_value(option).unwrap_or_else(|| "unavailable".to_string());
        details.push(format!("tmux {option}: {value}"));
    }
    details
}

fn push_tmux_display_detail(details: &mut Vec<String>, label: &str, format: &str) {
    if let Some(value) = tmux_display_message(format) {
        details.push(format!("{label}: {value}"));
    }
}

fn tmux_option_value(option: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["show-options", "-gqv", option])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    non_empty_trimmed(String::from_utf8(output.stdout).ok()?)
}

fn tmux_display_message(format: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", format])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    non_empty_trimmed(String::from_utf8(output.stdout).ok()?)
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

async fn state_check(config: &Config) -> DoctorCheck {
    let mut details = Vec::new();
    path_readiness(&mut details, "CODEX_HOME", &config.codex_home);
    path_readiness(&mut details, "log dir", &config.log_dir);
    path_readiness(&mut details, "sqlite home", &config.sqlite_home);
    let state_db = state::state_db_path(&config.sqlite_home);
    let log_db = state::logs_db_path(&config.sqlite_home);
    path_readiness(&mut details, "state DB", &state_db);
    path_readiness(&mut details, "log DB", &log_db);
    let mut integrity_failures = Vec::new();
    sqlite_integrity_detail(&mut details, &mut integrity_failures, "state DB", &state_db).await;
    sqlite_integrity_detail(&mut details, &mut integrity_failures, "log DB", &log_db).await;
    rollout_stats_details(&mut details, &config.codex_home);
    standalone_release_cache_details(&mut details);

    let status = if integrity_failures.is_empty() {
        CheckStatus::Ok
    } else {
        CheckStatus::Fail
    };
    let summary = if status == CheckStatus::Ok {
        "state paths and databases are inspectable"
    } else {
        "state database integrity check failed"
    };
    let mut check = DoctorCheck::new("state.paths", "state", status, summary).details(details);
    if status == CheckStatus::Fail {
        check = check
            .remediation("Back up CODEX_HOME, then remove or repair the affected SQLite database.");
    }
    check
}

async fn sqlite_integrity_detail(
    details: &mut Vec<String>,
    integrity_failures: &mut Vec<String>,
    label: &str,
    path: &Path,
) {
    if !path.is_file() {
        details.push(format!("{label} integrity: skipped (missing)"));
        return;
    }

    match state::sqlite_integrity_check(path).await {
        Ok(rows) if rows.iter().all(|row| row == "ok") => {
            details.push(format!("{label} integrity: ok"));
        }
        Ok(rows) => {
            let message = format!("{label} integrity: {}", rows.join("; "));
            integrity_failures.push(message.clone());
            details.push(message);
        }
        Err(err) => {
            let message = format!("{label} integrity: {err}");
            integrity_failures.push(message.clone());
            details.push(message);
        }
    }
}

fn rollout_stats_details(details: &mut Vec<String>, codex_home: &Path) {
    let active = collect_rollout_stats(&codex_home.join("sessions"));
    let archived = collect_rollout_stats(&codex_home.join("archived_sessions"));
    push_rollout_stats_detail(details, "active rollout files", active);
    push_rollout_stats_detail(details, "archived rollout files", archived);
}

fn push_rollout_stats_detail(details: &mut Vec<String>, label: &str, stats: RolloutStats) {
    match stats.error {
        Some(error) => details.push(format!("{label}: scan failed ({error})")),
        None => details.push(format!(
            "{label}: {} files, {} total bytes, {} average bytes",
            stats.files,
            stats.total_bytes,
            stats.average_bytes()
        )),
    }
}

#[derive(Default)]
struct RolloutStats {
    files: u64,
    total_bytes: u64,
    error: Option<String>,
}

impl RolloutStats {
    fn average_bytes(&self) -> u64 {
        if self.files == 0 {
            0
        } else {
            self.total_bytes / self.files
        }
    }
}

fn collect_rollout_stats(root: &Path) -> RolloutStats {
    let mut stats = RolloutStats::default();
    collect_rollout_stats_inner(root, &mut stats);
    stats
}

fn collect_rollout_stats_inner(path: &Path, stats: &mut RolloutStats) {
    if stats.error.is_some() {
        return;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            stats.error = Some(err.to_string());
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                stats.error = Some(err.to_string());
                return;
            }
        };
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                stats.error = Some(err.to_string());
                return;
            }
        };
        if metadata.is_dir() {
            collect_rollout_stats_inner(&path, stats);
        } else if metadata.is_file() && is_rollout_file(&path) {
            stats.files += 1;
            stats.total_bytes = stats.total_bytes.saturating_add(metadata.len());
        }
    }
}

fn is_rollout_file(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("jsonl"))
        && path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("rollout-"))
}

async fn websocket_reachability_check(
    config: &Config,
    auth_manager: Option<Arc<AuthManager>>,
) -> DoctorCheck {
    let provider = &config.model_provider;
    let mut details = vec![
        format!("model provider: {}", config.model_provider_id),
        format!("provider name: {}", provider.name),
        format!("wire API: {}", provider.wire_api),
        format!("supports websockets: {}", provider.supports_websockets),
    ];
    push_proxy_env_details(&mut details);

    if !provider.supports_websockets {
        return DoctorCheck::new(
            "network.websocket_reachability",
            "websocket",
            CheckStatus::Ok,
            "Responses WebSocket is not enabled for the active provider",
        )
        .details(details);
    }

    details.push(format!(
        "connect timeout: {} ms",
        provider.websocket_connect_timeout().as_millis()
    ));

    let runtime_provider = create_model_provider(provider.clone(), auth_manager);
    let auth = runtime_provider.auth().await;
    details.push(format!(
        "auth mode: {}",
        auth.as_ref()
            .map(codex_auth_types::RequestAuthSnapshot::auth_mode)
            .map(auth_mode_name)
            .unwrap_or("none")
    ));

    let api_provider = match runtime_provider.api_provider().await {
        Ok(api_provider) => api_provider,
        Err(err) => {
            return websocket_probe_warning(
                "Responses WebSocket provider setup failed",
                details,
                format!("provider setup failed: {err}"),
            );
        }
    };
    match websocket_url_for_endpoint(api_provider.url_for_path("responses")) {
        Ok(url) => {
            details.push(format!("endpoint: {url}"));
            if let Some(host) = url.host_str()
                && let Some(port) = url.port_or_known_default()
            {
                details.extend(dns_address_family_details(host, port).await);
            }
        }
        Err(err) => {
            return websocket_probe_warning(
                "Responses WebSocket endpoint could not be built",
                details,
                format!("endpoint build failed: {err}"),
            );
        }
    }

    let api_auth = match runtime_provider.api_auth().await {
        Ok(api_auth) => api_auth,
        Err(err) => {
            return websocket_probe_warning(
                "Responses WebSocket auth could not be resolved",
                details,
                format!("auth resolution failed: {err}"),
            );
        }
    };

    let mut extra_headers = HeaderMap::new();
    extra_headers.insert(
        OPENAI_BETA_HEADER,
        HeaderValue::from_static(RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE),
    );
    let client = ResponsesWebsocketClient::new(api_provider, api_auth);
    match tokio::time::timeout(
        provider.websocket_connect_timeout(),
        client.probe_handshake(
            extra_headers,
            default_headers(),
            WEBSOCKET_IMMEDIATE_CLOSE_GRACE,
        ),
    )
    .await
    {
        Ok(Ok(probe)) => {
            details.push(format!("handshake result: HTTP {}", probe.status));
            details.push(format!("reasoning header: {}", probe.reasoning_included));
            details.push(format!(
                "models etag present: {}",
                probe.models_etag_present
            ));
            details.push(format!(
                "server model present: {}",
                probe.server_model_present
            ));
            if let Some(close) = probe.immediate_close {
                details.push(format!("immediate close code: {}", close.code));
                details.push(format!("immediate close reason: {}", close.reason));
                return DoctorCheck::new(
                    "network.websocket_reachability",
                    "websocket",
                    CheckStatus::Warning,
                    "Responses WebSocket closed immediately after handshake",
                )
                .details(details)
                .remediation(
                    "Check proxy, VPN, firewall, DNS, custom CA, and WebSocket policy support.",
                );
            }
            DoctorCheck::new(
                "network.websocket_reachability",
                "websocket",
                CheckStatus::Ok,
                "Responses WebSocket handshake succeeded",
            )
            .details(details)
        }
        Ok(Err(err)) => websocket_probe_warning(
            "Responses WebSocket failed; HTTPS fallback may still work",
            details,
            websocket_error_detail(&err),
        ),
        Err(_) => websocket_probe_warning(
            "Responses WebSocket timed out; HTTPS fallback may still work",
            details,
            "handshake timed out".to_string(),
        ),
    }
}

fn websocket_probe_warning(
    summary: &'static str,
    mut details: Vec<String>,
    error_detail: String,
) -> DoctorCheck {
    details.push(error_detail);
    DoctorCheck::new(
        "network.websocket_reachability",
        "websocket",
        CheckStatus::Warning,
        summary,
    )
    .details(details)
    .remediation("Check proxy, VPN, firewall, DNS, custom CA, and WebSocket policy support.")
}

fn websocket_error_detail(err: &ApiError) -> String {
    match err {
        ApiError::Transport(transport) => format!("handshake transport error: {transport}"),
        ApiError::Api { status, message } => {
            format!("handshake API error: {status} {message}")
        }
        ApiError::Stream(message) => format!("handshake stream error: {message}"),
        ApiError::ContextWindowExceeded
        | ApiError::QuotaExceeded
        | ApiError::UsageNotIncluded
        | ApiError::Retryable { .. }
        | ApiError::RateLimit(_)
        | ApiError::InvalidRequest { .. }
        | ApiError::CyberPolicy { .. }
        | ApiError::ServerOverloaded => format!("handshake error: {err}"),
    }
}

fn auth_mode_name(auth_mode: codex_auth_types::AuthMode) -> &'static str {
    match auth_mode {
        codex_auth_types::AuthMode::ApiKey => "api_key",
        codex_auth_types::AuthMode::Chatgpt => "chatgpt",
        codex_auth_types::AuthMode::ChatgptAuthTokens => "chatgpt_auth_tokens",
        codex_auth_types::AuthMode::AgentIdentity => "agent_identity",
    }
}

async fn dns_address_family_details(host: &str, port: u16) -> Vec<String> {
    match tokio::net::lookup_host((host, port)).await {
        Ok(addresses) => {
            let addresses = addresses.collect::<Vec<_>>();
            let ipv4_count = addresses
                .iter()
                .filter(|address| matches!(address.ip(), IpAddr::V4(_)))
                .count();
            let ipv6_count = addresses
                .iter()
                .filter(|address| matches!(address.ip(), IpAddr::V6(_)))
                .count();
            let first_family = addresses
                .first()
                .map(|address| match address.ip() {
                    IpAddr::V4(_) => "IPv4",
                    IpAddr::V6(_) => "IPv6",
                })
                .unwrap_or("none");
            vec![format!(
                "DNS: {ipv4_count} IPv4, {ipv6_count} IPv6, first {first_family}"
            )]
        }
        Err(err) => vec![format!("DNS: lookup failed ({err})")],
    }
}

fn fallback_state_check() -> DoctorCheck {
    let codex_home = find_codex_home();
    match codex_home {
        Ok(path) => DoctorCheck::new(
            "state.paths",
            "state",
            CheckStatus::Ok,
            "CODEX_HOME was resolved without config",
        )
        .detail(format!("CODEX_HOME: {}", path.display())),
        Err(err) => DoctorCheck::new(
            "state.paths",
            "state",
            CheckStatus::Warning,
            "CODEX_HOME could not be resolved",
        )
        .detail(err.to_string()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReachabilityPlan {
    description: String,
    endpoints: Vec<ReachabilityEndpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReachabilityEndpoint {
    label: String,
    url: String,
    required: bool,
    route_probe_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProviderAuthReachabilityMode {
    NotRequired,
    ApiKey,
    Chatgpt,
}

impl ProviderAuthReachabilityMode {
    fn description(self) -> &'static str {
        match self {
            Self::NotRequired => "provider auth",
            Self::ApiKey => "API key auth",
            Self::Chatgpt => "ChatGPT auth",
        }
    }
}

fn provider_reachability_plan(config: &Config) -> ReachabilityPlan {
    let stored_auth =
        load_auth_dot_json(&config.codex_home, config.cli_auth_credentials_store_mode)
            .ok()
            .flatten();
    let mode = provider_auth_reachability_mode_from_auth(
        config.model_provider.requires_openai_auth,
        env_var_present,
        stored_auth.as_ref(),
    );
    provider_reachability_plan_from_parts(
        mode,
        &config.model_provider_id,
        &config.model_provider.name,
        config.model_provider.base_url.as_deref(),
        config.model_provider.query_params.as_ref(),
        config.model_provider.is_amazon_bedrock(),
        &config.chatgpt_base_url,
    )
}

fn default_reachability_plan() -> ReachabilityPlan {
    provider_reachability_plan_from_parts(
        ProviderAuthReachabilityMode::Chatgpt,
        "openai",
        "OpenAI",
        /*provider_base_url*/ None,
        /*provider_query_params*/ None,
        /*is_amazon_bedrock*/ false,
        "https://chatgpt.com/backend-api/",
    )
}

fn provider_auth_reachability_mode_from_auth(
    requires_openai_auth: bool,
    env_var_present: impl Fn(&str) -> bool,
    stored_auth: Option<&AuthDotJson>,
) -> ProviderAuthReachabilityMode {
    if !requires_openai_auth {
        return ProviderAuthReachabilityMode::NotRequired;
    }
    if env_var_present(OPENAI_API_KEY_ENV_VAR) || env_var_present(CODEX_API_KEY_ENV_VAR) {
        return ProviderAuthReachabilityMode::ApiKey;
    }
    if env_var_present(CODEX_ACCESS_TOKEN_ENV_VAR) {
        return ProviderAuthReachabilityMode::Chatgpt;
    }
    match stored_auth.map(stored_auth_mode_value) {
        Some(codex_auth_types::AuthMode::ApiKey) => ProviderAuthReachabilityMode::ApiKey,
        Some(
            codex_auth_types::AuthMode::Chatgpt
            | codex_auth_types::AuthMode::ChatgptAuthTokens
            | codex_auth_types::AuthMode::AgentIdentity,
        )
        | None => ProviderAuthReachabilityMode::Chatgpt,
    }
}
