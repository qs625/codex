#[derive(Debug, Parser)]
pub struct DoctorCommand {
    /// Emit a redacted machine-readable report.
    #[arg(long, default_value_t = false)]
    json: bool,

    /// Only show grouped check rows and the final count summary.
    #[arg(long, default_value_t = false)]
    summary: bool,

    /// Expand long lists in detailed human output.
    #[arg(long, default_value_t = false)]
    all: bool,

    /// Disable ANSI color in human output.
    #[arg(long, default_value_t = false)]
    no_color: bool,

    /// Use ASCII status labels and separators in human output.
    #[arg(long, default_value_t = false)]
    ascii: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    Ok,
    Warning,
    Fail,
}

/// Machine-readable doctor output shared by human and JSON renderers.
///
/// The schema is intentionally flat: each check carries its own category,
/// status, details, remediation, and duration so support tooling can filter or
/// redact individual rows without understanding the renderer's section layout.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorReport {
    schema_version: u32,
    generated_at: String,
    overall_status: CheckStatus,
    codex_version: String,
    checks: Vec<DoctorCheck>,
}

/// One diagnostic result in the doctor report.
///
/// Summaries are safe for compact human output. Details may include local paths
/// or command output and are redacted before rendering or JSON serialization.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorCheck {
    id: String,
    category: String,
    status: CheckStatus,
    summary: String,
    details: Vec<String>,
    issues: Vec<DoctorIssue>,
    remediation: Option<String>,
    duration_ms: u64,
}

/// Structured cause/remedy metadata for a non-ok doctor check.
///
/// Human output uses issues to make warnings and failures self-explanatory:
/// the row headline says what is wrong, matching detail rows show measured vs.
/// expected values, and remedies are printed as explicit next actions.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorIssue {
    severity: CheckStatus,
    cause: String,
    measured: Option<String>,
    expected: Option<String>,
    remedy: Option<String>,
    fields: Vec<String>,
}

impl DoctorIssue {
    fn new(severity: CheckStatus, cause: impl Into<String>) -> Self {
        Self {
            severity,
            cause: cause.into(),
            measured: None,
            expected: None,
            remedy: None,
            fields: Vec::new(),
        }
    }

    fn measured(mut self, measured: impl Into<String>) -> Self {
        self.measured = Some(measured.into());
        self
    }

    fn expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    fn remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remedy = Some(remedy.into());
        self
    }

    fn field(mut self, field: impl Into<String>) -> Self {
        self.fields.push(field.into());
        self
    }
}

impl DoctorCheck {
    fn new(
        id: impl Into<String>,
        category: impl Into<String>,
        status: CheckStatus,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status,
            summary: summary.into(),
            details: Vec::new(),
            issues: Vec::new(),
            remediation: None,
            duration_ms: 0,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.details.push(detail.into());
        self
    }

    fn details(mut self, details: Vec<String>) -> Self {
        self.details.extend(details);
        self
    }

    fn remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    fn issue(mut self, issue: DoctorIssue) -> Self {
        self.issues.push(issue);
        self
    }
}

/// Builds, renders, and exits according to the current doctor report.
///
/// This is the CLI entry point for codex doctor. It does not repair issues;
/// failures are represented in the report and cause a non-zero process exit so
/// scripts can distinguish a clean environment from one that needs attention.
pub async fn run_doctor(
    command: DoctorCommand,
    root_config_overrides: CliConfigOverrides,
    interactive: &TuiCli,
    arg0_paths: &Arg0DispatchPaths,
) -> anyhow::Result<()> {
    let report = build_report(&command, root_config_overrides, interactive, arg0_paths).await;

    if command.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&redacted_json_report(&report))?
        );
    } else {
        print!(
            "{}",
            render_human_report(&report, human_output_options(&command))
        );
    }

    if report.overall_status == CheckStatus::Fail {
        std::process::exit(1);
    }

    Ok(())
}

