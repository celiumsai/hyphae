// SPDX-License-Identifier: Apache-2.0

//! Live Rust-client runner for the shared version 1 conformance fixture.

use std::{env, error::Error, io};

use hyphae_client::{ClientError, HyphaeClient};
use hyphae_contracts::v1::{DeleteRequestV1, GetRequestV1, PutRequestV1, QueryRequestV1};
use serde::Deserialize;

const FIXTURE: &str = include_str!("../../v1/cases.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    version: u16,
    put_request: PutRequestV1,
    conflict_put_request: PutRequestV1,
    present_get_request: GetRequestV1,
    absent_get_request: GetRequestV1,
    query_request: QueryRequestV1,
    delete_request: DeleteRequestV1,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    first_page_keys: Vec<String>,
    second_page_keys: Vec<String>,
    matched_records: u64,
    aggregation: serde_json::Value,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let fixture: Fixture = serde_json::from_str(FIXTURE)?;
    require(fixture.version == 1, "unsupported conformance fixture")?;
    let base_url =
        env::var("HYPHAE_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".to_owned());
    let mut builder = HyphaeClient::builder(&base_url)?;
    if let Ok(token) = env::var("HYPHAE_BEARER_TOKEN") {
        builder = builder.bearer_token(&token)?;
    }
    let client = builder.build()?;

    require(
        client.liveness().await?.value.status == "live",
        "liveness mismatch",
    )?;
    let capabilities = client.capabilities().await?.value;
    require(capabilities.api_version == "v1", "API version mismatch")?;
    require(
        capabilities
            .features
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "capabilities are not strictly sorted",
    )?;

    let first_put = client.put(&fixture.put_request).await?.value;
    require(first_put.status == "committed", "first put did not commit")?;
    let retry = client.put(&fixture.put_request).await?.value;
    require(
        retry.status == "existing",
        "idempotent retry was not existing",
    )?;
    match client.put(&fixture.conflict_put_request).await {
        Err(ClientError::Api(failure)) => require(
            failure.code == "idempotency_conflict",
            "wrong conflict error code",
        )?,
        _ => return Err(io::Error::other("idempotency conflict was accepted").into()),
    }

    let present = client.get(&fixture.present_get_request).await?.value;
    require(present.found, "present key was reported absent")?;
    require(
        present
            .record
            .as_ref()
            .map(|record| record.key_hex.as_str())
            == Some("61"),
        "present record key mismatch",
    )?;
    let witness = client.download_witness(&present.proof).await?.value;
    require(
        witness.starts_with(b"HYSNAP01"),
        "snapshot witness magic mismatch",
    )?;

    let absent = client.get(&fixture.absent_get_request).await?.value;
    require(!absent.found && absent.record.is_none(), "absence mismatch")?;

    let first_page = client.query(&fixture.query_request).await?.value;
    require(
        keys(&first_page.rows) == fixture.expected.first_page_keys,
        "first query page mismatch",
    )?;
    require(
        first_page.matched_records == fixture.expected.matched_records,
        "matched record count mismatch",
    )?;
    require(
        serde_json::to_value(&first_page.aggregation)? == fixture.expected.aggregation,
        "aggregation mismatch",
    )?;
    let mut second_request = fixture.query_request.clone();
    second_request.cursor = first_page.next_cursor;
    let second_page = client.query(&second_request).await?.value;
    require(
        keys(&second_page.rows) == fixture.expected.second_page_keys,
        "second query page mismatch",
    )?;
    require(second_page.next_cursor.is_none(), "unexpected third page")?;

    client.delete(&fixture.delete_request).await?;
    let deleted = client
        .get(&GetRequestV1 {
            key_hex: "62".to_owned(),
        })
        .await?
        .value;
    require(!deleted.found, "deleted key is still present")?;

    println!(r#"{{"client":"rust","status":"ok"}}"#);
    Ok(())
}

fn keys(records: &[hyphae_contracts::v1::RecordV1]) -> Vec<String> {
    records
        .iter()
        .map(|record| record.key_hex.clone())
        .collect()
}

fn require(condition: bool, message: &'static str) -> Result<(), io::Error> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}

#[cfg(test)]
mod tests {
    use super::{FIXTURE, Fixture};

    #[test]
    fn shared_fixture_matches_typed_public_contracts() -> Result<(), serde_json::Error> {
        let fixture: Fixture = serde_json::from_str(FIXTURE)?;
        assert_eq!(fixture.version, 1);
        assert_eq!(fixture.put_request.records.len(), 4);
        Ok(())
    }
}
