use crate::ToolArgumentDiffConsumer;
use crate::ToolRegistryView;
use codex_code_mode_api::is_code_mode_nested_tool;
use codex_protocol::models::ResponseItem;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolExposure;
use codex_tool_types::ToolName;
use codex_tool_types::ToolSpec;
use std::marker::PhantomData;

pub struct ToolRouter<Registry, DiffContext> {
    registry: Registry,
    model_visible_specs: Vec<ToolSpec>,
    _marker: PhantomData<fn(DiffContext)>,
}

impl<Registry, DiffContext> ToolRouter<Registry, DiffContext>
where
    Registry: ToolRegistryView<DiffContext>,
{
    pub fn new(code_mode_only_enabled: bool, specs: Vec<ToolSpec>, registry: Registry) -> Self {
        let model_visible_specs = specs
            .into_iter()
            .filter(|spec| !is_hidden_by_code_mode_only(code_mode_only_enabled, &registry, spec))
            .collect();

        Self {
            registry,
            model_visible_specs,
            _marker: PhantomData,
        }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn model_visible_specs(&self) -> Vec<ToolSpec> {
        self.model_visible_specs.clone()
    }

    pub fn create_diff_consumer(
        &self,
        tool_name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>> {
        self.registry.create_diff_consumer(tool_name)
    }

    pub fn tool_supports_parallel(&self, call: &ToolCall) -> bool {
        self.registry
            .supports_parallel_tool_calls(&call.tool_name)
            .unwrap_or(false)
    }

    pub fn build_tool_call(item: ResponseItem) -> Result<Option<ToolCall>, FunctionCallError> {
        ToolCall::from_response_item(item)
    }
}

fn is_hidden_by_code_mode_only<Registry, DiffContext>(
    code_mode_only_enabled: bool,
    registry: &Registry,
    spec: &ToolSpec,
) -> bool
where
    Registry: ToolRegistryView<DiffContext>,
{
    if !code_mode_only_enabled || !is_code_mode_nested_tool(spec.name()) {
        return false;
    }

    let exposure = registry
        .tool_exposure(&ToolName::plain(spec.name()))
        .unwrap_or(ToolExposure::Direct);
    exposure != ToolExposure::DirectModelOnly
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolArgumentDiffConsumer;
    use crate::ToolRegistryView;
    use codex_tool_types::FreeformTool;
    use codex_tool_types::ToolSpec;
    use pretty_assertions::assert_eq;

    struct TestRegistry {
        exposure: Option<ToolExposure>,
    }

    impl ToolRegistryView<()> for TestRegistry {
        fn tool_exposure(&self, _name: &ToolName) -> Option<ToolExposure> {
            self.exposure
        }

        fn create_diff_consumer(
            &self,
            _name: &ToolName,
        ) -> Option<Box<dyn ToolArgumentDiffConsumer<()>>> {
            None
        }

        fn supports_parallel_tool_calls(&self, _name: &ToolName) -> Option<bool> {
            None
        }
    }

    fn freeform_spec(name: &str) -> ToolSpec {
        ToolSpec::Freeform(FreeformTool {
            name: name.to_string(),
            description: "test".to_string(),
            format: codex_tool_types::FreeformToolFormat {
                r#type: "text".to_string(),
                syntax: "text".to_string(),
                definition: "text".to_string(),
            },
        })
    }

    #[test]
    fn code_mode_only_hides_nested_non_direct_model_tools() {
        let router = ToolRouter::new(
            true,
            vec![freeform_spec("exec"), freeform_spec("nested_tool")],
            TestRegistry {
                exposure: Some(ToolExposure::Direct),
            },
        );

        let names = router
            .model_visible_specs()
            .into_iter()
            .map(|spec| spec.name().to_string())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["exec"]);
    }

    #[test]
    fn code_mode_only_keeps_direct_model_only_nested_tools() {
        let router = ToolRouter::new(
            true,
            vec![freeform_spec("nested_tool")],
            TestRegistry {
                exposure: Some(ToolExposure::DirectModelOnly),
            },
        );

        let names = router
            .model_visible_specs()
            .into_iter()
            .map(|spec| spec.name().to_string())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["nested_tool"]);
    }
}
