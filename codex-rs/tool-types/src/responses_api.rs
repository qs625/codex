use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::JsonSchema;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FreeformTool {
    pub name: String,
    pub description: String,
    pub format: FreeformToolFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FreeformToolFormat {
    pub r#type: String,
    pub syntax: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResponsesApiTool {
    pub name: String,
    pub description: String,
    /// TODO: Validation. When strict is set to true, the JSON schema,
    /// `required` and `additional_properties` must be present. All fields in
    /// `properties` must be present in `required`.
    pub strict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_loading: Option<bool>,
    pub parameters: JsonSchema,
    #[serde(skip)]
    pub output_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum LoadableToolSpec {
    #[allow(dead_code)]
    #[serde(rename = "function")]
    Function(ResponsesApiTool),
    #[serde(rename = "namespace")]
    Namespace(ResponsesApiNamespace),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResponsesApiNamespace {
    pub name: String,
    pub description: String,
    pub tools: Vec<ResponsesApiNamespaceTool>,
}

pub fn default_namespace_description(namespace_name: &str) -> String {
    format!("Tools in the {namespace_name} namespace.")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum ResponsesApiNamespaceTool {
    #[serde(rename = "function")]
    Function(ResponsesApiTool),
}

pub fn coalesce_loadable_tool_specs(
    specs: impl IntoIterator<Item = LoadableToolSpec>,
) -> Vec<LoadableToolSpec> {
    let mut coalesced_specs = Vec::new();
    for spec in specs {
        match spec {
            LoadableToolSpec::Function(tool) => {
                coalesced_specs.push(LoadableToolSpec::Function(tool));
            }
            LoadableToolSpec::Namespace(mut namespace) => {
                if let Some(existing_namespace) =
                    coalesced_specs.iter_mut().find_map(|spec| match spec {
                        LoadableToolSpec::Namespace(existing_namespace)
                            if existing_namespace.name == namespace.name =>
                        {
                            Some(existing_namespace)
                        }
                        LoadableToolSpec::Function(_) | LoadableToolSpec::Namespace(_) => None,
                    })
                {
                    existing_namespace.tools.append(&mut namespace.tools);
                } else {
                    coalesced_specs.push(LoadableToolSpec::Namespace(namespace));
                }
            }
        }
    }
    coalesced_specs
}
