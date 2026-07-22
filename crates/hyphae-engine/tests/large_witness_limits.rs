//! Regression coverage for large, bounded offline retrieval witnesses.

use hyphae_engine::retrieval_proof::RetrievalVerificationLimits;
use hyphae_storage::SnapshotReadLimits;

#[test]
fn default_offline_limits_accept_large_but_bounded_retrieval_witnesses() {
    let snapshot = SnapshotReadLimits::default();
    assert_eq!(snapshot.file_bytes, 2 * 1024 * 1024 * 1024);
    assert_eq!(snapshot.decoded_bytes, 1024 * 1024 * 1024);

    let retrieval = RetrievalVerificationLimits::default();
    assert_eq!(retrieval.snapshot, snapshot);
    assert_eq!(retrieval.max_candidate_bytes, 1024 * 1024 * 1024);
}
