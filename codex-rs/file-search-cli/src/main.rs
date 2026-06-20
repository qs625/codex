use std::io::IsTerminal;
use std::num::NonZero;
use std::path::Path;
use std::path::PathBuf;

use clap::ArgAction;
use clap::Parser;
use codex_file_search::FileMatch;
use codex_file_search::FileSearchOptions;
use codex_file_search::FileSearchResults;
use codex_file_search::run;
use serde_json::json;
use tokio::process::Command;

/// Fuzzy matches filenames under a directory.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Whether to output results in JSON format.
    #[clap(long, default_value = "false")]
    json: bool,

    /// Maximum number of results to return.
    #[clap(long, short = 'l', default_value = "64")]
    limit: NonZero<usize>,

    /// Directory to search.
    #[clap(long, short = 'C')]
    cwd: Option<PathBuf>,

    /// Include matching file indices in the output.
    #[arg(long, default_value = "false")]
    compute_indices: bool,

    // While it is common to default to the number of logical CPUs when creating
    // a thread pool, empirically, the I/O of the filetree traversal offers
    // limited parallelism and is the bottleneck, so using a smaller number of
    // threads is more efficient. (Empirically, using more than 2 threads doesn't seem to provide much benefit.)
    //
    /// Number of worker threads to use.
    #[clap(long, default_value = "2")]
    threads: NonZero<usize>,

    /// Exclude patterns
    #[arg(short, long, action = ArgAction::Append)]
    exclude: Vec<String>,

    /// Search pattern.
    pattern: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let reporter = StdioReporter {
        write_output_as_json: cli.json,
        show_indices: cli.compute_indices && std::io::stdout().is_terminal(),
    };
    run_main(cli, reporter).await?;
    Ok(())
}

async fn run_main(cli: Cli, reporter: StdioReporter) -> anyhow::Result<()> {
    let search_directory = match cli.cwd {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    let pattern_text = match cli.pattern {
        Some(pattern) => pattern,
        None => {
            reporter.warn_no_search_pattern(&search_directory);
            #[cfg(unix)]
            Command::new("ls")
                .arg("-al")
                .current_dir(search_directory)
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .await?;
            #[cfg(windows)]
            {
                Command::new("cmd")
                    .arg("/c")
                    .arg(search_directory)
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::inherit())
                    .status()
                    .await?;
            }
            return Ok(());
        }
    };

    let FileSearchResults {
        total_match_count,
        matches,
    } = run(
        &pattern_text,
        vec![search_directory.to_path_buf()],
        FileSearchOptions {
            limit: cli.limit,
            exclude: cli.exclude,
            threads: cli.threads,
            compute_indices: cli.compute_indices,
            respect_gitignore: true,
        },
        /*cancel_flag*/ None,
    )?;
    let match_count = matches.len();
    let matches_truncated = total_match_count > match_count;

    for file_match in matches {
        reporter.report_match(&file_match);
    }
    if matches_truncated {
        reporter.warn_matches_truncated(total_match_count, match_count);
    }

    Ok(())
}

struct StdioReporter {
    write_output_as_json: bool,
    show_indices: bool,
}

impl StdioReporter {
    fn report_match(&self, file_match: &FileMatch) {
        if self.write_output_as_json {
            #[allow(clippy::unwrap_used)]
            let json = serde_json::to_string(file_match).unwrap();
            println!("{json}");
        } else if self.show_indices {
            #[allow(clippy::expect_used)]
            let indices = file_match
                .indices
                .as_ref()
                .expect("--compute-indices was specified");
            // `indices` is guaranteed to be sorted in ascending order. Instead
            // of calling `contains` for every character (which would be O(N^2)
            // in the worst-case), walk through the `indices` vector once while
            // iterating over the characters.
            let mut indices_iter = indices.iter().peekable();

            for (i, c) in file_match.path.to_string_lossy().chars().enumerate() {
                match indices_iter.peek() {
                    Some(next) if **next == i as u32 => {
                        // ANSI escape code for bold: \x1b[1m ... \x1b[0m
                        print!("\x1b[1m{c}\x1b[0m");
                        // advance the iterator since we've consumed this index
                        indices_iter.next();
                    }
                    _ => {
                        print!("{c}");
                    }
                }
            }
            println!();
        } else {
            println!("{}", file_match.path.to_string_lossy());
        }
    }

    fn warn_matches_truncated(&self, total_match_count: usize, shown_match_count: usize) {
        if self.write_output_as_json {
            let value = json!({"matches_truncated": true});
            #[allow(clippy::unwrap_used)]
            let json = serde_json::to_string(&value).unwrap();
            println!("{json}");
        } else {
            eprintln!(
                "Warning: showing {shown_match_count} out of {total_match_count} results. Provide a more specific pattern or increase the --limit.",
            );
        }
    }

    fn warn_no_search_pattern(&self, search_directory: &Path) {
        eprintln!(
            "No search pattern specified. Showing the contents of the current directory ({}):",
            search_directory.to_string_lossy()
        );
    }
}
