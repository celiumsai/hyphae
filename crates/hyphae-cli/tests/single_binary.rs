// SPDX-License-Identifier: Apache-2.0

//! Black-box conformance for the autonomous single-binary experience.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use uuid::Uuid;

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn create() -> Result<Self, Box<dyn Error>> {
        let path =
            std::env::temp_dir().join(format!("hyphae-cli-single-binary-{}", Uuid::now_v7()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.path);
    }
}

fn run(data_directory: &Path, arguments: &[&str]) -> Result<serde_json::Value, Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_hyphae"))
        .args(arguments)
        .arg("--data-dir")
        .arg(data_directory)
        .output()?;
    decode_output(&output, arguments)
}

fn run_without_data(arguments: &[&str]) -> Result<serde_json::Value, Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_hyphae"))
        .args(arguments)
        .output()?;
    decode_output(&output, arguments)
}

fn decode_output(
    output: &std::process::Output,
    arguments: &[&str],
) -> Result<serde_json::Value, Box<dyn Error>> {
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "hyphae {:?} failed: {}",
            arguments,
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
fn one_binary_persists_queries_and_compacts_without_external_services() -> Result<(), Box<dyn Error>>
{
    let data_directory = TestDirectory::create()?;
    let transaction_id = "019f0000-0000-7000-8000-000000000001";

    let committed = run(
        &data_directory.path,
        &[
            "put",
            "--key",
            "alpha",
            "--json",
            r#"{"score":10,"group":"x"}"#,
            "--transaction-id",
            transaction_id,
        ],
    )?;
    assert_eq!(committed["status"], "committed");

    let repeated = run(
        &data_directory.path,
        &[
            "put",
            "--key",
            "alpha",
            "--json",
            r#"{"group":"x","score":10}"#,
            "--transaction-id",
            transaction_id,
        ],
    )?;
    assert_eq!(repeated["status"], "existing");
    assert_eq!(repeated["commit_digest"], committed["commit_digest"]);

    run(
        &data_directory.path,
        &[
            "put",
            "--key",
            "beta",
            "--json",
            r#"{"score":20,"group":"x"}"#,
        ],
    )?;

    let filtered = run(
        &data_directory.path,
        &[
            "query", "--field", "group", "--equals", r#""x""#, "--sort", "score",
        ],
    )?;
    assert_eq!(filtered["scanned_records"], 2);
    assert_eq!(filtered["matched_records"], 2);

    let before = run(
        &data_directory.path,
        &["query", "--sort", "score", "--descending", "--limit", "2"],
    )?;
    assert_eq!(before["rows"][0]["key_hex"], "62657461");
    assert_eq!(before["rows"][1]["key_hex"], "616c706861");

    let proof_path = data_directory.path.join("query.hyproof");
    let proof_path_string = proof_path.to_string_lossy().into_owned();
    let proven = run(
        &data_directory.path,
        &[
            "query",
            "--sort",
            "score",
            "--descending",
            "--limit",
            "2",
            "--proof-out",
            &proof_path_string,
        ],
    )?;
    assert_eq!(proven["rows"], before["rows"]);
    let snapshot_path = proven["proof"]["snapshot_path"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("missing snapshot path"))?;
    let anchor = proven["proof"]["anchor_digest"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("missing anchor digest"))?;
    let verified = run_without_data(&[
        "verify",
        "--proof",
        &proof_path_string,
        "--snapshot",
        snapshot_path,
        "--anchor",
        anchor,
    ])?;
    assert_eq!(verified["status"], "verified");

    let snapshot = run(&data_directory.path, &["snapshot"])?;
    assert_eq!(snapshot["entry_count"], 2);
    let compacted = run(&data_directory.path, &["compact"])?;
    assert_eq!(compacted["status"], "compacted");

    let after = run(
        &data_directory.path,
        &["query", "--sort", "score", "--descending", "--limit", "2"],
    )?;
    assert_eq!(after["rows"], before["rows"]);
    assert_eq!(after["matched_records"], before["matched_records"]);
    assert_eq!(after["scanned_records"], before["scanned_records"]);
    Ok(())
}
