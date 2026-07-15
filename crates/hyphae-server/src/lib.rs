// SPDX-License-Identifier: Apache-2.0

//! Secure loopback-first HTTP delivery for public versioned contracts.
//!
//! Opening the embedded engine does not start a listener. Callers explicitly
//! construct [`HyphaeServer`], bind it, and provide a graceful-shutdown future.

mod config;
mod error;
mod server;

pub use config::{BearerToken, DEFAULT_PORT, ServerConfig, ServerConfigError, ServerLimits};
pub use error::ServerError;
pub use server::{BoundServer, HyphaeServer};

use error::ApiError;
