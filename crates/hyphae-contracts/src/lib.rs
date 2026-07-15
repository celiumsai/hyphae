// SPDX-License-Identifier: Apache-2.0

//! Canonical public contract documents embedded for validation and generation.

pub mod v1;

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

/// JSON Schema for version 1 liveness and readiness responses.
pub const HEALTH_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/health-v1.schema.json"
));

/// JSON Schema for version 1 atomic put requests.
pub const PUT_REQUEST_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/put-request-v1.schema.json"
));

/// JSON Schema for version 1 atomic delete requests.
pub const DELETE_REQUEST_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/delete-request-v1.schema.json"
));

/// JSON Schema for version 1 exact-get requests.
pub const GET_REQUEST_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/get-request-v1.schema.json"
));

/// JSON Schema for version 1 proof-bearing get responses.
pub const GET_RESPONSE_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/get-response-v1.schema.json"
));

/// JSON Schema for version 1 durable commit receipts.
pub const COMMIT_RECEIPT_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/commit-receipt-v1.schema.json"
));

/// JSON Schema for version 1 structured query requests.
pub const QUERY_REQUEST_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/query-request-v1.schema.json"
));

/// JSON Schema for version 1 proof-bearing query responses.
pub const QUERY_RESPONSE_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/query-response-v1.schema.json"
));

/// JSON Schema for version 1 result-proof transport.
pub const PROOF_SCHEMA_V1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/json-schema/proof-v1.schema.json"
));

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, io, path::Path};

    use serde_json::Value as JsonValue;
    use serde_yaml_ng::Value as YamlValue;

    use schemars::{JsonSchema, SchemaGenerator};

    use super::{
        CAPABILITIES_SCHEMA_V1, COMMIT_RECEIPT_SCHEMA_V1, DELETE_REQUEST_SCHEMA_V1,
        ERROR_SCHEMA_V1, GET_REQUEST_SCHEMA_V1, GET_RESPONSE_SCHEMA_V1, HEALTH_SCHEMA_V1,
        OPENAPI_V1, PROOF_SCHEMA_V1, PUT_REQUEST_SCHEMA_V1, QUERY_REQUEST_SCHEMA_V1,
        QUERY_RESPONSE_SCHEMA_V1,
        v1::{
            CapabilitiesV1, CommitReceiptV1, DeleteRequestV1, ErrorV1, GetRequestV1, GetResponseV1,
            HealthV1, ProofV1, PutRequestV1, QueryRequestV1, QueryResponseV1,
        },
    };

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
    fn openapi_defines_phase_five_surface_and_resolves_external_schemas()
    -> Result<(), Box<dyn Error>> {
        let document: YamlValue = serde_yaml_ng::from_str(OPENAPI_V1)?;
        let paths = document
            .get("paths")
            .and_then(YamlValue::as_mapping)
            .ok_or_else(|| io::Error::other("OpenAPI paths object is missing"))?;
        for (path, method) in [
            ("/v1/capabilities", "get"),
            ("/v1/health/live", "get"),
            ("/v1/health/ready", "get"),
            ("/v1/kv/put", "post"),
            ("/v1/kv/get", "post"),
            ("/v1/kv/delete", "post"),
            ("/v1/query", "post"),
            (
                "/v1/witnesses/{checkpoint_sequence}/{snapshot_digest}",
                "get",
            ),
        ] {
            let operation = paths
                .get(YamlValue::String(path.to_owned()))
                .and_then(|item| item.get(method));
            assert!(operation.is_some(), "missing {method} {path}");
        }

        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/openapi");
        validate_external_schema_refs(&document, &base)?;
        Ok(())
    }

    #[test]
    fn json_schemas_use_draft_2020_12() -> Result<(), Box<dyn Error>> {
        for schema in [
            CAPABILITIES_SCHEMA_V1,
            ERROR_SCHEMA_V1,
            HEALTH_SCHEMA_V1,
            PUT_REQUEST_SCHEMA_V1,
            DELETE_REQUEST_SCHEMA_V1,
            GET_REQUEST_SCHEMA_V1,
            GET_RESPONSE_SCHEMA_V1,
            COMMIT_RECEIPT_SCHEMA_V1,
            QUERY_REQUEST_SCHEMA_V1,
            QUERY_RESPONSE_SCHEMA_V1,
            PROOF_SCHEMA_V1,
        ] {
            let document: JsonValue = serde_json::from_str(schema)?;
            assert_eq!(
                document.get("$schema").and_then(JsonValue::as_str),
                Some("https://json-schema.org/draft/2020-12/schema")
            );
        }
        Ok(())
    }

    #[test]
    fn checked_in_json_schemas_match_rust_wire_models() -> Result<(), Box<dyn Error>> {
        assert_schema::<CapabilitiesV1>(CAPABILITIES_SCHEMA_V1)?;
        assert_schema::<ErrorV1>(ERROR_SCHEMA_V1)?;
        assert_schema::<HealthV1>(HEALTH_SCHEMA_V1)?;
        assert_schema::<PutRequestV1>(PUT_REQUEST_SCHEMA_V1)?;
        assert_schema::<DeleteRequestV1>(DELETE_REQUEST_SCHEMA_V1)?;
        assert_schema::<GetRequestV1>(GET_REQUEST_SCHEMA_V1)?;
        assert_schema::<GetResponseV1>(GET_RESPONSE_SCHEMA_V1)?;
        assert_schema::<CommitReceiptV1>(COMMIT_RECEIPT_SCHEMA_V1)?;
        assert_schema::<QueryRequestV1>(QUERY_REQUEST_SCHEMA_V1)?;
        assert_schema::<QueryResponseV1>(QUERY_RESPONSE_SCHEMA_V1)?;
        assert_schema::<ProofV1>(PROOF_SCHEMA_V1)?;
        Ok(())
    }

    fn assert_schema<T: JsonSchema>(checked_in: &str) -> Result<(), Box<dyn Error>> {
        let generated = SchemaGenerator::default().into_root_schema_for::<T>();
        let checked_in: JsonValue = serde_json::from_str(checked_in)?;
        assert_eq!(serde_json::to_value(generated)?, checked_in);
        Ok(())
    }

    fn validate_external_schema_refs(value: &YamlValue, base: &Path) -> Result<(), Box<dyn Error>> {
        match value {
            YamlValue::Mapping(mapping) => {
                if let Some(reference) = mapping
                    .get(YamlValue::String("$ref".to_owned()))
                    .and_then(YamlValue::as_str)
                    .filter(|reference| reference.starts_with("../json-schema/"))
                {
                    let encoded = fs::read_to_string(base.join(reference))?;
                    let _schema: JsonValue = serde_json::from_str(&encoded)?;
                }
                for (key, value) in mapping {
                    validate_external_schema_refs(key, base)?;
                    validate_external_schema_refs(value, base)?;
                }
            }
            YamlValue::Sequence(values) => {
                for value in values {
                    validate_external_schema_refs(value, base)?;
                }
            }
            YamlValue::Tagged(value) => validate_external_schema_refs(&value.value, base)?,
            YamlValue::Null | YamlValue::Bool(_) | YamlValue::Number(_) | YamlValue::String(_) => {}
        }
        Ok(())
    }
}
