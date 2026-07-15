// SPDX-License-Identifier: Apache-2.0

use std::{io, net::SocketAddr};

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use hyphae_contracts::v1::ErrorV1;
use hyphae_engine::{EngineError, ProofError};
use hyphae_query::QueryError;
use hyphae_storage::{LogError, MutationError, StorageError};
use thiserror::Error;

use crate::ServerConfigError;

/// Failure before or while running the optional HTTP delivery surface.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Secure configuration validation failed before socket bind.
    #[error(transparent)]
    Configuration(#[from] ServerConfigError),
    /// The exclusively owned embedded engine could not open.
    #[error("failed to open Hyphae engine: {0}")]
    Engine(#[from] EngineError),
    /// The requested socket could not be bound.
    #[error("failed to bind Hyphae server at {address}: {source}")]
    Bind {
        /// Requested listener address.
        address: SocketAddr,
        /// Operating-system failure.
        #[source]
        source: io::Error,
    },
    /// The bound HTTP service failed.
    #[error("Hyphae HTTP service failed: {0}")]
    Serve(#[source] io::Error),
}

#[derive(Clone, Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: String,
}

impl ApiError {
    pub(crate) fn new(
        status: StatusCode,
        code: &'static str,
        message: &'static str,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code,
            message,
            request_id: request_id.into(),
        }
    }

    pub(crate) fn invalid(request_id: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "request does not satisfy the version 1 contract",
            request_id,
        )
    }

    pub(crate) fn limit(request_id: &str) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "limit_exceeded",
            "request exceeds an enforced server limit",
            request_id,
        )
    }

    pub(crate) fn payload_too_large(request_id: &str) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "request or response byte budget exceeded",
            request_id,
        )
    }

    pub(crate) fn result_too_large(request_id: &str) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "result_too_large",
            "proof-bearing result exceeds an enforced byte limit",
            request_id,
        )
    }

    pub(crate) fn internal(request_id: &str) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal operation failed; inspect local server diagnostics",
            request_id,
        )
    }

    pub(crate) fn unavailable(request_id: &str) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "owned engine requires local recovery before serving data operations",
            request_id,
        )
    }

    pub(crate) fn from_engine(error: EngineError, request_id: &str) -> Self {
        match error {
            EngineError::DuplicateDocumentKey => Self::invalid(request_id),
            EngineError::Document(_) => Self::limit(request_id),
            EngineError::Query(source) => Self::from_query(&source, request_id),
            EngineError::Storage(source) => Self::from_storage(&source, request_id),
            EngineError::Proof(ProofError::ProofLimitExceeded { .. }) => {
                Self::result_too_large(request_id)
            }
            EngineError::Proof(_) | EngineError::Retrieval(_) => Self::internal(request_id),
        }
    }

    fn from_query(error: &QueryError, request_id: &str) -> Self {
        match error {
            QueryError::TimedOut => Self::new(
                StatusCode::REQUEST_TIMEOUT,
                "timeout",
                "query deadline elapsed without a partial result",
                request_id,
            ),
            QueryError::ResultLimitExceeded { .. }
            | QueryError::FilterNodesExceeded { .. }
            | QueryError::FilterDepthExceeded { .. }
            | QueryError::SortFieldsExceeded { .. }
            | QueryError::GroupFieldsExceeded { .. }
            | QueryError::MetricsExceeded { .. }
            | QueryError::ScannedBudgetExceeded { .. }
            | QueryError::MatchedBudgetExceeded { .. }
            | QueryError::GroupBudgetExceeded { .. } => Self::limit(request_id),
            QueryError::EmptyRecordKey
            | QueryError::DuplicateRecordKey
            | QueryError::ZeroLimit
            | QueryError::CursorShape { .. }
            | QueryError::EmptyCursorKey
            | QueryError::NoncanonicalCursorNull
            | QueryError::InvalidPrefixType
            | QueryError::InvalidFieldPath
            | QueryError::EmptyMetricName
            | QueryError::DuplicateMetricName { .. }
            | QueryError::MetricTypeMismatch { .. }
            | QueryError::ArithmeticOverflow { .. }
            | QueryError::MetricStateMismatch => Self::invalid(request_id),
        }
    }

    fn from_storage(error: &StorageError, request_id: &str) -> Self {
        match error {
            StorageError::Mutation(
                MutationError::EmptyKey
                | MutationError::KeyTooLarge { .. }
                | MutationError::OperationTooLarge { .. },
            )
            | StorageError::Log(
                LogError::EmptyTransaction
                | LogError::TooManyOperations
                | LogError::PayloadTooLarge { .. },
            ) => Self::limit(request_id),
            StorageError::Log(LogError::IdempotencyConflict { .. }) => Self::new(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "transaction identifier was already committed with different contents",
                request_id,
            ),
            _ => Self::internal(request_id),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let envelope = ErrorV1 {
            code: self.code.to_owned(),
            message: self.message.to_owned(),
            request_id: self.request_id,
        };
        let mut response = (self.status, Json(envelope)).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"hyphae\""),
            );
        }
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        response
    }
}
