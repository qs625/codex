//! User instruction assembly for model-visible project context.

use crate::config::Config;
use codex_features::Feature;
use codex_file_system::ExecutorFileSystem;
use codex_utils_absolute_path::AbsolutePathBuf;
#[cfg(test)]
use exec_server_api::ExecEnvironment;
use std::io;
use tracing::error;

pub(crate) const HIERARCHICAL_AGENTS_MESSAGE: &str =
    include_str!("../hierarchical_agents_message.md");

/// Default filename scanned for AGENTS.md instructions.
pub const DEFAULT_AGENTS_MD_FILENAME: &str = "AGENTS.md";
/// Preferred local override for AGENTS.md instructions.
pub const LOCAL_AGENTS_MD_FILENAME: &str = "AGENTS.override.md";

/// When both `Config::instructions` and AGENTS.md docs are present, they will
/// be concatenated with the following separator.
const AGENTS_MD_SEPARATOR: &str = "\n\n--- project-doc ---\n\n";

/// Resolves configured instruction files into model-visible user instructions
/// and source paths.
pub struct AgentsMdManager<'a> {
    config: &'a Config,
}

impl<'a> AgentsMdManager<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    /// Test-only wrapper that preserves the historical "no environment"
    /// boundary for callers that intentionally model its absence.
    #[cfg(test)]
    pub(crate) async fn user_instructions(
        &self,
        environment: Option<&dyn ExecEnvironment>,
    ) -> Option<String> {
        let fs = environment?.get_filesystem();
        self.user_instructions_with_fs(fs.as_ref()).await
    }

    pub(crate) async fn user_instructions_with_fs(
        &self,
        fs: &dyn ExecutorFileSystem,
    ) -> Option<String> {
        let explicit_instruction_docs = self.read_instruction_files(fs).await;

        let mut output = String::new();

        if let Some(instructions) = self.config.user_instructions.clone() {
            output.push_str(&instructions);
        }

        match explicit_instruction_docs {
            Ok(Some(docs)) => {
                if !output.is_empty() {
                    output.push_str(AGENTS_MD_SEPARATOR);
                }
                output.push_str(&docs);
            }
            Ok(None) => {}
            Err(e) => {
                error!("error trying to load configured instruction files: {e:#}");
            }
        };

        if self.config.features.enabled(Feature::ChildAgentsMd) {
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str(HIERARCHICAL_AGENTS_MESSAGE);
        }

        if !output.is_empty() {
            Some(output)
        } else {
            None
        }
    }

    /// Returns all explicit instruction source files included in the current
    /// config.
    pub async fn instruction_sources(&self, _fs: &dyn ExecutorFileSystem) -> Vec<AbsolutePathBuf> {
        self.config.instruction_files.clone()
    }

    async fn read_instruction_files(
        &self,
        fs: &dyn ExecutorFileSystem,
    ) -> io::Result<Option<String>> {
        if self.config.instruction_files.is_empty() {
            return Ok(None);
        }

        let max_total = self.config.project_doc_max_bytes;
        if max_total == 0 {
            return Ok(None);
        }

        let mut remaining: u64 = max_total as u64;
        let mut parts = Vec::new();

        for path in &self.config.instruction_files {
            if remaining == 0 {
                break;
            }

            match fs.get_metadata(path, /*sandbox*/ None).await {
                Ok(metadata) if !metadata.is_file => continue,
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            }

            let mut data = match fs.read_file(path, /*sandbox*/ None).await {
                Ok(data) => data,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            let size = data.len() as u64;
            if size > remaining {
                data.truncate(remaining as usize);
            }

            if size > remaining {
                tracing::warn!(
                    "Instruction file `{}` exceeds remaining budget ({} bytes) - truncating.",
                    path.display(),
                    remaining,
                );
            }

            let text = String::from_utf8_lossy(&data).to_string();
            if !text.trim().is_empty() {
                parts.push(text);
                remaining = remaining.saturating_sub(data.len() as u64);
            }
        }

        if parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(parts.join("\n\n")))
        }
    }
}

#[cfg(test)]
#[path = "agents_md_tests.rs"]
mod tests;
