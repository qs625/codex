use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

pub fn deserialize_double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

pub fn serialize_double_option<T, S>(
    value: &Option<Option<T>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: Serializer,
{
    match value {
        Some(Some(value)) => value.serialize(serializer),
        Some(None) | None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde::Serialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq, Serialize)]
    struct Patch {
        #[serde(
            default,
            deserialize_with = "deserialize_double_option",
            serialize_with = "serialize_double_option",
            skip_serializing_if = "Option::is_none"
        )]
        value: Option<Option<String>>,
    }

    #[test]
    fn double_option_distinguishes_missing_null_and_value() {
        assert_eq!(
            serde_json::from_value::<Patch>(json!({})).expect("missing field"),
            Patch { value: None }
        );
        assert_eq!(
            serde_json::from_value::<Patch>(json!({ "value": null })).expect("null field"),
            Patch { value: Some(None) }
        );
        assert_eq!(
            serde_json::from_value::<Patch>(json!({ "value": "set" })).expect("set field"),
            Patch {
                value: Some(Some("set".to_string())),
            }
        );

        assert_eq!(
            serde_json::to_value(Patch { value: None }).expect("serialize missing"),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(Patch { value: Some(None) }).expect("serialize null"),
            json!({ "value": null })
        );
        assert_eq!(
            serde_json::to_value(Patch {
                value: Some(Some("set".to_string())),
            })
            .expect("serialize set"),
            json!({ "value": "set" })
        );
    }
}
