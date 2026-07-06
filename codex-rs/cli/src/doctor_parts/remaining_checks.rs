fn provider_reachability_plan_from_parts(
    mode: ProviderAuthReachabilityMode,
    provider_id: &str,
    provider_name: &str,
    provider_base_url: Option<&str>,
    provider_query_params: Option<&HashMap<String, String>>,
    is_amazon_bedrock: bool,
    chatgpt_base_url: &str,
) -> ReachabilityPlan {
    let provider_route_probe_url = provider_base_url
        .or_else(|| {
            (mode == ProviderAuthReachabilityMode::ApiKey).then_some("https://api.openai.com/v1")
        })
        .and_then(|url| {
            should_probe_models_route(provider_name, url, is_amazon_bedrock)
                .then(|| provider_url_for_path(url, "models", provider_query_params))
        });
    let endpoints = match mode {
        ProviderAuthReachabilityMode::ApiKey => vec![ReachabilityEndpoint {
            label: format!("{provider_id} API"),
            url: provider_base_url
                .unwrap_or("https://api.openai.com/v1")
                .to_string(),
            required: true,
            route_probe_url: provider_route_probe_url,
        }],
        ProviderAuthReachabilityMode::Chatgpt => vec![ReachabilityEndpoint {
            label: "ChatGPT".to_string(),
            url: chatgpt_base_url.to_string(),
            required: true,
            route_probe_url: None,
        }],
        ProviderAuthReachabilityMode::NotRequired => provider_base_url
            .map(|url| {
                vec![ReachabilityEndpoint {
                    label: format!("{provider_id} API"),
                    url: url.to_string(),
                    required: true,
                    route_probe_url: provider_route_probe_url,
                }]
            })
            .unwrap_or_default(),
    };
    ReachabilityPlan {
        description: mode.description().to_string(),
        endpoints,
    }
}

fn should_probe_models_route(provider_name: &str, base_url: &str, is_amazon_bedrock: bool) -> bool {
    !is_amazon_bedrock && !is_azure_responses_provider(provider_name, Some(base_url))
}

fn provider_url_for_path(
    base_url: &str,
    path: &str,
    query_params: Option<&HashMap<String, String>>,
) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    let mut url = if path.is_empty() {
        base.to_string()
    } else {
        format!("{base}/{path}")
    };

    if let Some(params) = query_params
        && !params.is_empty()
    {
        let separator = if url.contains('?') { '&' } else { '?' };
        url.push(separator);
        url.push_str(
            &params
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&"),
        );
    }

    url
}

fn websocket_url_for_endpoint(endpoint_url: String) -> Result<url::Url, url::ParseError> {
    let mut url = url::Url::parse(&endpoint_url)?;

    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" | "wss" => return Ok(url),
        _ => return Ok(url),
    };
    let _ = url.set_scheme(scheme);
    Ok(url)
}

async fn provider_reachability_check(plan: ReachabilityPlan) -> DoctorCheck {
    let mut details = vec![format!("reachability mode: {}", plan.description)];
    if plan.endpoints.is_empty() {
        details.push("active provider endpoint: none configured".to_string());
        return DoctorCheck::new(
            "network.provider_reachability",
            "reachability",
            CheckStatus::Ok,
            "active provider has no HTTP endpoint to probe",
        )
        .details(details);
    }

    let mut failures = Vec::new();
    let mut optional_failures = Vec::new();
    let mut route_failures = Vec::new();
    let mut route_warnings = Vec::new();
    let mut issues = Vec::new();
    for endpoint in plan.endpoints {
        match http_probe_url(&endpoint.url).await {
            Ok(status) => details.push(format!(
                "{} base URL: {} reachable ({status})",
                endpoint.label, endpoint.url
            )),
            Err(err) => {
                let requirement = if endpoint.required {
                    "required"
                } else {
                    "optional"
                };
                details.push(format!(
                    "{} base URL: {} {err} ({requirement})",
                    endpoint.label, endpoint.url
                ));
                if endpoint.required {
                    failures.push(endpoint.url);
                } else {
                    optional_failures.push(endpoint.url);
                }
                continue;
            }
        }

        let Some(route_probe_url) = endpoint.route_probe_url.as_deref() else {
            continue;
        };
        match provider_route_probe_url(route_probe_url).await {
            RouteProbeOutcome::Ok(status) => {
                details.push(format!(
                    "{} route probe: {route_probe_url} route exists ({status})",
                    endpoint.label,
                ));
            }
            RouteProbeOutcome::Warning(status) => {
                details.push(format!(
                    "{} route probe: {route_probe_url} returned {status} (warning)",
                    endpoint.label,
                ));
                route_warnings.push(route_probe_url.to_string());
            }
            RouteProbeOutcome::Fail(status) => {
                details.push(format!(
                    "{} route probe: {route_probe_url} returned {status} (required)",
                    endpoint.label,
                ));
                route_failures.push(route_probe_url.to_string());
                issues.push(
                    DoctorIssue::new(
                        CheckStatus::Fail,
                        "provider base URL route returned 404 - verify the configured API prefix",
                    )
                    .measured(format!("{route_probe_url} returned {status}"))
                    .expected("GET /models returns 2xx, 401, or 403")
                    .remedy("Set base_url to the provider API root, for example https://api.openai.com/v1")
                    .field("route probe"),
                );
            }
            RouteProbeOutcome::TransportError(err) => {
                details.push(format!(
                    "{} route probe: {route_probe_url} {err} (required)",
                    endpoint.label,
                ));
                route_failures.push(route_probe_url.to_string());
                issues.push(
                    DoctorIssue::new(
                        CheckStatus::Fail,
                        "provider route probe could not connect - verify network access to the provider API",
                    )
                    .measured(format!("{route_probe_url} {err}"))
                    .expected("GET /models completes")
                    .remedy("Check proxy, VPN, firewall, DNS, and custom CA configuration.")
                    .field("route probe"),
                );
            }
        }
    }

    let (status, summary) = provider_reachability_outcome(
        failures.len() + route_failures.len(),
        optional_failures.len() + route_warnings.len(),
    );
    let mut check = DoctorCheck::new(
        "network.provider_reachability",
        "reachability",
        status,
        summary,
    )
    .details(details);
    for issue in issues {
        check = check.issue(issue);
    }
    if status != CheckStatus::Ok {
        check = check.remediation("Check proxy, VPN, firewall, DNS, and custom CA configuration.");
    }
    check
}

