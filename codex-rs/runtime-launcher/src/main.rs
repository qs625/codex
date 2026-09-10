#[cfg(unix)]
mod supported {
    use clap::Parser;
    use clap::Subcommand;
    use runtime_launcher::CapsuleTarget;
    use runtime_launcher::LauncherPaths;
    use runtime_launcher::RunOutcome;
    use serde::Serialize;
    use serde_json::json;
    use std::path::PathBuf;

    #[derive(Debug, Parser)]
    #[command(name = "RuntimeCapsuleLauncher")]
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
            outer_bundle: PathBuf,
            #[arg(long)]
            target_os: String,
            #[arg(long)]
            target_arch: String,
        },
        SelectCandidate {
            #[arg(long)]
            request: PathBuf,
        },
        Status,
    }

    #[derive(Serialize)]
    struct Success<T: Serialize> {
        ok: bool,
        result: T,
    }

    pub(super) fn main() {
        let exit_code = match execute(Cli::parse()) {
            Ok(Some(code)) => code,
            Ok(None) => 0,
            Err(error) => {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "ok": false,
                        "error": {"message": error.to_string()}
                    }))
                    .unwrap_or_else(|_| {
                        "{\"ok\":false,\"error\":{\"message\":\"launcher error\"}}".to_string()
                    })
                );
                1
            }
        };
        std::process::exit(exit_code);
    }

    fn execute(cli: Cli) -> runtime_launcher::Result<Option<i32>> {
        let root = runtime_launcher::state_root(cli.state_root)?;
        let paths = LauncherPaths::new(root);
        let launcher_path =
            std::env::current_exe().map_err(|error| runtime_launcher::LauncherError::Io {
                context: "resolve launcher executable".to_string(),
                source: error,
            })?;
        match cli.command {
            None => {
                let outer_bundle = std::env::var_os("RUNTIME_CAPSULE_OUTER_BUNDLE")
                    .map(PathBuf::from)
                    .map(Ok)
                    .unwrap_or_else(infer_outer_bundle)?;
                let target = host_target();
                match runtime_launcher::run(&paths, &outer_bundle, target, &launcher_path)? {
                    RunOutcome::Exited(code) => Ok(Some(code)),
                }
            }
            Some(Command::Run {
                outer_bundle,
                target_os,
                target_arch,
            }) => match runtime_launcher::run(
                &paths,
                &outer_bundle,
                CapsuleTarget {
                    os: target_os,
                    arch: target_arch,
                },
                &launcher_path,
            )? {
                RunOutcome::Exited(code) => Ok(Some(code)),
            },
            Some(Command::SelectCandidate { request }) => {
                print_success(runtime_launcher::select_candidate(&paths, &request)?)?;
                Ok(None)
            }
            Some(Command::Status) => {
                print_success(runtime_launcher::status(&paths)?)?;
                Ok(None)
            }
        }
    }

    fn host_target() -> CapsuleTarget {
        CapsuleTarget {
            os: protocol_os(std::env::consts::OS).to_string(),
            arch: protocol_arch(std::env::consts::ARCH).to_string(),
        }
    }

    fn protocol_arch(host_arch: &str) -> &str {
        match host_arch {
            "aarch64" => "arm64",
            "x86_64" => "x64",
            other => other,
        }
    }

    fn protocol_os(host_os: &str) -> &str {
        match host_os {
            "macos" => "darwin",
            other => other,
        }
    }

    fn infer_outer_bundle() -> runtime_launcher::Result<PathBuf> {
        infer_outer_bundle_from_executable(&std::env::current_exe().map_err(|error| {
            runtime_launcher::LauncherError::Io {
                context: "resolve launcher executable".to_string(),
                source: error,
            }
        })?)
    }

    fn infer_outer_bundle_from_executable(
        executable: &std::path::Path,
    ) -> runtime_launcher::Result<PathBuf> {
        let macos = executable.parent().filter(|path| {
            path.file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new("MacOS"))
        });
        let contents = macos.and_then(std::path::Path::parent).filter(|path| {
            path.file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new("Contents"))
        });
        let bundle = contents.and_then(std::path::Path::parent).filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == std::ffi::OsStr::new("app"))
        });
        bundle.map(PathBuf::from).ok_or_else(|| {
            runtime_launcher::LauncherError::InvalidRequest(format!(
                "launcher executable is not inside <bundle>.app/Contents/MacOS: {}",
                executable.display()
            ))
        })
    }

    fn print_success<T: Serialize>(result: T) -> runtime_launcher::Result<()> {
        let json = serde_json::to_string(&Success { ok: true, result }).map_err(|error| {
            runtime_launcher::LauncherError::Json {
                context: "serialize command result".to_string(),
                source: error,
            }
        })?;
        println!("{json}");
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn macos_uses_manifest_darwin_vocabulary() {
            assert_eq!(protocol_os("macos"), "darwin");
            assert_eq!(protocol_os("linux"), "linux");
            assert_eq!(protocol_arch("aarch64"), "arm64");
            assert_eq!(protocol_arch("x86_64"), "x64");
        }

        #[test]
        fn installed_launcher_path_resolves_outer_bundle() {
            assert_eq!(
                infer_outer_bundle_from_executable(std::path::Path::new(
                    "/Applications/Runtime.app/Contents/MacOS/runtime-capsule-launcher"
                ))
                .expect("outer bundle"),
                PathBuf::from("/Applications/Runtime.app")
            );
            assert!(
                infer_outer_bundle_from_executable(std::path::Path::new(
                    "/tmp/runtime-capsule-launcher"
                ))
                .is_err()
            );
        }
    }
}

#[cfg(unix)]
fn main() {
    supported::main();
}

#[cfg(not(unix))]
fn main() {
    eprintln!("Runtime Capsule Launcher is unsupported on this platform");
    std::process::exit(1);
}
