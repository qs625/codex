use super::*;
use pretty_assertions::assert_eq;

#[test]
fn serialize_multiagent_context_without_direct_subagents() {
    let context = MultiagentContext::new(AgentPath::root(), Vec::new());

    assert_eq!(
        context.render(),
        r#"<multiagent_context>
  <current_thread_canonical_path>/root</current_thread_canonical_path>
</multiagent_context>"#
    );
}

#[test]
fn serialize_multiagent_context_with_direct_subagents() {
    let context = MultiagentContext::new(
        AgentPath::root()
            .join("worker")
            .expect("worker path should be valid"),
        vec![
            AgentPath::from_string("/root/worker/researcher".to_string())
                .expect("researcher path should be valid"),
            AgentPath::from_string("/root/worker/tester".to_string())
                .expect("tester path should be valid"),
        ],
    );

    assert_eq!(
        context.render(),
        r#"<multiagent_context>
  <current_thread_canonical_path>/root/worker</current_thread_canonical_path>
  <direct_subagents>
    <canonical_path>/root/worker/researcher</canonical_path>
    <canonical_path>/root/worker/tester</canonical_path>
  </direct_subagents>
</multiagent_context>"#
    );
}
