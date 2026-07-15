// SPDX-License-Identifier: Apache-2.0

//! Exact retrieval and provider-neutral abstention semantics.
//!
//! The reference engine accepts vectors directly. No embedding or model
//! provider is enabled by default.

mod engine;
mod model;

pub use engine::{
    RetrievalClock, RetrievalError, RetrievalSystemClock, retrieve, retrieve_with_clock,
};
pub use model::{
    Abstention, AbstentionReason, RetrievalLimits, RetrievalMatch, RetrievalOutcome,
    RetrievalRequest, VectorRecord,
};
