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
/// User-home instruction directory loaded from MORPHEUS_HOME.
pub const MORPHEUS_HOME_INSTRUCTIONS_DIR_NAME: &str = "instructions";

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
        let instruction_sources = self.visible_instruction_sources();
        let explicit_instruction_docs = self.read_instruction_files(fs, &instruction_sources).await;

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
        self.visible_instruction_sources()
    }

    fn visible_instruction_sources(&self) -> Vec<AbsolutePathBuf> {
        let mut sources = self.config.instruction_files.clone();
        let mut seen = sources
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();

        for path in self.morpheus_home_instruction_sources() {
            if seen.insert(path.clone()) {
                sources.push(path);
            }
        }

        for layer in self.config.config_layer_stack.get_layers(
            config_service::ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ true,
        ) {
            let codex_config_types::ConfigLayerSource::Project { dot_codex_folder } = &layer.name
            else {
                continue;
            };
            if layer.disabled_reason.is_none() {
                continue;
            }

            let Some(project_root) = dot_codex_folder.parent() else {
                continue;
            };
            let Ok(project_root) = std::fs::canonicalize(project_root) else {
                continue;
            };

            for path in disabled_layer_instruction_paths(&layer.config) {
                // Disabled project layers still surface UI metadata, but they do
                // not participate in effective config. Only recover repo-local
                // instruction files here so initial context can show checked-in
                // docs without allowing arbitrary out-of-repo file reads.
                let disabled_path =
                    resolve_project_relative_path(path.as_path(), project_root.as_path());
                let Some(canonical_path) =
                    canonical_repo_local_path(disabled_path.as_path(), project_root.as_path())
                else {
                    continue;
                };
                let Ok(canonical_path) = AbsolutePathBuf::try_from(canonical_path) else {
                    continue;
                };
                if seen.insert(canonical_path.clone()) {
                    sources.push(canonical_path);
                }
            }
        }

        sources
    }

    fn morpheus_home_instruction_sources(&self) -> Vec<AbsolutePathBuf> {
        let instructions_dir = self
            .config
            .codex_home
            .join(MORPHEUS_HOME_INSTRUCTIONS_DIR_NAME);
        let Ok(entries) = std::fs::read_dir(instructions_dir.as_path()) else {
            return Vec::new();
        };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            if file_name.is_empty() || file_name.starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            let Ok(path) = AbsolutePathBuf::try_from(path) else {
                continue;
            };
            files.push(path);
        }
        files.sort();
        files
    }

    async fn read_instruction_files(
        &self,
        fs: &dyn ExecutorFileSystem,
        instruction_files: &[AbsolutePathBuf],
    ) -> io::Result<Option<String>> {
        if instruction_files.is_empty() {
            return Ok(None);
        }

        let max_total = self.config.project_doc_max_bytes;
        if max_total == 0 {
            return Ok(None);
        }

        let mut remaining: u64 = max_total as u64;
        let mut parts = Vec::new();

        for path in instruction_files {
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

fn canonical_repo_local_path(
    path: &std::path::Path,
    canonical_project_root: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let canonical_path = std::fs::canonicalize(path).ok()?;
    canonical_path
        .starts_with(canonical_project_root)
        .then_some(canonical_path)
}

fn resolve_project_relative_path(
    path: &std::path::Path,
    project_root: &std::path::Path,
) -> std::path::PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn disabled_layer_instruction_paths(config: &toml::Value) -> Vec<std::path::PathBuf> {
    config
        .get("instruction_files")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(std::path::PathBuf::from)
        .collect()
}

#[cfg(test)]
#[path = "agents_md_tests.rs"]
mod tests;
