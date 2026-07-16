// SPDX-License-Identifier: Apache-2.0

//! Executable compatibility evidence for every supported disk format.

use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Component, Path, PathBuf},
};

use hyphae_engine::HyphaeEngine;
use hyphae_query::{Record, Value};
use hyphae_storage::AppendOutcome;
use uuid::Uuid;

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn create() -> Result<Self, Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!(
            "hyphae-disk-compatibility-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.path);
    }
}

fn decode_hex(encoded: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    if !encoded.len().is_multiple_of(2) {
        return Err("fixture contains odd-length hexadecimal data".into());
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair)?;
            Ok(u8::from_str_radix(text, 16)?)
        })
        .collect()
}

fn fixture_path(root: &Path, relative: &str) -> Result<PathBuf, Box<dyn Error>> {
    let relative = Path::new(relative);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("fixture contains a noncanonical relative path".into());
    }
    Ok(root.join(relative))
}

#[test]
fn disk_format_1_opens_rebuilds_and_preserves_idempotency() -> Result<(), Box<dyn Error>> {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/data-directory.json"))?;
    assert_eq!(fixture["fixture_version"], 1);
    assert_eq!(fixture["disk_format_version"], 1);

    let temporary = TestDirectory::create()?;
    let data = temporary.path.join("data");
    let files = fixture["files_hex"]
        .as_object()
        .ok_or("fixture files_hex is not an object")?;
    for (relative, encoded) in files {
        let path = fixture_path(&data, relative)?;
        fs::create_dir_all(path.parent().ok_or("fixture file has no parent")?)?;
        fs::write(
            path,
            decode_hex(encoded.as_str().ok_or("fixture file is not hexadecimal")?)?,
        )?;
    }

    assert!(!data.join("indexes/primary.redb").exists());
    let mut opened = HyphaeEngine::open(&data)?;
    let expected_value = Value::Object(BTreeMap::from([
        ("group".to_owned(), Value::String("fixture".to_owned())),
        ("score".to_owned(), Value::Integer(42)),
    ]));
    assert_eq!(
        opened.engine.get_record(b"alpha")?,
        Some(Record::new(b"alpha", expected_value.clone()))
    );
    assert!(data.join("indexes/primary.redb").exists());

    let transaction_id = Uuid::parse_str(
        fixture["expected"]["transaction_id"]
            .as_str()
            .ok_or("fixture transaction ID is not a string")?,
    )?;
    let repeated = opened
        .engine
        .put_record(transaction_id, &Record::new(b"alpha", expected_value))?;
    let AppendOutcome::Existing(receipt) = repeated else {
        return Err("historical transaction was appended again".into());
    };
    assert_eq!(
        receipt.commit_sequence,
        fixture["expected"]["commit_sequence"]
            .as_u64()
            .ok_or("fixture commit sequence is not an integer")?
    );
    assert_eq!(
        hex(&receipt.commit_digest),
        fixture["expected"]["commit_digest"]
            .as_str()
            .ok_or("fixture commit digest is not a string")?
    );
    assert_eq!(
        hex(&receipt.transaction_digest),
        fixture["expected"]["transaction_digest"]
            .as_str()
            .ok_or("fixture transaction digest is not a string")?
    );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}
