// SPDX-License-Identifier: Apache-2.0

//! Stable product identity and version values shared by Hyphae surfaces.

/// Canonical product and executable name.
pub const PRODUCT_NAME: &str = "hyphae";

/// Current public HTTP API version.
pub const API_VERSION: &str = "v1";

/// Current on-disk format version.
pub const DISK_FORMAT_VERSION: u16 = 1;

/// Product version information that can be reported without opening a data directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionInfo {
    /// Product name.
    pub product: &'static str,
    /// Cargo package version for the running binary.
    pub engine: &'static str,
    /// Public HTTP API version.
    pub api: &'static str,
    /// On-disk format version.
    pub disk_format: u16,
}

/// Returns the version information compiled into this build.
pub const fn current_version() -> VersionInfo {
    VersionInfo {
        product: PRODUCT_NAME,
        engine: env!("CARGO_PKG_VERSION"),
        api: API_VERSION,
        disk_format: DISK_FORMAT_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::{API_VERSION, DISK_FORMAT_VERSION, PRODUCT_NAME, current_version};

    #[test]
    fn current_version_matches_public_constants() {
        let version = current_version();
        assert_eq!(version.product, PRODUCT_NAME);
        assert_eq!(version.api, API_VERSION);
        assert_eq!(version.disk_format, DISK_FORMAT_VERSION);
        assert!(!version.engine.is_empty());
    }
}
