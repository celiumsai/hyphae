// SPDX-License-Identifier: Apache-2.0

//! Deterministic typed query model and executable reference semantics.

mod document;
mod engine;
mod model;
mod value;

pub use document::{
    DocumentError, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_DEPTH, MAX_DOCUMENT_NODES, decode_document,
    encode_document,
};
pub use engine::{
    MonotonicClock, QueryError, SystemClock, execute, execute_with_clock, validate_query,
};
pub use model::{
    AggregationPlan, AggregationResult, CompareOperator, Cursor, ExecutionLimits, Filter,
    GroupResult, Metric, MetricValue, NamedMetric, NamedMetricValue, NullPlacement, Query,
    QueryResult, Record, SortDirection, SortField,
};
pub use value::{FieldPath, Value};