async fn build_report(
    command: &DoctorCommand,
    root_config_overrides: CliConfigOverrides,
    interactive: &TuiCli,
    arg0_paths: &Arg0DispatchPaths,
) -> DoctorReport {
    let progress = doctor_progress(command.json);
    let mut checks = Vec::new();
    checks.push(run_sync_check("installation", progress.clone(), || {
        installation_check(!command.summary)
    }));
    checks.push(run_sync_check("runtime", progress.clone(), runtime_check));
    checks.push(run_sync_check("search", progress.clone(), search_check));

    progress.begin("config");
    let config_result = load_config(root_config_overrides, interactive, arg0_paths).await;
    match &config_result {
        Ok(config) => {
            let auth_manager =
                AuthManager::shared_from_config(config, /*enable_codex_api_key_env*/ true).await;
            let reachability_plan = provider_reachability_plan(config);
            let (
                config_check,
                auth_check,
                updates_check,
                network_check,
                websocket_check,
                mcp_check,
                sandbox_check,
                terminal_check,
                state_check,
                background_server_check,
                reachability_check,
            ) = tokio::join!(
                async { run_sync_check("config", progress.clone(), || config_check(config)) },
                async { run_sync_check("auth", progress.clone(), || auth_check(config)) },
                async { run_sync_check("updates", progress.clone(), || updates_check(config)) },
                async { run_sync_check("network", progress.clone(), network_check) },
                run_async_check(
                    "websocket",
                    progress.clone(),
                    websocket_reachability_check(config, Some(auth_manager)),
                ),
                run_async_check("MCP", progress.clone(), mcp_check(config)),
                async {
                    run_sync_check("sandbox", progress.clone(), || {
                        sandbox_check(config, arg0_paths)
                    })
                },
                async {
                    run_sync_check("terminal", progress.clone(), || {
                        terminal_check(command.no_color)
                    })
                },
                run_async_check("state", progress.clone(), state_check(config)),
                async {
                    run_sync_check("app-server", progress.clone(), || {
                        background_server_check(config)
                    })
                },
                run_async_check(
                    "provider reachability",
                    progress.clone(),
                    provider_reachability_check(reachability_plan),
                ),
            );
            checks.extend([
                config_check,
                auth_check,
                updates_check,
                network_check,
                websocket_check,
                mcp_check,
                sandbox_check,
                terminal_check,
                state_check,
                background_server_check,
                reachability_check,
            ]);
        }
        Err(err) => {
            let reachability_plan = default_reachability_plan();
            let (config_check, network_check, terminal_check, state_check, reachability_check) = tokio::join!(
                async {
                    run_sync_check("config", progress.clone(), || {
                        DoctorCheck::new(
                            "config.load",
                            "config",
                            CheckStatus::Fail,
                            "config could not be loaded",
                        )
                        .detail(err.to_string())
                        .remediation("Fix the reported config error, then rerun codex doctor.")
                    })
                },
                async { run_sync_check("network", progress.clone(), network_check) },
                async {
                    run_sync_check("terminal", progress.clone(), || {
                        terminal_check(command.no_color)
                    })
                },
                async { run_sync_check("state", progress.clone(), fallback_state_check) },
                run_async_check(
                    "provider reachability",
                    progress.clone(),
                    provider_reachability_check(reachability_plan),
                ),
            );
            checks.extend([
                config_check,
                network_check,
                terminal_check,
                state_check,
                reachability_check,
            ]);
        }
    }

    progress.settle();

    let overall_status = overall_status(&checks);
    DoctorReport {
        schema_version: 1,
        generated_at: generated_at(),
        overall_status,
        codex_version: env!("CARGO_PKG_VERSION").to_string(),
        checks,
    }
}

async fn load_config(
    root_config_overrides: CliConfigOverrides,
    interactive: &TuiCli,
    arg0_paths: &Arg0DispatchPaths,
) -> anyhow::Result<Config> {
    let mut cli_kv_overrides = root_config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    if interactive.web_search {
        cli_kv_overrides.push((
            "web_search".to_string(),
            toml::Value::String("live".to_string()),
        ));
    }

    let overrides = ConfigOverrides {
        ephemeral: Some(true),
        ..config_overrides_from_interactive(interactive, arg0_paths)
    };

    crate::config_builder()
        .cli_overrides(cli_kv_overrides)
        .harness_overrides(overrides)
        .build()
        .await
        .context("failed to load Codex config")
}

