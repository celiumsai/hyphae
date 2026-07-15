// SPDX-License-Identifier: Apache-2.0

//! Regenerates checked-in JSON Schema 2020-12 public contract files.

use std::{error::Error, fs, path::Path};

use hyphae_contracts::v1::{
    CapabilitiesV1, CommitReceiptV1, DeleteRequestV1, ErrorV1, GetRequestV1, GetResponseV1,
    HealthV1, ProofV1, PutRequestV1, QueryRequestV1, QueryResponseV1,
};
use schemars::{JsonSchema, SchemaGenerator};

fn main() -> Result<(), Box<dyn Error>> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/json-schema");
    write::<CapabilitiesV1>(&directory, "capabilities-v1.schema.json")?;
    write::<ErrorV1>(&directory, "error-v1.schema.json")?;
    write::<HealthV1>(&directory, "health-v1.schema.json")?;
    write::<PutRequestV1>(&directory, "put-request-v1.schema.json")?;
    write::<DeleteRequestV1>(&directory, "delete-request-v1.schema.json")?;
    write::<GetRequestV1>(&directory, "get-request-v1.schema.json")?;
    write::<GetResponseV1>(&directory, "get-response-v1.schema.json")?;
    write::<CommitReceiptV1>(&directory, "commit-receipt-v1.schema.json")?;
    write::<QueryRequestV1>(&directory, "query-request-v1.schema.json")?;
    write::<QueryResponseV1>(&directory, "query-response-v1.schema.json")?;
    write::<ProofV1>(&directory, "proof-v1.schema.json")?;
    Ok(())
}

fn write<T: JsonSchema>(directory: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    let schema = SchemaGenerator::default().into_root_schema_for::<T>();
    let mut encoded = serde_json::to_string_pretty(&schema)?;
    encoded.push('\n');
    fs::write(directory.join(name), encoded)?;
    Ok(())
}
