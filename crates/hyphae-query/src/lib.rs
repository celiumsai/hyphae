// SPDX-License-Identifier: Apache-2.0

//! Deterministic typed query model and executable reference semantics.

mod engine;
mod model;
mod value;

pub use engine::{MonotonicClock, QueryError, SystemClock, execute, execute_with_clock};
pub use model::{
    AggregationPlan, AggregationResult, CompareOperator, Cursor, ExecutionLimits, Filter,
    GroupResult, Metric, MetricValue, NamedMetric, NamedMetricValue, NullPlacement, Query,
    QueryResult, Record, SortDirection, SortField,
};
pub use value::{FieldPath, Value};
