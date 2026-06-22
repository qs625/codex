use super::*;

#[test]
fn parse_memory_citation_extracts_entries_and_rollout_ids() {
    let citation = [
        "<citation_entries>",
        "memories/MEMORY.md:1-2|note=[Useful context]",
        "</citation_entries>",
        "<rollout_ids>",
        "00000000-0000-4000-8000-000000000001",
        "00000000-0000-4000-8000-000000000001",
        "</rollout_ids>",
    ]
    .join("\n");

    let parsed = parse_memory_citation(vec![citation]).expect("citation should parse");

    assert_eq!(
        parsed,
        MemoryCitation {
            entries: vec![MemoryCitationEntry {
                path: "memories/MEMORY.md".to_string(),
                line_start: 1,
                line_end: 2,
                note: "Useful context".to_string(),
            }],
            rollout_ids: vec!["00000000-0000-4000-8000-000000000001".to_string()],
        }
    );
}

#[test]
fn parse_memory_citation_supports_thread_ids_alias() {
    let parsed = parse_memory_citation(vec![
        "<thread_ids>\n00000000-0000-4000-8000-000000000002\n</thread_ids>".to_string(),
    ])
    .expect("thread id citation should parse");

    assert_eq!(
        thread_ids_from_memory_citation(&parsed),
        vec![ThreadId::try_from("00000000-0000-4000-8000-000000000002").unwrap()]
    );
}

#[test]
fn parse_memory_citation_returns_none_for_empty_or_malformed_content() {
    assert_eq!(parse_memory_citation(Vec::new()), None);
    assert_eq!(
        parse_memory_citation(vec!["<citation_entries>bad</citation_entries>".to_string()]),
        None
    );
}
