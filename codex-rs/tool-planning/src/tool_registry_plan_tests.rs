use std::sync::Arc;

use super::*;
use crate::GET_GOAL_TOOL_NAME;
use crate::ToolExposure;
use crate::ToolName;
use crate::create_get_goal_tool;

#[derive(Clone, Debug)]
struct TestEntry {
    name: ToolName,
    spec: Option<ToolSpec>,
    exposure: ToolExposure,
}

impl ToolRegistryEntry for TestEntry {
    fn tool_name(&self) -> ToolName {
        self.name.clone()
    }

    fn spec(&self) -> Option<ToolSpec> {
        self.spec.clone()
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn search_info(&self) -> Option<ToolSearchInfo> {
        None
    }
}

#[test]
fn direct_entries_add_specs_and_entries() {
    let mut builder = ToolRegistryPlanBuilder::new();
    let name = ToolName::plain(GET_GOAL_TOOL_NAME);
    builder
        .register_tool(TestEntry {
            name: name.clone(),
            spec: Some(create_get_goal_tool()),
            exposure: ToolExposure::Direct,
        })
        .expect("entry should register");

    let plan = builder.build();

    assert_eq!(plan.specs, vec![create_get_goal_tool()]);
    assert_eq!(plan.entries.len(), 1);
    assert!(plan.entries.contains_key(&name));
}

#[test]
fn register_without_spec_keeps_entry_hidden_from_model_specs() {
    let mut builder = ToolRegistryPlanBuilder::new();
    let name = ToolName::plain(GET_GOAL_TOOL_NAME);
    builder
        .register_tool_without_spec(TestEntry {
            name: name.clone(),
            spec: Some(create_get_goal_tool()),
            exposure: ToolExposure::Direct,
        })
        .expect("entry should register");

    let plan = builder.build();

    assert_eq!(plan.specs, Vec::<ToolSpec>::new());
    assert!(plan.entries.contains_key(&name));
}

#[test]
fn duplicate_entries_return_duplicate_name() {
    let mut builder = ToolRegistryPlanBuilder::new();
    let name = ToolName::plain(GET_GOAL_TOOL_NAME);
    let entry = TestEntry {
        name: name.clone(),
        spec: None,
        exposure: ToolExposure::Direct,
    };
    builder
        .register_tool(entry.clone())
        .expect("first entry should register");

    let err = builder
        .register_tool(entry)
        .expect_err("duplicate should be rejected");

    assert_eq!(err, DuplicateToolName { name });
}

#[test]
fn arc_entries_delegate_registry_metadata() {
    let mut builder = ToolRegistryPlanBuilder::new();
    let name = ToolName::plain(GET_GOAL_TOOL_NAME);
    let entry = Arc::new(TestEntry {
        name: name.clone(),
        spec: Some(create_get_goal_tool()),
        exposure: ToolExposure::Direct,
    });

    builder
        .register_tool(entry)
        .expect("arc entry should register");

    let plan = builder.build();
    assert_eq!(plan.specs, vec![create_get_goal_tool()]);
    assert!(plan.entries.contains_key(&name));
}
