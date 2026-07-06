mod current_work;
mod prompt;
mod replacement_history;
mod soft_compact;

use std::io;

use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::approx_token_count;
use codex_utils_output_truncation::truncate_text;
use compact_service_api::AppliedCompactMemory;
use compact_service_api::CompactMemoryBundle;
use compact_service_api::CompactMemoryLayout;
use compact_service_api::CompactModelOutput;
use compact_service_api::CompactPromptSpec;
use compact_service_api::CompactWindowSummary;
use compact_service_api::CompactWritePolicy;
use compact_service_api::ReplacementHistoryInput;
use compact_service_api::SoftCompactDecision;
use compact_service_api::SoftCompactInputs;
use protocol::models::ResponseItem;

pub use compact_service_api::CompactCurrentWork;
pub use compact_service_api::CompactFileNote;

const USER_PREFERENCES_FILENAME: &str = "user-preferences.md";
const PROJECT_UNDERSTANDING_FILENAME: &str = "project-understanding.md";
const CURRENT_WORK_FILENAME: &str = "current-work.md";
const MEMORY_FILE_TOKEN_LIMIT: usize = 1_500;

#[derive(Debug, Clone, Default)]
pub struct FsCompactService;

impl FsCompactService {
    pub fn new() -> Self {
        Self
    }

    pub async fn derive_memory_layout(
        &self,
        cwd: &AbsolutePathBuf,
        codex_home: &AbsolutePathBuf,
        compact_prompt: Option<&str>,
    ) -> io::Result<CompactMemoryLayout> {
        let worktree_memory_root = cwd.join(".codex").join("memory");
        let shared_memory_root =
            derive_shared_memory_root(cwd, codex_home, compact_prompt).await?;
        Ok(CompactMemoryLayout {
            shared_memory_root,
            worktree_memory_root,
            write_policy: CompactWritePolicy::LocalCurrentWorkOnly,
        })
    }

    pub async fn read_memory_bundle(
        &self,
        layout: &CompactMemoryLayout,
    ) -> io::Result<CompactMemoryBundle> {
        let user_preferences = if let Some(root) = &layout.shared_memory_root {
            read_optional_markdown(&root.join(USER_PREFERENCES_FILENAME)).await?
        } else {
            None
        };
        let project_understanding = if let Some(root) = &layout.shared_memory_root {
            read_optional_markdown(&root.join(PROJECT_UNDERSTANDING_FILENAME)).await?
        } else {
            None
        };
        let current_work =
            read_optional_markdown(&layout.worktree_memory_root.join(CURRENT_WORK_FILENAME)).await?;
        Ok(CompactMemoryBundle {
            user_preferences,
            project_understanding,
            current_work,
        })
    }

    pub fn build_prompt_spec(
        &self,
        compact_prompt: &str,
        bundle: &CompactMemoryBundle,
    ) -> CompactPromptSpec {
        prompt::build_prompt_spec(compact_prompt, bundle)
    }

    pub fn parse_model_output(&self, text: &str) -> serde_json::Result<CompactModelOutput> {
        serde_json::from_str(text)
    }

    pub async fn apply_model_output(
        &self,
        layout: &CompactMemoryLayout,
        bundle: &CompactMemoryBundle,
        output: &CompactModelOutput,
    ) -> io::Result<AppliedCompactMemory> {
        let current_work_markdown = current_work::render_current_work(output);
        tokio::fs::create_dir_all(layout.worktree_memory_root.as_path()).await?;
        tokio::fs::write(
            layout.worktree_memory_root.join(CURRENT_WORK_FILENAME).as_path(),
            &current_work_markdown,
        )
        .await?;
        Ok(AppliedCompactMemory {
            bundle: CompactMemoryBundle {
                user_preferences: bundle.user_preferences.clone(),
                project_understanding: bundle.project_understanding.clone(),
                current_work: Some(current_work_markdown.clone()),
            },
            current_work_markdown,
        })
    }

    pub fn summarize_compact_window(
        &self,
        items: &[ResponseItem],
        summary_prefix: &str,
    ) -> CompactWindowSummary {
        soft_compact::summarize_compact_window(items, summary_prefix)
    }

    pub fn evaluate_soft_compact(&self, inputs: SoftCompactInputs) -> SoftCompactDecision {
        soft_compact::evaluate_soft_compact(inputs)
    }

    pub fn current_work_completeness(&self, current_work: Option<&str>) -> f64 {
        current_work::current_work_completeness(current_work)
    }

    pub fn build_replacement_history(&self, input: ReplacementHistoryInput) -> Vec<ResponseItem> {
        replacement_history::build_replacement_history(input)
    }
}

async fn derive_shared_memory_root(
    cwd: &AbsolutePathBuf,
    codex_home: &AbsolutePathBuf,
    compact_prompt: Option<&str>,
) -> io::Result<Option<AbsolutePathBuf>> {
    let Some(compact_prompt) = compact_prompt else {
        return Ok(None);
    };
    let workspace_prompt_path = cwd.join(".codex").join("compact").join("COMPACT.md");
    if prompt_matches_file(&workspace_prompt_path, compact_prompt).await? {
        return Ok(Some(cwd.join(".codex").join("memory")));
    }
    let home_prompt_path = codex_home.join("compact").join("COMPACT.md");
    if prompt_matches_file(&home_prompt_path, compact_prompt).await? {
        return Ok(None);
    }
    Ok(None)
}

async fn prompt_matches_file(path: &AbsolutePathBuf, compact_prompt: &str) -> io::Result<bool> {
    match tokio::fs::read_to_string(path.as_path()).await {
        Ok(contents) => Ok(contents.trim() == compact_prompt.trim()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

async fn read_optional_markdown(path: &AbsolutePathBuf) -> io::Result<Option<String>> {
    match tokio::fs::read_to_string(path.as_path()).await {
        Ok(contents) => {
            let trimmed = truncate_memory_markdown(contents.trim());
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn truncate_memory_markdown(contents: &str) -> String {
    if approx_token_count(contents) <= MEMORY_FILE_TOKEN_LIMIT {
        return contents.to_string();
    }
    truncate_text(contents, TruncationPolicy::Tokens(MEMORY_FILE_TOKEN_LIMIT))
}

#[cfg(test)]
mod tests;
