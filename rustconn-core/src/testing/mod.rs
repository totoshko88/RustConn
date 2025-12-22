//! Connection testing module for verifying connectivity.
//!
//! This module provides functionality to test connections by verifying
//! port accessibility and protocol handshakes.

// Allow precision loss for percentage calculations - acceptable for display purposes
#![allow(clippy::cast_precision_loss)]
// Allow truncation for millisecond conversion - latencies won't exceed u64::MAX
#![allow(clippy::cast_possible_truncation)]

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;
use uuid::Uuid;

use crate::models::{Connection, ProtocolType};

/// Default timeout for connection tests (10 seconds)
pub const DEFAULT_TEST_TIMEOUT_SECS: u64 = 10;

/// Default number of concurrent tests for batch operations
pub const DEFAULT_CONCURRENCY: usize = 10;

/// Errors that can occur during connection testing
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TestError {
    /// Connection timed out
    #[error("Connection timeout after {0} seconds")]
    Timeout(u64),

    /// Connection was refused by the remote host
    #[error("Connection refused")]
    ConnectionRefused,

    /// Host is unreachable
    #[error("Host unreachable: {0}")]
    HostUnreachable(String),

    /// DNS resolution failed
    #[error("DNS resolution failed: {0}")]
    DnsResolutionFailed(String),

    /// Protocol handshake failed
    #[error("Protocol handshake failed: {0}")]
    ProtocolError(String),

    /// I/O error during test
    #[error("IO error: {0}")]
    IoError(String),

    /// Invalid connection configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Result type alias for testing operations
pub type TestResult2<T> = std::result::Result<T, TestError>;

/// Result of testing a single connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// ID of the connection that was tested
    pub connection_id: Uuid,
    /// Name of the connection
    pub connection_name: String,
    /// Whether the test was successful
    pub success: bool,
    /// Connection latency in milliseconds (if successful)
    pub latency_ms: Option<u64>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// Additional details about the test
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub details: HashMap<String, String>,
}

impl TestResult {
    /// Creates a successful test result
    #[must_use]
    pub fn success(connection_id: Uuid, connection_name: String, latency_ms: u64) -> Self {
        Self {
            connection_id,
            connection_name,
            success: true,
            latency_ms: Some(latency_ms),
            error: None,
            details: HashMap::new(),
        }
    }

    /// Creates a failed test result
    #[must_use]
    pub fn failure(connection_id: Uuid, connection_name: String, error: impl Into<String>) -> Self {
        Self {
            connection_id,
            connection_name,
            success: false,
            latency_ms: None,
            error: Some(error.into()),
            details: HashMap::new(),
        }
    }

    /// Creates a failed test result from a `TestError`
    #[must_use]
    pub fn from_error(connection_id: Uuid, connection_name: String, error: &TestError) -> Self {
        Self::failure(connection_id, connection_name, error.to_string())
    }

    /// Adds a detail to the test result
    #[must_use]
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    /// Returns true if the test was successful
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.success
    }

    /// Returns true if the test failed
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        !self.success
    }
}

/// Summary of batch test results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSummary {
    /// Total number of connections tested
    pub total: usize,
    /// Number of tests that passed
    pub passed: usize,
    /// Number of tests that failed
    pub failed: usize,
    /// Individual test results
    pub results: Vec<TestResult>,
}

impl TestSummary {
    /// Creates a new empty test summary
    #[must_use]
    pub const fn new() -> Self {
        Self {
            total: 0,
            passed: 0,
            failed: 0,
            results: Vec::new(),
        }
    }

    /// Creates a test summary from a list of results
    #[must_use]
    pub fn from_results(results: Vec<TestResult>) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.success).count();
        let failed = total - passed;

        Self {
            total,
            passed,
            failed,
            results,
        }
    }

    /// Adds a test result to the summary
    pub fn add_result(&mut self, result: TestResult) {
        if result.success {
            self.passed += 1;
        } else {
            self.failed += 1;
        }
        self.total += 1;
        self.results.push(result);
    }

    /// Returns true if all tests passed
    #[must_use]
    pub const fn all_passed(&self) -> bool {
        self.failed == 0
    }

    /// Returns true if any tests failed
    #[must_use]
    pub const fn has_failures(&self) -> bool {
        self.failed > 0
    }

    /// Returns the pass rate as a percentage (0.0 to 100.0)
    #[must_use]
    pub fn pass_rate(&self) -> f64 {
        if self.total == 0 {
            return 100.0;
        }
        (self.passed as f64 / self.total as f64) * 100.0
    }

    /// Returns a summary string
    #[must_use]
    pub fn summary_string(&self) -> String {
        format!(
            "Total: {}, Passed: {}, Failed: {} ({:.1}% pass rate)",
            self.total,
            self.passed,
            self.failed,
            self.pass_rate()
        )
    }

    /// Returns only the failed results
    #[must_use]
    pub fn failed_results(&self) -> Vec<&TestResult> {
        self.results.iter().filter(|r| !r.success).collect()
    }

    /// Returns only the successful results
    #[must_use]
    pub fn successful_results(&self) -> Vec<&TestResult> {
        self.results.iter().filter(|r| r.success).collect()
    }
}

