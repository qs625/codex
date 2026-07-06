mod current_work;
mod replacement_history;
mod soft_compact;

use std::io;

use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::approx_token_count;
use codex_utils_output_truncation::truncate_text;
use compact_service_api::CompactMemoryBundle;
use compact_service_api::CompactMemoryRole;
use compact_service_api::CompactMemorySnapshot;
use compact_service_api::CompactReplacementFile;
use compact_service_api::CompactWindowSummary;
use compact_service_api::ReplacementHistoryInput;
use compact_service_api::SoftCompactDecision;
use compact_service_api::SoftCompactInputs;
use protocol::models::ResponseItem;

#[derive(Debug, Clone, Default)]
pub struct FsCompactService;

impl FsCompactService {
    pub fn new() -> Self {
        Self
    }

    pub async fn read_memory_bundle(
        &self,
        files: &[CompactReplacementFile],
    ) -> io::Result<CompactMemoryBundle> {
        let mut snapshots = Vec::new();
        for file in files {
            let Some(content) = read_optional_markdown(&file.path, file.token_limit).await? else {
                continue;
            };
            snapshots.push(CompactMemorySnapshot {
                role: file.role,
                label: snapshot_label(file),
                content,
            });
        }
        Ok(CompactMemoryBundle { snapshots })
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

    pub fn current_work_completeness(&self, bundle: &CompactMemoryBundle) -> f64 {
        current_work::current_work_completeness(bundle.current_work_content())
    }

    pub fn build_replacement_history(&self, input: ReplacementHistoryInput) -> Vec<ResponseItem> {
        replacement_history::build_replacement_history(input)
    }
}

async fn read_optional_markdown(path: &AbsolutePathBuf, token_limit: usize) -> io::Result<Option<String>> {
    match tokio::fs::read_to_string(path.as_path()).await {
        Ok(contents) => {
            let trimmed = truncate_memory_markdown(contents.trim(), token_limit);
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

fn truncate_memory_markdown(contents: &str, token_limit: usize) -> String {
    if approx_token_count(contents) <= token_limit {
        return contents.to_string();
    }
    truncate_text(contents, TruncationPolicy::Tokens(token_limit))
}

fn snapshot_label(file: &CompactReplacementFile) -> String {
    match file.role {
        CompactMemoryRole::CurrentWork => "current work".to_string(),
        CompactMemoryRole::ProjectUnderstanding => "project understanding".to_string(),
        CompactMemoryRole::UserPreferences => "user preferences".to_string(),
        CompactMemoryRole::Custom => file
            .label
            .clone()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| {
                file.path
                    .as_path()
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("custom memory")
                    .replace('-', " ")
            }),
    }
}

#[cfg(test)]
mod tests;
