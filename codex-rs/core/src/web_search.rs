use codex_protocol::models::WebSearchAction;

pub fn web_search_action_detail(action: &WebSearchAction) -> String {
    codex_turn_items::web_search_action_detail(action)
}

pub fn web_search_detail(action: Option<&WebSearchAction>, query: &str) -> String {
    let detail = action.map(web_search_action_detail).unwrap_or_default();
    if detail.is_empty() {
        query.to_string()
    } else {
        detail
    }
}