impl Default for TestSummary {
    fn default() -> Self {
        Self::new()
    }
}

/// Connection tester for verifying connectivity
pub struct ConnectionTester {
    /// Timeout for connection tests
    timeout: Duration,
    /// Maximum concurrent tests for batch operations
    concurrency: usize,
}

impl ConnectionTester {
    /// Creates a new connection tester with default settings
    #[must_use]
    pub const fn new() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_TEST_TIMEOUT_SECS),
            concurrency: DEFAULT_CONCURRENCY,
        }
    }

    /// Creates a new connection tester with a custom timeout
    #[must_use]
    pub const fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            concurrency: DEFAULT_CONCURRENCY,
        }
    }

    /// Sets the timeout for connection tests
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the maximum concurrency for batch tests
    #[must_use]
    pub const fn concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// Gets the current timeout setting
    #[must_use]
    pub const fn get_timeout(&self) -> Duration {
        self.timeout
    }

    /// Gets the current concurrency setting
    #[must_use]
    pub const fn get_concurrency(&self) -> usize {
        self.concurrency
    }

    /// Tests a single connection
    ///
    /// This method tests connectivity to the specified connection by:
    /// 1. Testing TCP port accessibility
    /// 2. For SSH connections, verifying the SSH banner exchange
    ///
    /// # Arguments
    ///
    /// * `connection` - The connection to test
    ///
    /// # Returns
    ///
    /// A `TestResult` indicating success or failure with details
    pub async fn test_connection(&self, connection: &Connection) -> TestResult {
        let start = std::time::Instant::now();

        // First test port connectivity
        match self.test_port(&connection.host, connection.port).await {
            Ok(latency) => {
                // For SSH, also verify the protocol handshake
                if connection.protocol == ProtocolType::Ssh {
                    match self.test_ssh(connection).await {
                        Ok(()) => {
                            let latency_ms = latency.as_millis() as u64;
                            TestResult::success(connection.id, connection.name.clone(), latency_ms)
                                .with_detail("protocol", "SSH")
                                .with_detail("handshake", "verified")
                        }
                        Err(e) => {
                            TestResult::from_error(connection.id, connection.name.clone(), &e)
                                .with_detail("protocol", "SSH")
                                .with_detail("port_open", "true")
                        }
                    }
                } else {
                    // For RDP/VNC, port connectivity is sufficient
                    let latency_ms = latency.as_millis() as u64;
                    TestResult::success(connection.id, connection.name.clone(), latency_ms)
                        .with_detail("protocol", connection.protocol.to_string())
                }
            }
            Err(e) => {
                let elapsed = start.elapsed().as_millis() as u64;
                TestResult::from_error(connection.id, connection.name.clone(), &e)
                    .with_detail("elapsed_ms", elapsed.to_string())
            }
        }
    }

    /// Tests TCP port connectivity
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname or IP address
    /// * `port` - The port number
    ///
    /// # Returns
    ///
    /// The connection latency on success, or an error on failure
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails due to timeout, connection refused,
    /// host unreachable, DNS resolution failure, or other I/O errors.
    pub async fn test_port(&self, host: &str, port: u16) -> TestResult2<Duration> {
        let addr = format!("{host}:{port}");
        let start = std::time::Instant::now();

        let timeout_secs = self.timeout.as_secs();

        match timeout(self.timeout, TcpStream::connect(&addr)).await {
            Ok(Ok(_stream)) => Ok(start.elapsed()),
            Ok(Err(e)) => {
                let error_str = e.to_string().to_lowercase();
                if error_str.contains("refused") {
                    Err(TestError::ConnectionRefused)
                } else if error_str.contains("no route")
                    || error_str.contains("unreachable")
                    || error_str.contains("network is down")
                {
                    Err(TestError::HostUnreachable(host.to_string()))
                } else if error_str.contains("name or service not known")
                    || error_str.contains("no such host")
                    || error_str.contains("dns")
                    || error_str.contains("resolve")
                {
                    Err(TestError::DnsResolutionFailed(host.to_string()))
                } else {
                    Err(TestError::IoError(e.to_string()))
                }
            }
            Err(_) => Err(TestError::Timeout(timeout_secs)),
        }
    }

    /// Tests SSH protocol handshake
    ///
    /// Verifies that the remote host responds with a valid SSH banner.
    ///
    /// # Arguments
    ///
    /// * `connection` - The SSH connection to test
    ///
    /// # Returns
    ///
    /// `Ok(())` if the SSH banner is valid, or an error otherwise
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, times out, or the SSH banner
    /// is invalid or missing.
    pub async fn test_ssh(&self, connection: &Connection) -> TestResult2<()> {
        let addr = format!("{}:{}", connection.host, connection.port);

        let stream = match timeout(self.timeout, TcpStream::connect(&addr)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(TestError::IoError(e.to_string())),
            Err(_) => return Err(TestError::Timeout(self.timeout.as_secs())),
        };

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut banner = String::new();

        // Read the SSH banner (server sends it first)
        match timeout(self.timeout, reader.read_line(&mut banner)).await {
            Ok(Ok(0)) => {
                return Err(TestError::ProtocolError(
                    "Connection closed before receiving banner".to_string(),
                ));
            }
            Ok(Ok(_)) => {
                // Verify it's a valid SSH banner (starts with "SSH-")
                if !banner.starts_with("SSH-") {
                    return Err(TestError::ProtocolError(format!(
                        "Invalid SSH banner: {}",
                        banner.trim()
                    )));
                }
            }
            Ok(Err(e)) => return Err(TestError::IoError(e.to_string())),
            Err(_) => return Err(TestError::Timeout(self.timeout.as_secs())),
        }

        // Send our client banner to complete the handshake
        let client_banner = "SSH-2.0-RustConn_Test\r\n";
        if let Err(e) = timeout(self.timeout, writer.write_all(client_banner.as_bytes())).await {
            return Err(TestError::IoError(format!("Failed to send banner: {e}")));
        }

        Ok(())
    }

    /// Tests multiple connections concurrently
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to test
    ///
    /// # Returns
    ///
    /// A `TestSummary` with results for all connections
    pub async fn test_batch(&self, connections: &[Connection]) -> TestSummary {
        use futures::stream::{self, StreamExt};

        let results: Vec<TestResult> = stream::iter(connections)
            .map(|conn| self.test_connection(conn))
            .buffer_unordered(self.concurrency)
            .collect()
            .await;

        TestSummary::from_results(results)
    }
}