fn config_overrides_from_interactive(
    interactive: &TuiCli,
    arg0_paths: &Arg0DispatchPaths,
) -> ConfigOverrides {
    let approval_policy = if interactive.dangerously_bypass_approvals_and_sandbox {
        Some(AskForApproval::Never)
    } else {
        interactive.approval_policy.map(Into::into)
    };
    let sandbox_mode = if interactive.dangerously_bypass_approvals_and_sandbox {
        Some(protocol::config_types::SandboxMode::DangerFullAccess)
    } else {
        interactive.sandbox_mode.map(Into::into)
    };
    ConfigOverrides {
        model: interactive.model.clone(),
        config_profile: interactive.config_profile.clone(),
        approval_policy,
        sandbox_mode,
        cwd: interactive.cwd.clone(),
        model_provider: interactive
            .oss
            .then(|| interactive.oss_provider.clone())
            .flatten(),
        codex_self_exe: arg0_paths.codex_self_exe.clone(),
        codex_linux_sandbox_exe: arg0_paths.codex_linux_sandbox_exe.clone(),
        main_execve_wrapper_exe: arg0_paths.main_execve_wrapper_exe.clone(),
        show_raw_agent_reasoning: interactive.oss.then_some(true),
        additional_writable_roots: interactive.add_dir.clone(),
        ..Default::default()
    }
}

/// JSON support report emitted by `codex doctor --json`.
///
/// The report is keyed by check id so support tooling can fetch paths like
/// `checks["terminal.metadata"]` without scanning arrays. Human rendering can
/// reorder or group rows independently, but this JSON shape should stay stable
/// across cosmetic output changes.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonDoctorReport {
    schema_version: u32,
    generated_at: String,
    overall_status: CheckStatus,
    codex_version: String,
    checks: BTreeMap<String, JsonDoctorCheck>,
}

/// One redacted check in the JSON support report.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonDoctorCheck {
    id: String,
    category: String,
    status: CheckStatus,
    summary: String,
    details: BTreeMap<String, JsonDetailValue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    issues: Vec<JsonDoctorIssue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
    remediation: Option<String>,
    duration_ms: u64,
}

/// One redacted issue in the JSON support report.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonDoctorIssue {
    severity: CheckStatus,
    cause: String,
    measured: Option<String>,
    expected: Option<String>,
    remedy: Option<String>,
    fields: Vec<String>,
}

/// JSON detail value that preserves repeated detail keys without inventing names.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum JsonDetailValue {
    One(String),
    Many(Vec<String>),
}

impl JsonDetailValue {
    fn push(&mut self, value: String) {
        match self {
            JsonDetailValue::One(previous) => {
                *self = JsonDetailValue::Many(vec![std::mem::take(previous), value]);
            }
            JsonDetailValue::Many(values) => values.push(value),
        }
    }
}

fn redacted_json_report(report: &DoctorReport) -> JsonDoctorReport {
    let checks = report
        .checks
        .iter()
        .map(|check| {
            let json_check = redacted_json_check(check);
            (check.id.clone(), json_check)
        })
        .collect();
    JsonDoctorReport {
        schema_version: report.schema_version,
        generated_at: report.generated_at.clone(),
        overall_status: report.overall_status,
        codex_version: report.codex_version.clone(),
        checks,
    }
}

fn redacted_json_check(check: &DoctorCheck) -> JsonDoctorCheck {
    let (details, notes) = structured_json_details(&check.details);
    JsonDoctorCheck {
        id: check.id.clone(),
        category: check.category.clone(),
        status: check.status,
        summary: check.summary.clone(),
        details,
        issues: check.issues.iter().map(redacted_json_issue).collect(),
        notes,
        remediation: check.remediation.as_deref().map(redact_detail),
        duration_ms: check.duration_ms,
    }
}

fn redacted_json_issue(issue: &DoctorIssue) -> JsonDoctorIssue {
    JsonDoctorIssue {
        severity: issue.severity,
        cause: redact_detail(&issue.cause),
        measured: issue.measured.as_deref().map(redact_detail),
        expected: issue.expected.as_deref().map(redact_detail),
        remedy: issue.remedy.as_deref().map(redact_detail),
        fields: issue
            .fields
            .iter()
            .map(|field| redact_detail(field))
            .collect(),
    }
}

