use std::collections::BTreeMap;

pub type EffortMap = BTreeMap<String, String>;

pub const WILDCARD: &str = "*";

pub fn parse_stored(raw: Option<&str>) -> Option<EffortMap> {
    let map: EffortMap = serde_json::from_str(raw?).ok()?;
    (!map.is_empty()).then_some(map)
}

pub fn validate(value: serde_json::Value) -> Result<Option<EffortMap>, String> {
    match value {
        serde_json::Value::Null => Ok(None),
        other => {
            let map: EffortMap = serde_json::from_value(other)
                .map_err(|_| "reasoning_effort_map must be an object of strings".to_string())?;
            Ok((!map.is_empty()).then_some(map))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stored_maps_round_trip_and_empty_is_none() {
        assert_eq!(parse_stored(Some(r#"{"max":"high"}"#)).unwrap()["max"], "high");
        assert!(parse_stored(Some("{}")).is_none());
        assert!(parse_stored(Some("nope")).is_none());
        assert!(parse_stored(None).is_none());
    }

    #[test]
    fn validation_rejects_non_string_values() {
        assert!(validate(json!({"max": 3})).is_err());
        assert!(validate(json!(["x"])).is_err());
        assert_eq!(validate(json!(null)), Ok(None));
        assert_eq!(validate(json!({})), Ok(None));
        assert!(validate(json!({"xhigh": "high"})).unwrap().is_some());
        assert_eq!(validate(json!({"*": ""})).unwrap().unwrap()["*"], "");
    }
}
