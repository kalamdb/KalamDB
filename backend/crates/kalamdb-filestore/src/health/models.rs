//! Health check result models.

use serde::{Deserialize, Serialize};

/// Health status of a storage backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// All health checks passed successfully.
    Healthy,
    /// Some health checks failed (e.g., read-only or write-only access).
    Degraded,
    /// Cannot connect to the storage backend at all.
    Unreachable,
}

impl HealthStatus {
    /// Returns the status as a lowercase string.
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Unreachable => "unreachable",
        }
    }
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Result of a lightweight connectivity test.
///
/// Used for quick connection validation without full CRUD testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityTestResult {
    /// Whether the connection was successful.
    pub connected: bool,
    /// Latency of the connection test in milliseconds.
    pub latency_ms: u64,
    /// Error message if connection failed.
    pub error: Option<String>,
}

impl ConnectivityTestResult {
    /// Create a successful connectivity result.
    pub fn success(latency_ms: u64) -> Self {
        Self {
            connected: true,
            latency_ms,
            error: None,
        }
    }

    /// Create a failed connectivity result.
    pub fn failure(error: String, latency_ms: u64) -> Self {
        Self {
            connected: false,
            latency_ms,
            error: Some(error),
        }
    }
}

/// Comprehensive health check result for a storage backend.
///
/// Contains detailed information about each operation's success/failure
/// and optional capacity information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageHealthResult {
    /// Overall health status.
    pub status: HealthStatus,
    /// Whether read operations succeed.
    pub readable: bool,
    /// Whether write operations succeed.
    pub writable: bool,
    /// Whether list operations succeed.
    pub listable: bool,
    /// Whether delete operations succeed.
    pub deletable: bool,
    /// Total latency for all operations in milliseconds.
    pub latency_ms: u64,
    /// Total storage capacity in bytes (if available).
    pub total_bytes: Option<u64>,
    /// Used storage space in bytes (if available).
    pub used_bytes: Option<u64>,
    /// Error message if any operation failed.
    pub error: Option<String>,
    /// Unix timestamp in microseconds when the health check was performed.
    pub tested_at: i64,
}

impl StorageHealthResult {
    /// Create a healthy result with all tests passing.
    pub fn healthy(latency_ms: u64) -> Self {
        Self {
            status: HealthStatus::Healthy,
            readable: true,
            writable: true,
            listable: true,
            deletable: true,
            latency_ms,
            total_bytes: None,
            used_bytes: None,
            error: None,
            tested_at: chrono::Utc::now().timestamp_micros(),
        }
    }

    /// Create an unreachable result when connection fails.
    pub fn unreachable(error: String, latency_ms: u64) -> Self {
        Self {
            status: HealthStatus::Unreachable,
            readable: false,
            writable: false,
            listable: false,
            deletable: false,
            latency_ms,
            total_bytes: None,
            used_bytes: None,
            error: Some(error),
            tested_at: chrono::Utc::now().timestamp_micros(),
        }
    }

    /// Create a degraded result with partial functionality.
    pub fn degraded(
        readable: bool,
        writable: bool,
        listable: bool,
        deletable: bool,
        error: String,
        latency_ms: u64,
    ) -> Self {
        Self {
            status: HealthStatus::Degraded,
            readable,
            writable,
            listable,
            deletable,
            latency_ms,
            total_bytes: None,
            used_bytes: None,
            error: Some(error),
            tested_at: chrono::Utc::now().timestamp_micros(),
        }
    }

    /// Set storage capacity information.
    pub fn with_capacity(mut self, total_bytes: Option<u64>, used_bytes: Option<u64>) -> Self {
        self.total_bytes = total_bytes;
        self.used_bytes = used_bytes;
        self
    }

    /// Check if the storage is fully operational.
    pub fn is_healthy(&self) -> bool {
        self.status == HealthStatus::Healthy
    }

    /// Check if there are any issues with the storage.
    pub fn has_issues(&self) -> bool {
        self.status != HealthStatus::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_as_str() {
        assert_eq!(HealthStatus::Healthy.as_str(), "healthy");
        assert_eq!(HealthStatus::Degraded.as_str(), "degraded");
        assert_eq!(HealthStatus::Unreachable.as_str(), "unreachable");
    }

    #[test]
    fn test_connectivity_result_success() {
        let result = ConnectivityTestResult::success(42);
        assert!(result.connected);
        assert_eq!(result.latency_ms, 42);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_connectivity_result_failure() {
        let result = ConnectivityTestResult::failure("timeout".to_string(), 5000);
        assert!(!result.connected);
        assert_eq!(result.latency_ms, 5000);
        assert_eq!(result.error, Some("timeout".to_string()));
    }

    #[test]
    fn test_storage_health_result_healthy() {
        let result = StorageHealthResult::healthy(100);
        assert!(result.is_healthy());
        assert!(!result.has_issues());
        assert!(result.readable);
        assert!(result.writable);
        assert!(result.listable);
        assert!(result.deletable);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_storage_health_result_unreachable() {
        let result = StorageHealthResult::unreachable("connection refused".to_string(), 0);
        assert!(!result.is_healthy());
        assert!(result.has_issues());
        assert_eq!(result.status, HealthStatus::Unreachable);
        assert!(!result.readable);
        assert!(!result.writable);
    }

    #[test]
    fn test_storage_health_result_with_capacity() {
        let result = StorageHealthResult::healthy(50).with_capacity(Some(1_000_000), Some(500_000));
        assert_eq!(result.total_bytes, Some(1_000_000));
        assert_eq!(result.used_bytes, Some(500_000));
    }
}
