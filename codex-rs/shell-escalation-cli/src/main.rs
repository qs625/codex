#[cfg(not(unix))]
fn main() {
    eprintln!("codex-execve-wrapper is only implemented for UNIX");
    std::process::exit(1);
}

#[cfg(unix)]
use clap::Parser;
#[cfg(unix)]
use tracing_subscriber::EnvFilter;

#[cfg(unix)]
#[derive(Parser)]
struct ExecveWrapperCli {
    file: String,

    #[arg(trailing_var_arg = true)]
    argv: Vec<String>,
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let ExecveWrapperCli { file, argv } = ExecveWrapperCli::parse();
    let exit_code = codex_shell_escalation::run_shell_escalation_execve_wrapper(file, argv).await?;
    std::process::exit(exit_code);
}