/// Converts redacted `label: value` detail strings into JSON key/value fields.
///
/// Detail strings that do not follow the doctor detail convention are preserved
/// as notes instead of being dropped. Repeated labels become arrays so callers
/// can still retrieve the common scalar case directly while keeping all values.
fn structured_json_details(details: &[String]) -> (BTreeMap<String, JsonDetailValue>, Vec<String>) {
    let mut structured: BTreeMap<String, JsonDetailValue> = BTreeMap::new();
    let mut notes = Vec::new();
    for detail in details {
        let redacted = redact_detail(detail);
        let Some((key, value)) = redacted.split_once(": ") else {
            notes.push(redacted);
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            notes.push(redacted);
            continue;
        }
        let value = value.to_string();
        match structured.get_mut(key) {
            Some(existing) => existing.push(value),
            None => {
                structured.insert(key.to_string(), JsonDetailValue::One(value));
            }
        }
    }
    (structured, notes)
}

fn run_sync_check(
    label: &'static str,
    progress: Arc<dyn DoctorProgress>,
    f: impl FnOnce() -> DoctorCheck,
) -> DoctorCheck {
    progress.begin(label);
    let start = Instant::now();
    let mut check = f();
    check.duration_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    progress.finish(label, check.status);
    check
}

async fn run_async_check<Fut>(
    label: &'static str,
    progress: Arc<dyn DoctorProgress>,
    future: Fut,
) -> DoctorCheck
where
    Fut: Future<Output = DoctorCheck>,
{
    progress.begin(label);
    let start = Instant::now();
    tokio::pin!(future);
    let mut progress_interval = tokio::time::interval(SLOW_CHECK_PROGRESS_INTERVAL);
    loop {
        tokio::select! {
            mut check = &mut future => {
                check.duration_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
                progress.finish(label, check.status);
                return check;
            }
            _ = progress_interval.tick() => {
                let elapsed = start.elapsed();
                if elapsed >= SLOW_CHECK_PROGRESS_THRESHOLD {
                    progress.heartbeat(label, elapsed);
                }
            }
        }
    }
}

fn overall_status(checks: &[DoctorCheck]) -> CheckStatus {
    if checks.iter().any(|check| check.status == CheckStatus::Fail) {
        CheckStatus::Fail
    } else if checks
        .iter()
        .any(|check| check.status == CheckStatus::Warning)
    {
        CheckStatus::Warning
    } else {
        CheckStatus::Ok
    }
}

fn generated_at() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => {
            let seconds = duration.as_secs();
            format!("{seconds}s since unix epoch")
        }
        Err(_) => "unknown".to_string(),
    }
}

fn installation_check(show_details: bool) -> DoctorCheck {
    let mut details = Vec::new();
    let current_exe = env::current_exe().ok();
    push_path_detail(&mut details, "current executable", current_exe.as_deref());
    let inherited_managed_env = inherited_managed_env_for_cargo_binary(current_exe.as_deref());
    let install_context = doctor_install_context(current_exe.as_deref());
    details.push(format!(
        "install context: {}",
        describe_install_context(&install_context)
    ));
    if inherited_managed_env {
        details.push(
            "ignored inherited package-manager launch env for cargo-built binary".to_string(),
        );
    }
    details.push(format!(
        "managed by npm: {}",
        doctor_managed_by_npm(current_exe.as_deref())
    ));
    details.push(format!(
        "managed by bun: {}",
        env::var_os("CODEX_MANAGED_BY_BUN").is_some()
    ));
    push_env_path_detail(
        &mut details,
        "managed package root",
        "CODEX_MANAGED_PACKAGE_ROOT",
    );

    let path_entries = codex_path_entries();
    let mut status = CheckStatus::Ok;
    let mut summary = "installation looks consistent".to_string();
    let mut remediation = None;

    if path_entries.len() > 1 {
        details.push(format!("PATH codex entries: {}", path_entries.len()));
    }
    if show_details || path_entries.len() > 1 {
        details.extend(
            path_entries
                .iter()
                .enumerate()
                .map(|(index, path)| format!("PATH codex #{}: {path}", index + 1)),
        );
    }

    if doctor_managed_by_npm(current_exe.as_deref()) {
        match npm_global_root_check() {
            NpmRootCheck::Match { package_root } => {
                details.push(format!("npm update target: {}", package_root.display()));
            }
            NpmRootCheck::Mismatch {
                running_package_root,
                npm_package_root,
            } => {
                status = CheckStatus::Fail;
                summary =
                    "npm install -g @openai/codex would update a different install".to_string();
                remediation = Some(format!(
                    "Fix PATH or npm prefix so the running package root ({}) matches the npm global package root ({}).",
                    running_package_root.display(),
                    npm_package_root.display()
                ));
                details.push(format!(
                    "running package root: {}",
                    running_package_root.display()
                ));
                details.push(format!("npm package root: {}", npm_package_root.display()));
            }
            NpmRootCheck::MissingPackageRoot => {
                status = status.max(CheckStatus::Warning);
                summary = "npm-managed launch is missing package-root provenance".to_string();
                remediation = Some(
                    "Reinstall or update Codex so the JS shim provides CODEX_MANAGED_PACKAGE_ROOT."
                        .to_string(),
                );
            }
            NpmRootCheck::NpmUnavailable(error) => {
                status = status.max(CheckStatus::Warning);
                summary = "npm-managed launch could not inspect npm global root".to_string();
                details.push(format!("npm root -g failed: {error}"));
            }
        }
    }

    let mut check = DoctorCheck::new("installation", "install", status, summary).details(details);
    if let Some(remediation) = remediation {
        check = check.remediation(remediation);
    }
    check
}

