// SPDX-License-Identifier: Apache-2.0

//! Canonical public contract documents embedded for validation and generation.

/// `OpenAPI` 3.1 document for HTTP API version 1.
pub const OPENAPI_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/openapi/hyphae-v1.yaml"
));

/// JSON Schema for the version 1 capability response.
pub const CAPABILITIES_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/capabilities-v1.schema.json"
));

/// JSON Schema for the version 1 error envelope.
pub const ERROR_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/error-v1.schema.json"
));

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::Value as JsonValue;
    use serde_yaml_ng::Value as YamlValue;

    use super::{CAPABILITIES_SCHEMA_V1, ERROR_SCHEMA_V1, OPENAPI_V1};

    #[test]
    fn openapi_document_is_version_3_1() -> Result<(), Box<dyn Error>> {
        let document: YamlValue = serde_yaml_ng::from_str(OPENAPI_V1)?;
        assert_eq!(
            document.get("openapi").and_then(YamlValue::as_str),
            Some("3.1.0")
        );
        Ok(())
    }

    #[test]
    fn json_schemas_use_draft_2020_12() -> Result<(), Box<dyn Error>> {
        for schema in [CAPABILITIES_SCHEMA_V1, ERROR_SCHEMA_V1] {
            let document: JsonValue = serde_json::from_str(schema)?;
            assert_eq!(
                document.get("$schema").and_then(JsonValue::as_str),
                Some("https://json-schema.org/draft/2020-12/schema")
            );
        }
        Ok(())
    }
}
