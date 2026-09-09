use clap::Parser;
use clap::Subcommand;
use runtime_launcher::LauncherPaths;
use runtime_launcher::RunOutcome;
use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "MorpheusLauncher")]
struct Cli {
    #[arg(long, global = true)]
    state_root: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run {
        #[arg(long)]
        app_bundle: PathBuf,
    },
    PrepareFull {
        #[arg(long)]
        request: PathBuf,
    },
    ActivateHot {
        #[arg(long)]
        request: PathBuf,
    },
    AbortFull {
        #[arg(long)]
        transaction: String,
    },
    CommitHot {
        #[arg(long)]
        transaction: String,
    },
    RollbackHot {
        #[arg(long)]
        transaction: String,
    },
    AckFailure {
        #[arg(long)]
        recovery_identity: String,
    },
    Status,
}

#[derive(Serialize)]
struct Success<T: Serialize> {
    ok: bool,
    result: T,
}

fn main() {
    let exit_code = match execute(Cli::parse()) {
        Ok(Some(code)) => code,
        Ok(None) => 0,
        Err(error) => {
            let body = json!({
                "ok": false,
                "error": {
                    "message": error.to_string(),
                }
            });
            println!(
                "{}",
                serde_json::to_string(&body)
                    .unwrap_or_else(|_| "{\"ok\":false,\"error\":{\"message\":\"launcher error\"}}".to_string())
            );
            1
        }
    };
    std::process::exit(exit_code);
}

fn execute(cli: Cli) -> runtime_launcher::Result<Option<i32>> {
    let implicit_start = cli.command.is_none();
    let root = if implicit_start {
        startup_state_root(cli.state_root)?
    } else {
        runtime_launcher::state_root(cli.state_root)?
    };
    let paths = LauncherPaths::new(root);
    match cli.command {
        None => match runtime_launcher::run(&paths, &current_app_bundle()?)? {
            RunOutcome::Exited(code) => Ok(Some(code)),
        },
        Some(Command::Run { app_bundle }) => match runtime_launcher::run(&paths, &app_bundle)? {
            RunOutcome::Exited(code) => Ok(Some(code)),
        },
        Some(Command::PrepareFull { request }) => {
            print_success(runtime_launcher::prepare_full(&paths, &request)?)?;
            Ok(None)
        }
        Some(Command::ActivateHot { request }) => {
            print_success(runtime_launcher::activate_hot(&paths, &request)?)?;
            Ok(None)
        }
        Some(Command::AbortFull { transaction }) => {
            print_success(runtime_launcher::abort_full(&paths, &transaction)?)?;
            Ok(None)
        }
        Some(Command::CommitHot { transaction }) => {
            print_success(runtime_launcher::commit_hot(&paths, &transaction)?)?;
            Ok(None)
        }
        Some(Command::RollbackHot { transaction }) => {
            print_success(runtime_launcher::rollback_hot(&paths, &transaction)?)?;
            Ok(None)
        }
        Some(Command::AckFailure { recovery_identity }) => {
            print_success(json!({
                "acknowledged": runtime_launcher::ack_failure(&paths, &recovery_identity)?
            }))?;
            Ok(None)
        }
        Some(Command::Status) => {
            print_success(runtime_launcher::status(&paths)?)?;
            Ok(None)
        }
    }
}

fn current_app_bundle() -> runtime_launcher::Result<PathBuf> {
    let executable = std::env::current_exe().map_err(|err| runtime_launcher::LauncherError::Io {
        context: "resolve current launcher executable".to_string(),
        source: err,
    })?;
    let bundle = executable
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .ok_or_else(|| {
            runtime_launcher::LauncherError::InvalidRequest(format!(
                "cannot derive app bundle from {}",
                executable.display()
            ))
        })?;
    if bundle.extension().and_then(|value| value.to_str()) != Some("app") {
        return Err(runtime_launcher::LauncherError::InvalidRequest(format!(
            "launcher is not inside a macOS app bundle: {}",
            executable.display()
        )));
    }
    Ok(bundle.to_path_buf())
}

fn startup_state_root(explicit: Option<PathBuf>) -> runtime_launcher::Result<PathBuf> {
    if explicit.is_some() || std::env::var_os("MORPHEUS_RUNTIME_LAUNCHER_HOME").is_some() {
        return runtime_launcher::state_root(explicit);
    }
    if let Some(home) = std::env::var_os("MORPHEUS_HOME") {
        return Ok(PathBuf::from(home).join("runtime-launcher"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".morpheus/runtime-launcher"))
        .ok_or_else(|| {
            runtime_launcher::LauncherError::InvalidRequest(
                "cannot resolve launcher state root".to_string(),
            )
        })
}

fn print_success<T: Serialize>(result: T) -> runtime_launcher::Result<()> {
    let body = Success { ok: true, result };
    let json = serde_json::to_string(&body)
        .map_err(|err| runtime_launcher::LauncherError::Json {
            context: "serialize command result".to_string(),
            source: err,
        })?;
    println!("{json}");
    Ok(())
}