impl Default for ConnectionTester {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_result_success() {
        let id = Uuid::new_v4();
        let result = TestResult::success(id, "Test Server".to_string(), 50);

        assert!(result.is_success());
        assert!(!result.is_failure());
        assert_eq!(result.connection_id, id);
        assert_eq!(result.connection_name, "Test Server");
        assert_eq!(result.latency_ms, Some(50));
        assert!(result.error.is_none());
    }

    #[test]
    fn test_test_result_failure() {
        let id = Uuid::new_v4();
        let result = TestResult::failure(id, "Test Server".to_string(), "Connection refused");

        assert!(!result.is_success());
        assert!(result.is_failure());
        assert_eq!(result.connection_id, id);
        assert!(result.latency_ms.is_none());
        assert_eq!(result.error, Some("Connection refused".to_string()));
    }

    #[test]
    fn test_test_result_from_error() {
        let id = Uuid::new_v4();
        let error = TestError::ConnectionRefused;
        let result = TestResult::from_error(id, "Test Server".to_string(), &error);

        assert!(result.is_failure());
        assert_eq!(result.error, Some("Connection refused".to_string()));
    }

    #[test]
    fn test_test_result_with_detail() {
        let id = Uuid::new_v4();
        let result = TestResult::success(id, "Test".to_string(), 50)
            .with_detail("protocol", "SSH")
            .with_detail("version", "2.0");

        assert_eq!(result.details.get("protocol"), Some(&"SSH".to_string()));
        assert_eq!(result.details.get("version"), Some(&"2.0".to_string()));
    }

    #[test]
    fn test_test_summary_new() {
        let summary = TestSummary::new();

        assert_eq!(summary.total, 0);
        assert_eq!(summary.passed, 0);
        assert_eq!(summary.failed, 0);
        assert!(summary.results.is_empty());
    }

