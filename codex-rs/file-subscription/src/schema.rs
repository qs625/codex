use schemars::JsonSchema;
use serde_json::Map;
use serde_json::Value;

/// Returns the JSON Schema `properties`/`required`/`additionalProperties`
/// fragment used as function-call input parameters.
pub(crate) fn input_schema_for<T: JsonSchema>() -> Value {
    use schemars::r#gen::SchemaSettings;

    let schema = SchemaSettings::draft2019_09()
        .with(|settings| {
            settings.inline_subschemas = true;
            settings.option_add_null_type = false;
        })
        .into_generator()
        .into_root_schema_for::<T>();
    let schema_value = serde_json::to_value(schema)
        .unwrap_or_else(|err| panic!("generated tool schema should serialize: {err}"));
    let Value::Object(mut schema_object) = schema_value else {
        unreachable!("root tool schema must be an object");
    };
    let mut tool_schema = Map::new();
    for key in ["properties", "required", "additionalProperties"] {
        if let Some(value) = schema_object.remove(key) {
            tool_schema.insert(key.to_string(), value);
        }
    }
    Value::Object(tool_schema)
}