fn doctor_install_context(current_exe: Option<&Path>) -> InstallContext {
    if inherited_managed_env_for_cargo_binary(current_exe) {
        InstallContext::Other
    } else {
        InstallContext::current().clone()
    }
}

fn doctor_managed_by_npm(current_exe: Option<&Path>) -> bool {
    env::var_os("CODEX_MANAGED_BY_NPM").is_some()
        && !inherited_managed_env_for_cargo_binary(current_exe)
}

fn inherited_managed_env_for_cargo_binary(current_exe: Option<&Path>) -> bool {
    if env::var_os("CODEX_MANAGED_BY_NPM").is_none()
        && env::var_os("CODEX_MANAGED_BY_BUN").is_none()
    {
        return false;
    }

    let Some(current_exe) = current_exe else {
        return false;
    };
    let components = current_exe
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    components
        .windows(2)
        .any(|window| window[0] == "target" && matches!(window[1].as_ref(), "debug" | "release"))
}

fn describe_install_context(context: &InstallContext) -> String {
    match context {
        InstallContext::Standalone {
            release_dir,
            resources_dir,
            platform,
        } => {
            let platform = match platform {
                StandalonePlatform::Unix => "unix",
                StandalonePlatform::Windows => "windows",
            };
            let resources = resources_dir
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "none".to_string());
            format!(
                "standalone ({platform}, release {}, resources {resources})",
                release_dir.display()
            )
        }
        InstallContext::Npm => "npm".to_string(),
        InstallContext::Bun => "bun".to_string(),
        InstallContext::Brew => "brew".to_string(),
        InstallContext::Other => "other".to_string(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum NpmRootCheck {
    Match {
        package_root: PathBuf,
    },
    Mismatch {
        running_package_root: PathBuf,
        npm_package_root: PathBuf,
    },
    MissingPackageRoot,
    NpmUnavailable(String),
}

fn npm_global_root_check() -> NpmRootCheck {
    let Some(running_package_root) = env::var_os("CODEX_MANAGED_PACKAGE_ROOT").map(PathBuf::from)
    else {
        return NpmRootCheck::MissingPackageRoot;
    };

    let output = match run_command("npm", ["root", "-g"]) {
        Ok(output) => output,
        Err(err) => return NpmRootCheck::NpmUnavailable(err),
    };
    let Some(npm_root) = output.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return NpmRootCheck::NpmUnavailable("empty output from npm root -g".to_string());
    };

    compare_npm_package_roots(&running_package_root, &PathBuf::from(npm_root))
}

fn compare_npm_package_roots(running_package_root: &Path, npm_root: &Path) -> NpmRootCheck {
    let npm_package_root = npm_root.join("@openai").join("codex");
    let running = normalize_path_for_compare(running_package_root);
    let target = normalize_path_for_compare(&npm_package_root);
    if running == target {
        NpmRootCheck::Match {
            package_root: npm_package_root,
        }
    } else {
        NpmRootCheck::Mismatch {
            running_package_root: running_package_root.to_path_buf(),
            npm_package_root,
        }
    }
}

fn normalize_path_for_compare(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let raw = canonical.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        raw.to_ascii_lowercase()
    } else {
        raw
    }
}