    #[test]
    fn test_test_summary_from_results() {
        let results = vec![
            TestResult::success(Uuid::new_v4(), "Server1".to_string(), 50),
            TestResult::failure(Uuid::new_v4(), "Server2".to_string(), "Error"),
            TestResult::success(Uuid::new_v4(), "Server3".to_string(), 100),
        ];

        let summary = TestSummary::from_results(results);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn test_test_summary_add_result() {
        let mut summary = TestSummary::new();

        summary.add_result(TestResult::success(Uuid::new_v4(), "S1".to_string(), 50));
        assert_eq!(summary.total, 1);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 0);

        summary.add_result(TestResult::failure(Uuid::new_v4(), "S2".to_string(), "Err"));
        assert_eq!(summary.total, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn test_test_summary_all_passed() {
        let results = vec![
            TestResult::success(Uuid::new_v4(), "S1".to_string(), 50),
            TestResult::success(Uuid::new_v4(), "S2".to_string(), 100),
        ];
        let summary = TestSummary::from_results(results);

        assert!(summary.all_passed());
        assert!(!summary.has_failures());
    }

    #[test]
    fn test_test_summary_has_failures() {
        let results = vec![
            TestResult::success(Uuid::new_v4(), "S1".to_string(), 50),
            TestResult::failure(Uuid::new_v4(), "S2".to_string(), "Error"),
        ];
        let summary = TestSummary::from_results(results);

        assert!(!summary.all_passed());
        assert!(summary.has_failures());
    }

    #[test]
    fn test_test_summary_pass_rate() {
        let results = vec![
            TestResult::success(Uuid::new_v4(), "S1".to_string(), 50),
            TestResult::success(Uuid::new_v4(), "S2".to_string(), 100),
            TestResult::failure(Uuid::new_v4(), "S3".to_string(), "Error"),
            TestResult::failure(Uuid::new_v4(), "S4".to_string(), "Error"),
        ];
        let summary = TestSummary::from_results(results);

        assert!((summary.pass_rate() - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_test_summary_pass_rate_empty() {
        let summary = TestSummary::new();
        assert!((summary.pass_rate() - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_test_summary_failed_results() {
        let results = vec![
            TestResult::success(Uuid::new_v4(), "S1".to_string(), 50),
            TestResult::failure(Uuid::new_v4(), "S2".to_string(), "Error"),
        ];
        let summary = TestSummary::from_results(results);

        let failed = summary.failed_results();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].connection_name, "S2");
    }

    #[test]
    fn test_test_summary_successful_results() {
        let results = vec![
            TestResult::success(Uuid::new_v4(), "S1".to_string(), 50),
            TestResult::failure(Uuid::new_v4(), "S2".to_string(), "Error"),
        ];
        let summary = TestSummary::from_results(results);

        let successful = summary.successful_results();
        assert_eq!(successful.len(), 1);
        assert_eq!(successful[0].connection_name, "S1");
    }

    #[test]
    fn test_connection_tester_new() {
        let tester = ConnectionTester::new();

        assert_eq!(
            tester.get_timeout(),
            Duration::from_secs(DEFAULT_TEST_TIMEOUT_SECS)
        );
        assert_eq!(tester.get_concurrency(), DEFAULT_CONCURRENCY);
    }

    #[test]
    fn test_connection_tester_with_timeout() {
        let tester = ConnectionTester::with_timeout(Duration::from_secs(30));

        assert_eq!(tester.get_timeout(), Duration::from_secs(30));
    }

    #[test]
    fn test_connection_tester_builder() {
        let tester = ConnectionTester::new()
            .timeout(Duration::from_secs(5))
            .concurrency(20);

        assert_eq!(tester.get_timeout(), Duration::from_secs(5));
        assert_eq!(tester.get_concurrency(), 20);
    }

    #[test]
    fn test_test_error_display() {
        assert_eq!(
            TestError::Timeout(10).to_string(),
            "Connection timeout after 10 seconds"
        );
        assert_eq!(
            TestError::ConnectionRefused.to_string(),
            "Connection refused"
        );
        assert_eq!(
            TestError::HostUnreachable("example.com".to_string()).to_string(),
            "Host unreachable: example.com"
        );
        assert_eq!(
            TestError::DnsResolutionFailed("invalid.host".to_string()).to_string(),
            "DNS resolution failed: invalid.host"
        );
        assert_eq!(
            TestError::ProtocolError("Invalid banner".to_string()).to_string(),
            "Protocol handshake failed: Invalid banner"
        );
    }
}
