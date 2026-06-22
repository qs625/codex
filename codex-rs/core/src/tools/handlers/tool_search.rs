use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::ToolSearchOutput;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;
use crate::tools::tool_search_entry::ToolSearchEntry;
use crate::tools::tool_search_entry::ToolSearchInfo;
use codex_tool_planning::LoadableToolSpec;
use codex_tool_planning::TOOL_SEARCH_DEFAULT_LIMIT;
use codex_tool_planning::TOOL_SEARCH_TOOL_NAME;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSearchSourceInfo;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::coalesce_loadable_tool_specs;
use codex_tool_planning::create_tool_search_tool;
use std::collections::HashMap;
use std::collections::HashSet;

pub struct ToolSearchHandler {
    entries: Vec<ToolSearchEntry>,
    search_source_infos: Vec<ToolSearchSourceInfo>,
    search_index: ToolSearchIndex,
}

impl ToolSearchHandler {
    pub(crate) fn new(search_infos: Vec<ToolSearchInfo>) -> Self {
        let mut entries = Vec::with_capacity(search_infos.len());
        let mut search_source_infos = Vec::new();
        for search_info in search_infos {
            entries.push(search_info.entry);
            if let Some(source_info) = search_info.source_info {
                search_source_infos.push(source_info);
            }
        }
        let search_index =
            ToolSearchIndex::new(entries.iter().map(|entry| entry.search_text.as_str()));

        Self {
            entries,
            search_source_infos,
            search_index,
        }
    }
}

impl ToolExecutor<ToolInvocation> for ToolSearchHandler {
    type Output = ToolSearchOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_SEARCH_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_tool_search_tool(
            &self.search_source_infos,
            TOOL_SEARCH_DEFAULT_LIMIT,
        ))
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation,
    ) -> crate::tools::registry::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation { payload, .. } = invocation;

            let args = match payload {
                ToolPayload::ToolSearch { arguments } => arguments,
                _ => {
                    return Err(FunctionCallError::Fatal(format!(
                        "{TOOL_SEARCH_TOOL_NAME} handler received unsupported payload"
                    )));
                }
            };

            let query = args.query.trim();
            if query.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "query must not be empty".to_string(),
                ));
            }
            let limit = args.limit.unwrap_or(TOOL_SEARCH_DEFAULT_LIMIT);

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
        })
    }
}

impl ToolHandler for ToolSearchHandler {}

impl ToolSearchHandler {
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
mod tests {
    use super::*;
    use crate::tools::handlers::DynamicToolHandler;
    use crate::tools::handlers::McpHandler;
    use codex_mcp_tool_types::ToolInfo;
    use codex_protocol::dynamic_tools::DynamicToolSpec;
    use codex_tool_planning::ResponsesApiNamespace;
    use codex_tool_planning::ResponsesApiNamespaceTool;
    use codex_tool_planning::ResponsesApiTool;
    use pretty_assertions::assert_eq;

    #[test]
    fn search_index_matches_underscore_terms_with_space_query() {
        let index = ToolSearchIndex::new([
            "name quasar_ping_beacon namespace orbit_ops",
            "name calendar_timezone_option_99 namespace calendar",
        ]);

        assert_eq!(index.search("quasar ping beacon", 1), vec![0]);
        assert_eq!(index.search("calendar_timezone_option_99", 1), vec![1]);
    }

    #[test]
    fn search_index_matches_description_and_schema_terms() {
        let index = ToolSearchIndex::new([
            "description Extract text from uploaded documents",
            "schema starts_at title",
            "description Delete archived records",
        ]);

        assert_eq!(index.search("uploaded document", 1), vec![0]);
        assert_eq!(index.search("starts_at", 1), vec![1]);
    }

    #[test]
    fn mixed_search_results_coalesce_mcp_namespaces() {
        let dynamic_tools = [DynamicToolSpec {
            namespace: Some("codex_app".to_string()),
            name: "automation_update".to_string(),
            description: "Create, update, view, or delete recurring automations.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string" },
                },
                "required": ["mode"],
                "additionalProperties": false,
            }),
            defer_loading: true,
        }];
        let mcp_tools = [
            tool_info("calendar", "create_event", "Create events"),
            tool_info("calendar", "list_events", "List events"),
        ];
        let mut search_infos = mcp_tools
            .iter()
            .map(|tool| {
                McpHandler::new(tool.clone())
                    .search_info()
                    .expect("MCP handler should return search info")
            })
            .collect::<Vec<_>>();
        search_infos.extend(dynamic_tools.iter().map(|tool| {
            DynamicToolHandler::new(tool)
                .expect("dynamic tool should convert")
                .search_info()
                .expect("dynamic handler should return search info")
        }));
        let handler = ToolSearchHandler::new(search_infos);
        let results = [
            &handler.entries[0],
            &handler.entries[2],
            &handler.entries[1],
        ];

        let tools = handler
            .search_output_tools(results)
            .expect("mixed search output should serialize");

        assert_eq!(
            tools,
            vec![
                LoadableToolSpec::Namespace(ResponsesApiNamespace {
                    name: "mcp__calendar__".to_string(),
                    description: "Tools in the mcp__calendar__ namespace.".to_string(),
                    tools: vec![
                        ResponsesApiNamespaceTool::Function(ResponsesApiTool {
                            name: "create_event".to_string(),
                            description: "Create events desktop tool".to_string(),
                            strict: false,
                            defer_loading: Some(true),
                            parameters: codex_tool_planning::JsonSchema::object(
                                Default::default(),
                                /*required*/ None,
                                Some(false.into()),
                            ),
                            output_schema: None,
                        }),
                        ResponsesApiNamespaceTool::Function(ResponsesApiTool {
                            name: "list_events".to_string(),
                            description: "List events desktop tool".to_string(),
                            strict: false,
                            defer_loading: Some(true),
                            parameters: codex_tool_planning::JsonSchema::object(
                                Default::default(),
                                /*required*/ None,
                                Some(false.into()),
                            ),
                            output_schema: None,
                        }),
                    ],
                }),
                LoadableToolSpec::Namespace(ResponsesApiNamespace {
                    name: "codex_app".to_string(),
                    description: "Tools in the codex_app namespace.".to_string(),
                    tools: vec![ResponsesApiNamespaceTool::Function(ResponsesApiTool {
                        name: "automation_update".to_string(),
                        description: "Create, update, view, or delete recurring automations."
                            .to_string(),
                        strict: false,
                        defer_loading: Some(true),
                        parameters: codex_tool_planning::JsonSchema::object(
                            std::collections::BTreeMap::from([(
                                "mode".to_string(),
                                codex_tool_planning::JsonSchema::string(/*description*/ None),
                            )]),
                            Some(vec!["mode".to_string()]),
                            Some(false.into()),
                        ),
                        output_schema: None,
                    })],
                }),
            ],
        );
    }

    fn tool_info(server_name: &str, tool_name: &str, description_prefix: &str) -> ToolInfo {
        ToolInfo {
            server_name: server_name.to_string(),
            supports_parallel_tool_calls: false,
            server_origin: None,
            callable_name: tool_name.to_string(),
            callable_namespace: format!("mcp__{server_name}__"),
            namespace_description: None,
            tool: codex_mcp_tool_types::McpTool {
                name: tool_name.to_string(),
                title: None,
                description: Some(format!("{description_prefix} desktop tool")),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false,
                }),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            connector_id: None,
            connector_name: None,
            plugin_display_names: Vec::new(),
        }
    }
}