fn display_list<T: AsRef<str>>(items: &[T]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn codex_path_entries() -> Vec<String> {
    #[cfg(windows)]
    let result = run_command("where", ["codex"]);
    #[cfg(not(windows))]
    let result = run_command("which", ["-a", "codex"]);

    result
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn run_command<I, S>(program: &str, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(format!("exited with status {}", output.status));
        }
        return Err(stderr);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn config_check(config: &Config) -> DoctorCheck {
    let mut details = Vec::new();
    details.push(format!("CODEX_HOME: {}", config.codex_home.display()));
    details.push(format!("cwd: {}", config.cwd.display()));
    details.push(format!(
        "model: {}",
        config.model.as_deref().unwrap_or("<default>")
    ));
    details.push(format!("model provider: {}", config.model_provider_id));
    details.push(format!("log dir: {}", config.log_dir.display()));
    details.push(format!("sqlite home: {}", config.sqlite_home.display()));
    details.push(format!("mcp servers: {}", config.mcp_servers.get().len()));
    feature_flag_details(config, &mut details);
    config_toml_details(config, &mut details);

    let status = if config.startup_warnings.is_empty() {
        CheckStatus::Ok
    } else {
        details.extend(
            config
                .startup_warnings
                .iter()
                .map(|warning| format!("startup warning: {warning}")),
        );
        CheckStatus::Warning
    };

    DoctorCheck::new("config.load", "config", status, "config loaded").details(details)
}

fn feature_flag_details(config: &Config, details: &mut Vec<String>) {
    let features = config.features.get();
    let enabled_features = FEATURES
        .iter()
        .filter(|spec| features.enabled(spec.id))
        .map(|spec| spec.key)
        .collect::<Vec<_>>();
    let overrides = FEATURES
        .iter()
        .filter(|spec| features.enabled(spec.id) != spec.default_enabled)
        .map(|spec| format!("{}={}", spec.key, features.enabled(spec.id)))
        .collect::<Vec<_>>();
    details.push(format!("feature flags enabled: {}", enabled_features.len()));
    details.push(format!(
        "enabled feature flags: {}",
        display_list(&enabled_features)
    ));
    details.push(format!(
        "feature flag overrides: {}",
        display_list(&overrides)
    ));
    for usage in features.legacy_feature_usages() {
        details.push(format!(
            "legacy feature flag: {} -> {}",
            usage.alias,
            usage.feature.key()
        ));
    }
}

fn config_toml_details(config: &Config, details: &mut Vec<String>) {
    let config_path = config.codex_home.join(CONFIG_TOML_FILE);
    details.push(format!("config.toml: {}", config_path.display()));
    match std::fs::read_to_string(&config_path) {
        Ok(contents) => match toml::from_str::<toml::Value>(&contents) {
            Ok(_) => details.push("config.toml parse: ok".to_string()),
            Err(err) => details.push(format!("config.toml parse: {err}")),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            details.push("config.toml: missing".to_string());
        }
        Err(err) => details.push(format!("config.toml read: {err}")),
    }
}

fn auth_check(config: &Config) -> DoctorCheck {
    let mut details = Vec::new();
    let auth_path = config.codex_home.join("auth.json");
    details.push(format!(
        "auth storage mode: {:?}",
        config.cli_auth_credentials_store_mode
    ));
    details.push(format!("auth file: {}", auth_path.display()));

    let env_auth_vars = [
        OPENAI_API_KEY_ENV_VAR,
        CODEX_API_KEY_ENV_VAR,
        CODEX_ACCESS_TOKEN_ENV_VAR,
    ]
    .into_iter()
    .filter(|name| env_var_present(name))
    .collect::<Vec<_>>();
    if !env_auth_vars.is_empty() {
        details.push(format!(
            "auth env vars present: {}",
            env_auth_vars.join(", ")
        ));
    }
    if let Some(check) = provider_specific_auth_check(
        config.model_provider.requires_openai_auth,
        config.model_provider.env_key.as_deref(),
        config.model_provider.env_key_instructions.as_deref(),
        details.clone(),
        env_var_present,
    ) {
        return check;
    }

    match load_auth_dot_json(&config.codex_home, config.cli_auth_credentials_store_mode) {
        Ok(Some(auth)) => {
            details.push(format!("stored auth mode: {}", stored_auth_mode(&auth)));
            details.push(format!("stored API key: {}", auth.openai_api_key.is_some()));
            details.push(format!("stored ChatGPT tokens: {}", auth.tokens.is_some()));
            details.push(format!(
                "stored agent identity: {}",
                auth.agent_identity.is_some()
            ));
            let auth_issues = stored_auth_issues(&auth, env_var_present);
            details.extend(
                auth_issues
                    .iter()
                    .map(|issue| format!("stored auth issue: {issue}")),
            );
            let status = if !auth_issues.is_empty() && env_auth_vars.is_empty() {
                CheckStatus::Fail
            } else if !auth_issues.is_empty() || env_auth_vars.len() > 1 {
                CheckStatus::Warning
            } else {
                CheckStatus::Ok
            };
            let summary = match status {
                CheckStatus::Ok => "auth is configured",
                CheckStatus::Warning if !auth_issues.is_empty() => {
                    "auth is provided by environment, but stored credentials are incomplete"
                }
                CheckStatus::Warning => {
                    "auth is configured, but multiple auth env vars are present"
                }
                CheckStatus::Fail => "stored credentials are incomplete",
            };
            let mut check =
                DoctorCheck::new("auth.credentials", "auth", status, summary).details(details);
            if status == CheckStatus::Fail {
                check =
                    check.remediation("Run codex login again or provide a supported auth env var.");
            }
            check
        }
        Ok(None) if !env_auth_vars.is_empty() => DoctorCheck::new(
            "auth.credentials",
            "auth",
            CheckStatus::Ok,
            "auth is provided by environment",
        )
        .details(details),
        Ok(None) => DoctorCheck::new(
            "auth.credentials",
            "auth",
            CheckStatus::Fail,
            "no Codex credentials were found",
        )
        .details(details)
        .remediation("Run codex login or provide an API key through a supported auth env var."),
        Err(err) => DoctorCheck::new(
            "auth.credentials",
            "auth",
            CheckStatus::Fail,
            "stored credentials could not be read",
        )
        .detail(err.to_string())
        .remediation("Fix auth storage access or run codex login again."),
    }
}

fn provider_specific_auth_check(
    requires_openai_auth: bool,
    provider_env_key: Option<&str>,
    provider_env_key_instructions: Option<&str>,
    mut details: Vec<String>,
    env_var_present: impl Fn(&str) -> bool,
) -> Option<DoctorCheck> {
    details.push(format!(
        "model provider requires OpenAI auth: {requires_openai_auth}"
    ));
    if requires_openai_auth {
        return None;
    }

    match provider_env_key {
        Some(env_key) if env_var_present(env_key) => {
            details.push(format!("provider auth env var: {env_key} (present)"));
            Some(
                DoctorCheck::new(
                    "auth.credentials",
                    "auth",
                    CheckStatus::Ok,
                    "auth is provided by the active model provider",
                )
                .details(details),
            )
        }
        Some(env_key) => {
            details.push(format!("provider auth env var: {env_key} (missing)"));
            let remediation = provider_env_key_instructions
                .map(str::to_string)
                .unwrap_or_else(|| format!("Set {env_key} for the active model provider."));
            Some(
                DoctorCheck::new(
                    "auth.credentials",
                    "auth",
                    CheckStatus::Fail,
                    "active model provider auth env var is missing",
                )
                .details(details)
                .remediation(remediation),
            )
        }
        None => Some(
            DoctorCheck::new(
                "auth.credentials",
                "auth",
                CheckStatus::Ok,
                "OpenAI auth is not required for the active model provider",
            )
            .details(details),
        ),
    }
}

fn stored_auth_mode(auth: &codex_login::AuthDotJson) -> &'static str {
    match stored_auth_mode_value(auth) {
        codex_auth_types::AuthMode::ApiKey => "api_key",
        codex_auth_types::AuthMode::Chatgpt => "chatgpt",
        codex_auth_types::AuthMode::ChatgptAuthTokens => "chatgpt_auth_tokens",
        codex_auth_types::AuthMode::AgentIdentity => "agent_identity",
    }
}
