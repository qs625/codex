use crate::config_toml::ConfigToml;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use std::io;
use std::path::Path;
use toml::Value as TomlValue;

/// Deserialize a `ConfigToml` while resolving relative path fields against
/// `config_base_dir`.
pub fn deserialize_config_toml_with_base(
    root_value: TomlValue,
    config_base_dir: &Path,
) -> io::Result<ConfigToml> {
    let guard = AbsolutePathBufGuard::new(config_base_dir);
    let config = root_value
        .try_into()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
    drop(guard);
    config
}

/// Resolve relative path fields in a `config.toml` value against `base_dir`.
///
/// Unknown fields are preserved, so callers can safely run this before merging
/// config layers that were loaded from different directories.
pub fn resolve_relative_paths_in_config_toml(
    value_from_config_toml: TomlValue,
    base_dir: &Path,
) -> io::Result<TomlValue> {
    // Use the serialize/deserialize round-trip to convert the TOML value into
    // ConfigToml while AbsolutePathBufGuard provides the base directory for
    // relative path fields.
    let guard = AbsolutePathBufGuard::new(base_dir);
    let Ok(resolved) = value_from_config_toml.clone().try_into::<ConfigToml>() else {
        return Ok(value_from_config_toml);
    };
    drop(guard);

    let resolved_value = TomlValue::try_from(resolved).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize resolved config: {e}"),
        )
    })?;

    Ok(copy_shape_from_original(
        &value_from_config_toml,
        &resolved_value,
    ))
}

/// Ensure that every field in `original` is present in the returned
/// `toml::Value`, taking the value from `resolved` where possible. This ensures
/// the fields that were removed during the serialize/deserialize round-trip are
/// preserved.
fn copy_shape_from_original(original: &TomlValue, resolved: &TomlValue) -> TomlValue {
    match (original, resolved) {
        (TomlValue::Table(original_table), TomlValue::Table(resolved_table)) => {
            let mut table = toml::map::Map::new();
            for (key, original_value) in original_table {
                let resolved_value = resolved_table.get(key).unwrap_or(original_value);
                table.insert(
                    key.clone(),
                    copy_shape_from_original(original_value, resolved_value),
                );
            }
            TomlValue::Table(table)
        }
        (TomlValue::Array(original_array), TomlValue::Array(resolved_array)) => {
            let mut items = Vec::new();
            for (index, original_value) in original_array.iter().enumerate() {
                let resolved_value = resolved_array.get(index).unwrap_or(original_value);
                items.push(copy_shape_from_original(original_value, resolved_value));
            }
            TomlValue::Array(items)
        }
        (_, resolved_value) => resolved_value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn resolves_paths_and_preserves_unknown_fields() -> anyhow::Result<()> {
        let base_dir = std::env::current_dir()?;
        let input = TomlValue::Table(toml::toml! {
            preferred_auth_method = "chatgpt"
            model_instructions_file = "instructions.md"
            unknown_table = { nested = "value" }
        });

        let resolved = resolve_relative_paths_in_config_toml(input.clone(), base_dir.as_path())?;
        let expected_model_instructions = base_dir
            .join("instructions.md")
            .to_string_lossy()
            .to_string();

        assert_eq!(
            resolved
                .get("unknown_table")
                .and_then(TomlValue::as_table)
                .and_then(|table| table.get("nested"))
                .and_then(TomlValue::as_str),
            Some("value")
        );
        assert_eq!(
            resolved
                .get("preferred_auth_method")
                .and_then(TomlValue::as_str),
            Some("chatgpt")
        );
        assert_eq!(
            resolved
                .get("model_instructions_file")
                .and_then(TomlValue::as_str),
            Some(expected_model_instructions.as_str())
        );
        Ok(())
    }

    #[test]
    fn deserializes_config_toml_with_relative_path_base() -> anyhow::Result<()> {
        let base_dir = std::env::current_dir()?;
        let input = TomlValue::Table(toml::toml! {
            model_instructions_file = "instructions.md"
        });
        let expected_model_instructions = base_dir
            .join("instructions.md")
            .to_string_lossy()
            .to_string();

        let config = deserialize_config_toml_with_base(input, base_dir.as_path())?;

        assert_eq!(
            config
                .model_instructions_file
                .as_ref()
                .map(|path| path.as_path().to_string_lossy().to_string()),
            Some(expected_model_instructions)
        );
        Ok(())
    }
}
