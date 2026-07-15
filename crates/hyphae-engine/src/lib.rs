// SPDX-License-Identifier: Apache-2.0

//! Embeddable Hyphae facade over durable storage, structured query, and exact
//! provider-neutral retrieval.

mod document;
mod facade;

pub use document::{
    DocumentError, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_DEPTH, MAX_DOCUMENT_NODES, decode_document,
    encode_document,
};
pub use facade::{EngineError, HyphaeEngine, OpenedEngine};
