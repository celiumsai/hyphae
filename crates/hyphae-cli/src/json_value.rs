// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use hyphae_query::Value;
use thiserror::Error;

/// Failure converting CLI JSON into deterministic structured values.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum JsonValueError {
    #[error("invalid JSON: {0}")]
    Syntax(String),

    #[error("structured query accepts only signed 64-bit integer JSON numbers")]
    NonIntegerNumber,
}

pub(crate) fn parse_json(input: &str) -> Result<Value, JsonValueError> {
    let value =
        serde_json::from_str(input).map_err(|source| JsonValueError::Syntax(source.to_string()))?;
    from_json(value)
}

pub(crate) fn from_json(value: serde_json::Value) -> Result<Value, JsonValueError> {
    match value {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(value) => Ok(Value::Boolean(value)),
        serde_json::Value::Number(value) => value
            .as_i64()
            .map(Value::Integer)
            .ok_or(JsonValueError::NonIntegerNumber),
        serde_json::Value::String(value) => Ok(Value::String(value)),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(from_json)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        serde_json::Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| Ok((key, from_json(value)?)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(Value::Object),
    }
}

pub(crate) fn to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Boolean(value) => serde_json::Value::Bool(*value),
        Value::Integer(value) => serde_json::Value::Number((*value).into()),
        Value::String(value) => serde_json::Value::String(value.clone()),
        Value::Bytes(value) => serde_json::json!({
            "$hyphae_bytes_hex": encode_hex(value),
        }),
        Value::Array(values) => serde_json::Value::Array(values.iter().map(to_json).collect()),
        Value::Object(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), to_json(value)))
                .collect(),
        ),
    }
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use hyphae_query::Value;

    use super::{JsonValueError, encode_hex, parse_json, to_json};

    #[test]
    fn json_round_trips_the_cli_subset() -> Result<(), JsonValueError> {
        let value = parse_json(r#"{"enabled":true,"items":[1,null,"x"]}"#)?;
        assert_eq!(parse_json(&to_json(&value).to_string())?, value);
        assert_eq!(encode_hex(&[0, 15, 16, 255]), "000f10ff");
        Ok(())
    }

    #[test]
    fn floating_point_json_is_rejected() {
        assert_eq!(parse_json("1.5"), Err(JsonValueError::NonIntegerNumber));
        assert_eq!(
            to_json(&Value::Bytes(vec![1, 2])),
            serde_json::json!({"$hyphae_bytes_hex": "0102"})
        );
    }
}