enum RouteProbeOutcome {
    Ok(String),
    Warning(String),
    Fail(String),
    TransportError(String),
}

async fn provider_route_probe_url(url: &str) -> RouteProbeOutcome {
    match http_get_probe_status_with_timeout(url, Duration::from_secs(3)).await {
        Ok(status) if (200..300).contains(&status) || matches!(status, 401 | 403) => {
            RouteProbeOutcome::Ok(format!("HTTP {status}"))
        }
        Ok(404) => RouteProbeOutcome::Fail("HTTP 404".to_string()),
        Ok(status) => RouteProbeOutcome::Warning(format!("HTTP {status}")),
        Err(err) => RouteProbeOutcome::TransportError(err),
    }
}

fn provider_reachability_outcome(
    required_failures: usize,
    warnings: usize,
) -> (CheckStatus, &'static str) {
    match (required_failures, warnings) {
        (0, 0) => (
            CheckStatus::Ok,
            "active provider endpoints are reachable over HTTP",
        ),
        (0, _) => (
            CheckStatus::Warning,
            "provider endpoint checks returned warnings",
        ),
        (_, _) => (
            CheckStatus::Fail,
            "one or more required provider endpoints are unreachable over HTTP",
        ),
    }
}

async fn http_probe_url(url: &str) -> Result<String, String> {
    http_probe_url_with_timeout(url, Duration::from_secs(3)).await
}

async fn mcp_http_probe_url(url: &str) -> Result<String, String> {
    mcp_http_probe_url_with_timeout(url, Duration::from_secs(3)).await
}

async fn mcp_http_probe_url_with_timeout(url: &str, timeout: Duration) -> Result<String, String> {
    match http_probe_url_with_timeout(url, timeout).await {
        Ok(status) => Ok(status),
        Err(head_err) => match http_get_probe_url_with_timeout(url, timeout).await {
            Ok(status) => Ok(status),
            Err(get_err) => Err(format!("HEAD {head_err}; GET {get_err}")),
        },
    }
}

async fn http_probe_url_with_timeout(url: &str, timeout: Duration) -> Result<String, String> {
    let response = build_reqwest_client()
        .head(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                "request timed out".to_string()
            } else if err.is_connect() {
                "connect failed".to_string()
            } else if err.is_builder() {
                "request could not be built".to_string()
            } else {
                err.to_string()
            }
        })?;
    Ok(format!("HTTP {}", response.status().as_u16()))
}

async fn http_get_probe_url_with_timeout(url: &str, timeout: Duration) -> Result<String, String> {
    http_get_probe_status_with_timeout(url, timeout)
        .await
        .map(|status| format!("HTTP {status}"))
}

async fn http_get_probe_status_with_timeout(url: &str, timeout: Duration) -> Result<u16, String> {
    let response = build_reqwest_client()
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                "request timed out".to_string()
            } else if err.is_connect() {
                "connect failed".to_string()
            } else if err.is_builder() {
                "request could not be built".to_string()
            } else {
                err.to_string()
            }
        })?;
    Ok(response.status().as_u16())
}

fn stdio_command_resolves(
    command: &str,
    cwd: Option<&Path>,
    server_env: Option<&HashMap<String, String>>,
) -> Result<(), String> {
    let command_path = Path::new(command);
    if command_path.is_absolute() {
        return executable_path_exists(command_path);
    }

    if command_path.components().count() > 1 {
        let base = cwd
            .map(Path::to_path_buf)
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        return executable_path_exists(&base.join(command_path));
    }

    let Some(path_env) = server_env
        .and_then(|env| env.get("PATH").map(String::as_str))
        .map(std::ffi::OsString::from)
        .or_else(|| env::var_os("PATH"))
    else {
        return Err("PATH is not set".to_string());
    };

    for dir in env::split_paths(&path_env) {
        let candidate = dir.join(command);
        if executable_path_exists(&candidate).is_ok() {
            return Ok(());
        }
        #[cfg(windows)]
        {
            let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
            for extension in pathext.split(';').filter(|extension| !extension.is_empty()) {
                let candidate = dir.join(format!("{command}{extension}"));
                if executable_path_exists(&candidate).is_ok() {
                    return Ok(());
                }
            }
        }
    }
    Err("not found on PATH".to_string())
}

fn executable_path_exists(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => executable_file_permission(path, &metadata),
        Ok(_) => Err("path is not a file".to_string()),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(unix)]
