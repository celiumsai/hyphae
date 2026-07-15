// SPDX-License-Identifier: Apache-2.0

//! Embeddable Hyphae facade over durable storage, structured query, and exact
//! provider-neutral retrieval.

mod document;
mod facade;
pub mod proof;

pub use document::{
    DocumentError, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_DEPTH, MAX_DOCUMENT_NODES, decode_document,
    encode_document,
};
pub use facade::{EngineError, HyphaeEngine, OpenedEngine};
pub use proof::{
    MAX_RESULT_PROOF_BYTES, ProofAnchor, ProofError, ProvenOperation, ProvenResult,
    RESULT_PROOF_FORMAT_VERSION, ResultProof, ResultProofArtifact, VerificationLimits,
    VerificationReport, read_result_proof, verify_result_proof, write_result_proof,
};
