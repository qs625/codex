use std::collections::HashMap;
use std::sync::Arc;

use crate::ToolName;
use crate::ToolSearchInfo;
use crate::ToolSpec;
use codex_tool_types::ToolExposure;

/// Host-neutral registry entry metadata used while planning tool exposure.
///
/// Implementations should only report stable planning data: the canonical tool
/// name, optional model-visible spec, exposure mode, and optional deferred
/// search metadata. Runtime execution, hooks, telemetry, and turn/session
/// state stay in the host crate that owns the concrete tool runtime.
pub trait ToolRegistryEntry {
    fn tool_name(&self) -> ToolName;

    fn spec(&self) -> Option<ToolSpec>;

    fn exposure(&self) -> ToolExposure;

    fn search_info(&self) -> Option<ToolSearchInfo>;
}

impl<T> ToolRegistryEntry for Arc<T>
where
    T: ToolRegistryEntry + ?Sized,
{
    fn tool_name(&self) -> ToolName {
        self.as_ref().tool_name()
    }

    fn spec(&self) -> Option<ToolSpec> {
        self.as_ref().spec()
    }

    fn exposure(&self) -> ToolExposure {
        self.as_ref().exposure()
    }

    fn search_info(&self) -> Option<ToolSearchInfo> {
        self.as_ref().search_info()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateToolName {
    pub name: ToolName,
}

pub struct ToolRegistryPlan<E> {
    pub specs: Vec<ToolSpec>,
    pub entries: HashMap<ToolName, E>,
}

pub struct ToolRegistryPlanBuilder<E> {
    entries: HashMap<ToolName, E>,
    specs: Vec<ToolSpec>,
}

impl<E> ToolRegistryPlanBuilder<E>
where
    E: ToolRegistryEntry,
{
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            specs: Vec::new(),
        }
    }

    pub fn push_spec(&mut self, spec: ToolSpec) {
        self.specs.push(spec);
    }

    pub fn register_tool(&mut self, entry: E) -> Result<(), DuplicateToolName> {
        self.register_tool_internal(entry, /*include_spec*/ true)
    }

    pub fn register_tool_without_spec(&mut self, entry: E) -> Result<(), DuplicateToolName> {
        self.register_tool_internal(entry, /*include_spec*/ false)
    }

    fn register_tool_internal(
        &mut self,
        entry: E,
        include_spec: bool,
    ) -> Result<(), DuplicateToolName> {
        let name = entry.tool_name();
        if self.entries.contains_key(&name) {
            return Err(DuplicateToolName { name });
        }

        if include_spec
            && entry.exposure().is_direct()
            && let Some(spec) = entry.spec()
        {
            self.push_spec(spec);
        }

        self.entries.insert(name, entry);
        Ok(())
    }

    pub fn build(self) -> ToolRegistryPlan<E> {
        ToolRegistryPlan {
            specs: self.specs,
            entries: self.entries,
        }
    }
}

#[cfg(test)]
#[path = "tool_registry_plan_tests.rs"]
mod tests;
