use std::collections::HashMap;
use std::collections::HashSet;

use protocol::models::SearchToolCallParams;

use crate::FunctionCallError;
use crate::LoadableToolSpec;
use crate::TOOL_SEARCH_DEFAULT_LIMIT;
use crate::ToolSearchEntry;
use crate::ToolSearchInfo;
use crate::ToolSearchOutput;
use crate::coalesce_loadable_tool_specs;

/// Host-neutral runtime for deferred tool search.
///
/// The executable host owns tool invocation and model-visible error delivery;
/// this type owns query validation, lightweight ranking, and loadable tool
/// result coalescing without depending on `codex-core`.
pub struct ToolSearchRuntime {
    entries: Vec<ToolSearchEntry>,
    search_index: ToolSearchIndex,
}

impl ToolSearchRuntime {
    pub fn new(search_infos: Vec<ToolSearchInfo>) -> Self {
        let mut entries = Vec::with_capacity(search_infos.len());
        for search_info in search_infos {
            entries.push(search_info.entry);
        }
        let search_index =
            ToolSearchIndex::new(entries.iter().map(|entry| entry.search_text.as_str()));

        Self {
            entries,
            search_index,
        }
    }

    pub fn handle_search(
        &self,
        arguments: SearchToolCallParams,
    ) -> Result<ToolSearchOutput, FunctionCallError> {
        let query = arguments.query.trim();
        if query.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "query must not be empty".to_string(),
            ));
        }
        let limit = arguments.limit.unwrap_or(TOOL_SEARCH_DEFAULT_LIMIT);

        if limit == 0 {
            return Err(FunctionCallError::RespondToModel(
                "limit must be greater than zero".to_string(),
            ));
        }

        if self.entries.is_empty() {
            return Ok(ToolSearchOutput { tools: Vec::new() });
        }

        let tools = self.search(query, limit)?;

        Ok(ToolSearchOutput { tools })
    }

    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LoadableToolSpec>, FunctionCallError> {
        let results = self
            .search_index
            .search(query, limit)
            .into_iter()
            .filter_map(|id| self.entries.get(id));
        self.search_output_tools(results)
    }

    fn search_output_tools<'a>(
        &self,
        results: impl IntoIterator<Item = &'a ToolSearchEntry>,
    ) -> Result<Vec<LoadableToolSpec>, FunctionCallError> {
        Ok(coalesce_loadable_tool_specs(
            results.into_iter().map(|entry| entry.output.clone()),
        ))
    }
}

#[derive(Debug)]
struct ToolSearchIndex {
    documents: Vec<IndexedToolSearchDocument>,
    document_frequencies: HashMap<String, usize>,
    average_document_len: f64,
}

#[derive(Debug)]
struct IndexedToolSearchDocument {
    id: usize,
    term_frequencies: HashMap<String, usize>,
    len: usize,
}

impl ToolSearchIndex {
    fn new<'a>(documents: impl IntoIterator<Item = &'a str>) -> Self {
        let documents: Vec<IndexedToolSearchDocument> = documents
            .into_iter()
            .enumerate()
            .map(|(id, text)| {
                let tokens = tokenize_search_text(text);
                let mut term_frequencies = HashMap::new();
                for token in tokens {
                    *term_frequencies.entry(token).or_insert(0) += 1;
                }
                let len = term_frequencies.values().sum();
                IndexedToolSearchDocument {
                    id,
                    term_frequencies,
                    len,
                }
            })
            .collect();

        let mut document_frequencies = HashMap::new();
        for document in &documents {
            for term in document.term_frequencies.keys() {
                *document_frequencies.entry(term.clone()).or_insert(0) += 1;
            }
        }

        let total_document_len = documents.iter().map(|document| document.len).sum::<usize>();
        let average_document_len = if documents.is_empty() {
            0.0
        } else {
            total_document_len as f64 / documents.len() as f64
        };

        Self {
            documents,
            document_frequencies,
            average_document_len,
        }
    }

    fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        let query_terms = unique_search_terms(query);
        if query_terms.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f64)> = self
            .documents
            .iter()
            .filter_map(|document| {
                let score = self.score_document(document, &query_terms);
                (score > 0.0).then_some((document.id, score))
            })
            .collect();
        scored.sort_by(|(left_id, left_score), (right_id, right_score)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| left_id.cmp(right_id))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(document_id, _)| document_id)
            .collect()
    }

    fn score_document(&self, document: &IndexedToolSearchDocument, query_terms: &[String]) -> f64 {
        if self.documents.is_empty() || self.average_document_len == 0.0 {
            return 0.0;
        }

        const K1: f64 = 1.5;
        const B: f64 = 0.75;
        let document_count = self.documents.len() as f64;
        let len_normalizer = 1.0 - B + B * (document.len as f64 / self.average_document_len);

        query_terms
            .iter()
            .filter_map(|term| {
                let term_frequency = *document.term_frequencies.get(term)? as f64;
                let document_frequency = *self.document_frequencies.get(term)? as f64;
                let inverse_document_frequency = ((document_count - document_frequency + 0.5)
                    / (document_frequency + 0.5)
                    + 1.0)
                    .ln();
                let term_weight =
                    term_frequency * (K1 + 1.0) / (term_frequency + K1 * len_normalizer);
                Some(inverse_document_frequency * term_weight)
            })
            .sum()
    }
}

fn unique_search_terms(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    tokenize_search_text(text)
        .into_iter()
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

fn tokenize_search_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else {
            push_search_token(&mut tokens, &mut current);
        }
    }
    push_search_token(&mut tokens, &mut current);
    tokens
}

fn push_search_token(tokens: &mut Vec<String>, current: &mut String) {
    if current.is_empty() {
        return;
    }

    tokens.push(current.clone());
    if let Some(singular) = singularize_ascii_token(current)
        && singular != *current
    {
        tokens.push(singular);
    }
    current.clear();
}

fn singularize_ascii_token(token: &str) -> Option<String> {
    if token.len() > 3 && token.ends_with('s') && !token.ends_with("ss") {
        Some(token[..token.len() - 1].to_string())
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tool_search_runtime_tests.rs"]
mod tests;