fn executable_file_permission(path: &Path, metadata: &std::fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o111 == 0 {
        Err(format!("{} is not executable", path.display()))
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn executable_file_permission(_path: &Path, _metadata: &std::fs::Metadata) -> Result<(), String> {
    Ok(())
}

fn path_readiness(details: &mut Vec<String>, label: &str, path: &Path) {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            let kind = if metadata.is_dir() {
                "dir"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            details.push(format!("{label}: {} ({kind})", path.display()));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            details.push(format!("{label}: {} (missing)", path.display()));
        }
        Err(err) => details.push(format!("{label}: {} ({err})", path.display())),
    }
}

fn standalone_release_cache_details(details: &mut Vec<String>) {
    let InstallContext::Standalone { release_dir, .. } = InstallContext::current() else {
        return;
    };
    let Some(releases_dir) = release_dir.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(releases_dir) else {
        return;
    };
    let release_count = entries.filter_map(Result::ok).count();
    details.push(format!(
        "standalone release cache: {release_count} entries in {}",
        releases_dir.display()
    ));
}

fn push_path_detail(details: &mut Vec<String>, label: &str, path: Option<&Path>) {
    match path {
        Some(path) => details.push(format!("{label}: {}", path.display())),
        None => details.push(format!("{label}: none")),
    }
}

fn push_env_path_detail(details: &mut Vec<String>, label: &str, name: &str) {
    match env::var_os(name) {
        Some(path) => details.push(format!("{label}: {}", PathBuf::from(path).display())),
        None => details.push(format!("{label}: not set")),
    }
}

fn env_var_present(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn human_output_options(command: &DoctorCommand) -> HumanOutputOptions {
    let term = env::var("TERM").ok();
    let color_enabled = should_enable_color(
        command.no_color,
        env::var_os("NO_COLOR").is_some(),
        term.as_deref(),
        std::io::stdout().is_terminal(),
        supports_color::on(Stream::Stdout).is_some(),
    );
    HumanOutputOptions {
        show_details: !command.summary,
        show_all: command.all,
        ascii: command.ascii,
        color_enabled,
    }
}

fn should_enable_color(
    no_color_flag: bool,
    no_color_env: bool,
    term: Option<&str>,
    stdout_is_tty: bool,
    stream_supports_color: bool,
) -> bool {
    !no_color_flag
        && !no_color_env
        && term != Some("dumb")
        && stdout_is_tty
        && stream_supports_color
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::Mutex;

    use clap::Parser;
    use pretty_assertions::assert_eq;
    use protocol::config_types::SandboxMode;

    use super::*;

    #[derive(Default)]
    struct RecordingProgress {
        events: Mutex<Vec<String>>,
    }

    impl RecordingProgress {
        fn events(&self) -> Vec<String> {
            self.events.lock().expect("events lock").clone()
        }
    }

    impl DoctorProgress for RecordingProgress {
        fn begin(&self, label: &'static str) {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("begin {label}"));
        }

        fn heartbeat(&self, label: &'static str, elapsed: Duration) {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("heartbeat {label} {}", elapsed.as_secs()));
        }

        fn finish(&self, label: &'static str, status: CheckStatus) {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("finish {label} {status:?}"));
        }

        fn settle(&self) {
            self.events
                .lock()
                .expect("events lock")
                .push("settle".to_string());
        }
    }

    fn respond_once(listener: &TcpListener, response: &[u8]) {
        let (mut stream, _) = listener.accept().expect("accept probe request");
        let mut request = [0; 1024];
        let _ = stream.read(&mut request);
        stream.write_all(response).expect("write response");
    }

    #[test]
    fn overall_status_prefers_fail() {
        let checks = vec![
            DoctorCheck::new("a", "config", CheckStatus::Warning, "warning"),
            DoctorCheck::new("b", "auth", CheckStatus::Fail, "fail"),
        ];
        assert_eq!(overall_status(&checks), CheckStatus::Fail);
    }

    #[test]
    fn run_sync_check_notifies_progress() {
        let progress_impl = Arc::new(RecordingProgress::default());
        let progress: Arc<dyn DoctorProgress> = progress_impl.clone();

        let check = run_sync_check("test", progress, || {
            DoctorCheck::new("test", "test", CheckStatus::Ok, "ok")
        });

        assert_eq!(check.status, CheckStatus::Ok);
        assert_eq!(
            progress_impl.events(),
            vec!["begin test".to_string(), "finish test Ok".to_string()]
        );
    }

    #[tokio::test]
    async fn run_async_check_notifies_progress() {
        let progress_impl = Arc::new(RecordingProgress::default());
        let progress: Arc<dyn DoctorProgress> = progress_impl.clone();

        let check = run_async_check("test", progress, async {
            DoctorCheck::new("test", "test", CheckStatus::Warning, "warning")
        })
        .await;

        assert_eq!(check.status, CheckStatus::Warning);
        assert_eq!(
            progress_impl.events(),
            vec!["begin test".to_string(), "finish test Warning".to_string()]
        );
    }

    #[test]
    fn compare_npm_package_roots_detects_match() {
        let running = PathBuf::from("/prefix/lib/node_modules/@openai/codex");
        let npm_root = PathBuf::from("/prefix/lib/node_modules");
        assert_eq!(
            compare_npm_package_roots(&running, &npm_root),
            NpmRootCheck::Match {
                package_root: npm_root.join("@openai").join("codex")
            }
        );
    }

    #[test]
    fn compare_npm_package_roots_detects_mismatch() {
        let running = PathBuf::from("/old/lib/node_modules/@openai/codex");
        let npm_root = PathBuf::from("/new/lib/node_modules");
        assert_eq!(
            compare_npm_package_roots(&running, &npm_root),
            NpmRootCheck::Mismatch {
                running_package_root: running,
                npm_package_root: npm_root.join("@openai").join("codex"),
            }
        );
    }

    #[test]
    fn config_overrides_from_interactive_preserves_global_options() {
        let interactive = TuiCli::parse_from([
            "codex",
            "--oss",
            "--local-provider",
            "ollama",
            "--model",
            "llama3.2",
            "--cd",
            "/tmp",
            "--sandbox",
            "danger-full-access",
            "--ask-for-approval",
            "never",
            "--add-dir",
            "/var/tmp",
        ]);
        let arg0_paths = Arg0DispatchPaths {
            codex_self_exe: Some(PathBuf::from("/bin/codex")),
            codex_linux_sandbox_exe: Some(PathBuf::from("/bin/codex-linux-sandbox")),
            main_execve_wrapper_exe: Some(PathBuf::from("/bin/codex-execve-wrapper")),
        };

        let overrides = config_overrides_from_interactive(&interactive, &arg0_paths);

        assert_eq!(overrides.model.as_deref(), Some("llama3.2"));
        assert_eq!(overrides.model_provider.as_deref(), Some("ollama"));
        assert_eq!(overrides.cwd.as_deref(), Some(Path::new("/tmp")));
        assert_eq!(overrides.approval_policy, Some(AskForApproval::Never));
        assert_eq!(overrides.sandbox_mode, Some(SandboxMode::DangerFullAccess));
        assert_eq!(overrides.show_raw_agent_reasoning, Some(true));
        assert_eq!(
            overrides.additional_writable_roots,
            vec![PathBuf::from("/var/tmp")]
        );
        assert_eq!(overrides.codex_self_exe, arg0_paths.codex_self_exe);
        assert_eq!(
            overrides.codex_linux_sandbox_exe,
            arg0_paths.codex_linux_sandbox_exe
        );
        assert_eq!(
            overrides.main_execve_wrapper_exe,
            arg0_paths.main_execve_wrapper_exe
        );
    }

    #[test]
    fn redacted_json_report_structures_and_sanitizes_details() {
        let report = DoctorReport {
            schema_version: 1,
            generated_at: "0s since unix epoch".to_string(),
            overall_status: CheckStatus::Warning,
            codex_version: "0.0.0".to_string(),
            checks: vec![
                DoctorCheck::new(
                    "mcp.config",
                    "mcp",
                    CheckStatus::Warning,
                    "MCP configuration has optional issues",
                )
                .detail(
                    "optional reachability failed: remote: https://user:pass@example.com/mcp?x=abc (connect failed)",
                )
                .detail("OPENAI_API_KEY: sk-live-secret")
                .detail("duplicate: one")
                .detail("duplicate: two")
                .detail("freeform note")
                .issue(
                    DoctorIssue::new(
                        CheckStatus::Warning,
                        "remote https://user:pass@example.com/mcp?x=abc is unreachable",
                    )
                    .measured("https://user:pass@example.com/mcp?x=abc")
                    .expected("reachable MCP endpoint")
                    .remedy("Check https://user:pass@example.com/help?x=abc.")
                    .field("optional reachability failed"),
                )
                .remediation("Open https://user:pass@example.com/help?x=abc."),
            ],
        };

        let redacted_report = redacted_json_report(&report);
        let redacted = serde_json::to_string(&redacted_report).expect("serialize report");
        let json = serde_json::to_value(redacted_report).expect("report should serialize");

        assert!(!redacted.contains("user:pass"));
        assert!(!redacted.contains("x=abc"));
        assert!(!redacted.contains("sk-live-secret"));
        assert!(redacted.contains("https://example.com/mcp"));
        assert_eq!(json["checks"].is_object(), true);
        assert_eq!(json["checks"]["mcp.config"]["id"], "mcp.config");
        assert_eq!(
            json["checks"]["mcp.config"]["details"]["OPENAI_API_KEY"],
            "<redacted>"
        );
        assert_eq!(
            json["checks"]["mcp.config"]["details"]["duplicate"],
            serde_json::json!(["one", "two"])
        );
        assert_eq!(
            json["checks"]["mcp.config"]["notes"],
            serde_json::json!(["freeform note"])
        );
        assert_eq!(
            json["checks"]["mcp.config"]["issues"][0]["measured"],
            "https://example.com/mcp"
        );
        assert_eq!(
            json["checks"]["mcp.config"]["issues"][0]["remedy"],
            "Check https://example.com/help."
        );
    }

    #[tokio::test]
    async fn mcp_check_ignores_disabled_servers() {
        let disabled_server: McpServerConfig = toml::from_str(
            r#"
                url = "http://127.0.0.1:9/mcp"
                enabled = false
                required = true
                bearer_token_env_var = "CODEX_DOCTOR_DISABLED_MCP_TOKEN"
            "#,
        )
        .expect("should deserialize disabled MCP config");
        let servers = HashMap::from([("disabled".to_string(), disabled_server)]);

        let check = mcp_check_from_servers(&servers).await;

        assert_eq!(check.status, CheckStatus::Ok);
        assert_eq!(check.summary, "MCP configuration is locally consistent");
        assert!(check.details.contains(&"disabled servers: 1".to_string()));
        assert!(
            check
                .details
                .iter()
                .all(|detail| !detail.contains("CODEX_DOCTOR_DISABLED_MCP_TOKEN"))
        );
        assert!(
            check
                .details
                .iter()
                .all(|detail| !detail.contains("reachability failed"))
        );
    }

    #[tokio::test]
    async fn mcp_check_warns_for_optional_http_reachability() {
        let optional_server: McpServerConfig = toml::from_str(
            r#"
                url = "http://127.0.0.1:9/mcp"
            "#,
        )
        .expect("should deserialize optional MCP config");
        let servers = HashMap::from([("optional".to_string(), optional_server)]);

        let check = mcp_check_from_servers(&servers).await;

        assert_eq!(check.status, CheckStatus::Warning);
        assert_eq!(check.summary, "MCP configuration has optional issues");
        assert!(
            check
                .details
                .iter()
                .any(|detail| detail.contains("optional reachability failed: optional:"))
        );
    }

    #[tokio::test]
    async fn mcp_check_fails_required_remote_stdio_env_var() {
        let command = toml::Value::String(
            std::env::current_exe()
                .expect("current exe")
                .to_string_lossy()
                .into_owned(),
        );
        let required_server: McpServerConfig = toml::from_str(&format!(
            r#"
                command = {command}
                required = true
                env_vars = [{{ name = "REMOTE_ONLY_TOKEN", source = "remote" }}]
            "#,
        ))
        .expect("should deserialize required MCP config");
        let servers = HashMap::from([("required".to_string(), required_server)]);

        let check = mcp_check_from_servers(&servers).await;

        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.details.iter().any(|detail| {
            detail.contains(
                "required: env_vars entry `REMOTE_ONLY_TOKEN` uses source `remote`, which requires remote MCP stdio",
            )
        }));
    }

    #[test]
    fn provider_specific_auth_allows_non_openai_provider_without_env_key() {
        let check = provider_specific_auth_check(
            /*requires_openai_auth*/ false,
            /*provider_env_key*/ None,
            /*provider_env_key_instructions*/ None,
            Vec::new(),
            |_| false,
        )
        .expect("non-OpenAI provider should produce a provider-specific check");

        assert_eq!(check.status, CheckStatus::Ok);
        assert_eq!(
            check.summary,
            "OpenAI auth is not required for the active model provider"
        );
    }

    #[test]
    fn provider_specific_auth_fails_when_provider_env_key_is_missing() {
        let check = provider_specific_auth_check(
            /*requires_openai_auth*/ false,
            Some("PROVIDER_API_KEY"),
            Some("Set PROVIDER_API_KEY before running Codex."),
            Vec::new(),
            |_| false,
        )
        .expect("non-OpenAI provider should produce a provider-specific check");

        assert_eq!(check.status, CheckStatus::Fail);
        assert_eq!(
            check.summary,
            "active model provider auth env var is missing"
        );
        assert_eq!(
            check.remediation,
            Some("Set PROVIDER_API_KEY before running Codex.".to_string())
        );
    }

    #[test]
    fn stored_auth_validation_rejects_missing_api_key() {
        let auth = AuthDotJson {
            auth_mode: Some(codex_auth_types::AuthMode::ApiKey),
            openai_api_key: None,
            tokens: None,
            last_refresh: None,
            agent_identity: None,
        };

        assert_eq!(
            stored_auth_issues(&auth, |_| false),
            vec!["API key auth is missing an API key"]
        );
        assert!(stored_auth_issues(&auth, |name| name == OPENAI_API_KEY_ENV_VAR).is_empty());
    }

    #[test]
    fn stored_auth_validation_rejects_missing_chatgpt_tokens() {
        let auth = AuthDotJson {
            auth_mode: None,
            openai_api_key: None,
            tokens: None,
            last_refresh: None,
            agent_identity: None,
        };

        assert_eq!(
            stored_auth_issues(&auth, |_| false),
            vec![
                "ChatGPT auth is missing token data",
                "ChatGPT auth is missing refresh metadata",
            ]
        );
    }

    #[test]
    fn provider_reachability_mode_uses_api_key_auth() {
        let api_key_auth = AuthDotJson {
            auth_mode: Some(codex_auth_types::AuthMode::ApiKey),
            openai_api_key: Some("sk-test".to_string()),
            tokens: None,
            last_refresh: None,
            agent_identity: None,
        };

        assert_eq!(
            provider_auth_reachability_mode_from_auth(
                /*requires_openai_auth*/ true,
                |_| false,
                Some(&api_key_auth),
            ),
            ProviderAuthReachabilityMode::ApiKey
        );
        assert_eq!(
            provider_auth_reachability_mode_from_auth(
                /*requires_openai_auth*/ true,
                |name| name == OPENAI_API_KEY_ENV_VAR,
                /*stored_auth*/ None,
            ),
            ProviderAuthReachabilityMode::ApiKey
        );
    }

    #[test]
    fn provider_reachability_uses_active_provider_endpoint() {
        assert_eq!(
            provider_reachability_plan_from_parts(
                ProviderAuthReachabilityMode::NotRequired,
                "azure",
                "azure",
                Some("https://example.openai.azure.com/openai/v1"),
                /*provider_query_params*/ None,
                /*is_amazon_bedrock*/ false,
                "https://chatgpt.com/backend-api/",
            ),
            ReachabilityPlan {
                description: "provider auth".to_string(),
                endpoints: vec![ReachabilityEndpoint {
                    label: "azure API".to_string(),
                    url: "https://example.openai.azure.com/openai/v1".to_string(),
                    required: true,
                    route_probe_url: None,
                }],
            }
        );
    }

    #[test]
    fn provider_reachability_adds_models_route_probe_for_openai_compatible_base_urls() {
        let query_params = HashMap::from([("api-version".to_string(), "2026-01-01".to_string())]);

        assert_eq!(
            provider_reachability_plan_from_parts(
                ProviderAuthReachabilityMode::NotRequired,
                "custom",
                "Custom",
                Some("https://example.com/openai/v1/"),
                Some(&query_params),
                /*is_amazon_bedrock*/ false,
                "https://chatgpt.com/backend-api/",
            ),
            ReachabilityPlan {
                description: "provider auth".to_string(),
                endpoints: vec![ReachabilityEndpoint {
                    label: "custom API".to_string(),
                    url: "https://example.com/openai/v1/".to_string(),
                    required: true,
                    route_probe_url: Some(
                        "https://example.com/openai/v1/models?api-version=2026-01-01".to_string()
                    ),
                }],
            }
        );
    }

    #[test]
    fn provider_reachability_skips_route_probe_for_bedrock() {
        let plan = provider_reachability_plan_from_parts(
            ProviderAuthReachabilityMode::NotRequired,
            "amazon-bedrock",
            "Amazon Bedrock",
            Some("https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1"),
            /*provider_query_params*/ None,
            /*is_amazon_bedrock*/ true,
            "https://chatgpt.com/backend-api/",
        );

        assert_eq!(plan.endpoints[0].route_probe_url, None);
    }

    #[test]
    fn provider_reachability_api_key_does_not_require_chatgpt() {
        let plan = provider_reachability_plan_from_parts(
            ProviderAuthReachabilityMode::ApiKey,
            "openai",
            "OpenAI",
            /*provider_base_url*/ None,
            /*provider_query_params*/ None,
            /*is_amazon_bedrock*/ false,
            "https://chatgpt.com/backend-api/",
        );

        assert_eq!(
            plan.endpoints,
            vec![ReachabilityEndpoint {
                label: "openai API".to_string(),
                url: "https://api.openai.com/v1".to_string(),
                required: true,
                route_probe_url: Some("https://api.openai.com/v1/models".to_string()),
            }]
        );
    }

    #[test]
    fn provider_reachability_outcome_reports_required_failures() {
        assert_eq!(
            provider_reachability_outcome(/*required_failures*/ 0, /*warnings*/ 1,),
            (
                CheckStatus::Warning,
                "provider endpoint checks returned warnings",
            )
        );
        assert_eq!(
            provider_reachability_outcome(/*required_failures*/ 1, /*warnings*/ 0,),
            (
                CheckStatus::Fail,
                "one or more required provider endpoints are unreachable over HTTP",
            )
        );
    }

    #[tokio::test]
    async fn provider_reachability_route_404_fails_bad_base_url_path() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("listener address");
        let server = std::thread::spawn(move || {
            respond_once(
                &listener,
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            respond_once(
                &listener,
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let plan = provider_reachability_plan_from_parts(
            ProviderAuthReachabilityMode::ApiKey,
            "openai",
            "OpenAI",
            Some(&format!("http://{addr}/xxxx")),
            /*provider_query_params*/ None,
            /*is_amazon_bedrock*/ false,
            "https://chatgpt.com/backend-api/",
        );

        let check = provider_reachability_check(plan).await;
        server.join().expect("probe server thread should finish");

        assert_eq!(check.status, CheckStatus::Fail);
        assert!(
            check
                .details
                .iter()
                .any(|detail| detail.contains("route probe:") && detail.contains("HTTP 404"))
        );
        assert_eq!(check.issues.len(), 1);
        assert_eq!(
            check.issues[0].remedy.as_deref(),
            Some("Set base_url to the provider API root, for example https://api.openai.com/v1")
        );
    }

    #[tokio::test]
    async fn provider_reachability_route_401_keeps_reachability_ok() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("listener address");
        let server = std::thread::spawn(move || {
            respond_once(
                &listener,
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            respond_once(
                &listener,
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let plan = provider_reachability_plan_from_parts(
            ProviderAuthReachabilityMode::ApiKey,
            "openai",
            "OpenAI",
            Some(&format!("http://{addr}/v1")),
            /*provider_query_params*/ None,
            /*is_amazon_bedrock*/ false,
            "https://chatgpt.com/backend-api/",
        );

        let check = provider_reachability_check(plan).await;
        server.join().expect("probe server thread should finish");

        assert_eq!(check.status, CheckStatus::Ok);
        assert!(
            check
                .details
                .iter()
                .any(|detail| detail.contains("route exists (HTTP 401)"))
        );
    }

    #[test]
    fn collect_rollout_stats_counts_nested_rollout_files() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let nested = temp
            .path()
            .join("sessions")
            .join("2026")
            .join("05")
            .join("13");
        std::fs::create_dir_all(&nested).expect("create nested rollout dir");
        std::fs::write(
            nested.join("rollout-2026-05-13T00-00-00-test.jsonl"),
            "12345",
        )
        .expect("write rollout file");
        std::fs::write(nested.join("not-a-rollout.jsonl"), "ignored").expect("write ignored jsonl");

        let stats = collect_rollout_stats(&temp.path().join("sessions"));

        assert_eq!(stats.files, 1);
        assert_eq!(stats.total_bytes, 5);
        assert_eq!(stats.average_bytes(), 5);
        assert_eq!(stats.error, None);
    }

    #[tokio::test]
    async fn http_probe_treats_http_status_as_reachable() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("listener address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept probe request");
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("write response");
        });

        let status = http_probe_url(&format!("http://{addr}/mcp")).await;
        server.join().expect("probe server thread should finish");

        assert_eq!(status, Ok("HTTP 405".to_string()));
    }

    #[tokio::test]
    async fn mcp_http_probe_falls_back_to_get_when_head_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("listener address");
        let server = std::thread::spawn(move || {
            let (mut head_stream, _) = listener.accept().expect("accept HEAD probe request");
            let head = std::thread::spawn(move || {
                let mut request = [0; 1024];
                let _ = head_stream.read(&mut request);
                std::thread::sleep(Duration::from_millis(50));
            });

            let (mut get_stream, _) = listener.accept().expect("accept GET probe request");
            let mut request = [0; 1024];
            let _ = get_stream.read(&mut request);
            get_stream
                .write_all(
                    b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("write response");
            head.join().expect("HEAD holder should finish");
        });

        let status = mcp_http_probe_url_with_timeout(
            &format!("http://{addr}/mcp"),
            Duration::from_millis(10),
        )
        .await;
        server.join().expect("probe server thread should finish");

        assert_eq!(status, Ok("HTTP 405".to_string()));
    }

    #[tokio::test]
    async fn mcp_check_fails_required_missing_stdio_command() {
        let required_server: McpServerConfig = toml::from_str(
            r#"
                command = "definitely-missing-codex-doctor-mcp"
                required = true
            "#,
        )
        .expect("should deserialize required MCP config");
        let servers = HashMap::from([("required".to_string(), required_server)]);

        let check = mcp_check_from_servers(&servers).await;

        assert_eq!(check.status, CheckStatus::Fail);
        assert_eq!(
            check.summary,
            "MCP configuration has failing required inputs or reachability"
        );
        assert!(check.details.iter().any(|detail| {
            detail.contains(
                "required: stdio command \"definitely-missing-codex-doctor-mcp\" is not resolvable",
            )
        }));
    }

    #[cfg(unix)]
    #[test]
    fn read_probe_file_rejects_unreadable_file() {
        use std::os::unix::fs::PermissionsExt;

        let file = tempfile::NamedTempFile::new().expect("create temp file");
        std::fs::write(file.path(), "cert").expect("write temp file");
        let mut permissions = std::fs::metadata(file.path())
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o000);
        std::fs::set_permissions(file.path(), permissions).expect("remove read permissions");

        let result = read_probe_file(file.path());

        let mut permissions = std::fs::metadata(file.path())
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(file.path(), permissions).expect("restore read permissions");
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn executable_path_exists_rejects_non_executable_file() {
        use std::os::unix::fs::PermissionsExt;

        let file = tempfile::NamedTempFile::new().expect("create temp file");
        std::fs::write(file.path(), "#!/bin/sh\n").expect("write temp file");
        let mut permissions = std::fs::metadata(file.path())
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(file.path(), permissions).expect("set non-executable mode");

        let result = executable_path_exists(file.path());

        assert!(result.is_err());
        let mut permissions = std::fs::metadata(file.path())
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(file.path(), permissions).expect("set executable mode");
        assert_eq!(executable_path_exists(file.path()), Ok(()));
    }

    #[test]
    fn should_enable_color_respects_terminal_inputs() {
        assert!(should_enable_color(
            /*no_color_flag*/ false,
            /*no_color_env*/ false,
            Some("xterm-256color"),
            /*stdout_is_tty*/ true,
            /*stream_supports_color*/ true,
        ));
        assert!(!should_enable_color(
            /*no_color_flag*/ true,
            /*no_color_env*/ false,
            Some("xterm-256color"),
            /*stdout_is_tty*/ true,
            /*stream_supports_color*/ true,
        ));
        assert!(!should_enable_color(
            /*no_color_flag*/ false,
            /*no_color_env*/ true,
            Some("xterm-256color"),
            /*stdout_is_tty*/ true,
            /*stream_supports_color*/ true,
        ));
        assert!(!should_enable_color(
            /*no_color_flag*/ false,
            /*no_color_env*/ false,
            Some("dumb"),
            /*stdout_is_tty*/ true,
            /*stream_supports_color*/ true,
        ));
        assert!(!should_enable_color(
            /*no_color_flag*/ false,
            /*no_color_env*/ false,
            Some("xterm-256color"),
            /*stdout_is_tty*/ false,
            /*stream_supports_color*/ true,
        ));
    }

    fn terminal_inputs() -> TerminalCheckInputs {
        TerminalCheckInputs {
            info: TerminalInfo {
                name: TerminalName::Unknown,
                term_program: None,
                version: None,
                term: Some("xterm-256color".to_string()),
                multiplexer: None,
            },
            env: BTreeMap::from([("TERM".to_string(), "xterm-256color".to_string())]),
            present_env: BTreeSet::from(["TERM".to_string()]),
            no_color_flag: false,
            stdin_is_terminal: true,
            stdout_is_terminal: true,
            stderr_is_terminal: true,
            stream_supports_color: true,
            terminal_size: Ok((120, 40)),
            tmux_details: Vec::new(),
        }
    }

    fn set_terminal_env(inputs: &mut TerminalCheckInputs, name: &str, value: &str) {
        inputs.present_env.insert(name.to_string());
        if value.is_empty() {
            inputs.env.remove(name);
        } else {
            inputs.env.insert(name.to_string(), value.to_string());
        }
    }

    #[test]
    fn terminal_check_warns_for_dumb_terminal() {
        let mut inputs = terminal_inputs();
        inputs.info.name = TerminalName::Dumb;
        inputs.info.term = Some("dumb".to_string());
        set_terminal_env(&mut inputs, "TERM", "dumb");

        let check = terminal_check_from_inputs(inputs);

        assert_eq!(check.status, CheckStatus::Fail);
        assert_eq!(
            check.summary,
            "TERM=dumb - colors and cursor control are disabled"
        );
        assert_eq!(check.issues.len(), 1);
        assert_eq!(
            check.issues[0].remedy.as_deref(),
            Some("set TERM to a real value, for example xterm-256color")
        );
    }

    #[test]
    fn terminal_check_warns_for_narrow_terminal() {
        let mut inputs = terminal_inputs();
        inputs.terminal_size = Ok((79, 24));

        let check = terminal_check_from_inputs(inputs);

        assert_eq!(check.status, CheckStatus::Warning);
        assert_eq!(
            check.summary,
            "width 79 cols - output may wrap (recommended >=80)"
        );
        assert_eq!(check.issues[0].expected.as_deref(), Some(">= 80 columns"));
        assert_eq!(
            check.issues[0].remedy.as_deref(),
            Some("resize the window to at least 80 columns")
        );
    }

    #[test]
    fn terminal_check_warns_for_declared_narrow_terminal() {
        let mut inputs = terminal_inputs();
        set_terminal_env(&mut inputs, "COLUMNS", "60");

        let check = terminal_check_from_inputs(inputs);

        assert_eq!(check.status, CheckStatus::Warning);
        assert_eq!(
            check.summary,
            "COLUMNS=60 - output may wrap (recommended >=80)"
        );
        assert!(check.details.contains(&"COLUMNS: 60".to_string()));
        assert_eq!(check.issues[0].fields, vec!["COLUMNS".to_string()]);
    }

    #[test]
    fn terminal_check_warns_for_non_utf8_locale() {
        let mut inputs = terminal_inputs();
        set_terminal_env(&mut inputs, "LANG", "C");

        let check = terminal_check_from_inputs(inputs);

        assert_eq!(check.status, CheckStatus::Warning);
        assert_eq!(
            check.summary,
            "locale is not UTF-8 - unicode glyphs may render incorrectly"
        );
        assert!(check.details.contains(&"effective locale: C".to_string()));
        assert_eq!(
            check.issues[0].remedy.as_deref(),
            Some("export LANG=en_US.UTF-8 or another UTF-8 locale")
        );
    }

    #[test]
    fn terminal_check_warns_for_unreadable_terminfo_path() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let missing = tempdir.path().join("missing-terminfo");
        let mut inputs = terminal_inputs();
        set_terminal_env(&mut inputs, "TERMINFO", &missing.to_string_lossy());

        let check = terminal_check_from_inputs(inputs);

        assert_eq!(check.status, CheckStatus::Fail);
        assert_eq!(
            check.summary,
            "TERMINFO unreadable - terminal capabilities are unknown"
        );
        assert!(
            check
                .details
                .iter()
                .any(|detail| detail.starts_with("TERMINFO: ") && detail.ends_with(" (missing)"))
        );
        assert_eq!(
            check.issues[0].remedy.as_deref(),
            Some("check that $TERMINFO points to a readable directory")
        );
    }

    #[test]
    fn terminal_check_reports_remote_indicators_as_present_only() {
        let mut inputs = terminal_inputs();
        set_terminal_env(&mut inputs, "SSH_CONNECTION", "10.0.0.1 1 10.0.0.2 22");

        let check = terminal_check_from_inputs(inputs);

        assert!(
            check
                .details
                .contains(&"SSH_CONNECTION: present".to_string())
        );
        assert!(
            !check
                .details
                .iter()
                .any(|detail| detail.contains("10.0.0.1"))
        );
    }

    #[test]
    fn terminal_check_keeps_tmux_probe_failures_non_fatal() {
        let mut inputs = terminal_inputs();
        inputs.info.multiplexer = Some(Multiplexer::Tmux { version: None });

        let check = terminal_check_from_inputs(inputs);

        assert_eq!(check.status, CheckStatus::Ok);
        assert_eq!(check.summary, "terminal metadata was detected");
    }

    #[test]
    fn color_output_summary_reports_disabled_reasons() {
        let mut inputs = terminal_inputs();
        inputs.no_color_flag = true;
        assert_eq!(color_output_summary(&inputs), "disabled (--no-color)");

        inputs = terminal_inputs();
        set_terminal_env(&mut inputs, "NO_COLOR", "");
        assert_eq!(color_output_summary(&inputs), "disabled (NO_COLOR)");

        inputs = terminal_inputs();
        inputs.info.term = Some("dumb".to_string());
        set_terminal_env(&mut inputs, "TERM", "dumb");
        assert_eq!(color_output_summary(&inputs), "disabled (TERM=dumb)");

        inputs = terminal_inputs();
        inputs.stdout_is_terminal = false;
        assert_eq!(
            color_output_summary(&inputs),
            "disabled (stdout is not a terminal)"
        );
    }
}
