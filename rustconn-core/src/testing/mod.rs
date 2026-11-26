//! Connection testing module for verifying connectivity.
//!
//! [`ConnectionTester`] picks a probe per connection with [`plan_probe`]: a
//! TCP dial (with an SSH banner check for SSH), an `ssh -W` probe through the
//! connection's jump host, `kubectl` for Kubernetes or `aws ssm` for an AWS
//! Session Manager target. A connection with nothing to probe is
//! [`TestStatus::Skipped`], which is neither a pass nor a failure.

// cast_precision_loss, cast_possible_truncation allowed at workspace level

mod probe;

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::time::timeout;
use uuid::Uuid;

use crate::connection::connect_tcp;
use crate::connection::gateway_failover::{gateway_probes_by, probe_gateways, use_gateway};
use crate::models::Connection;

pub use probe::{
    JumpOutcome, JumpProbe, ProbePlan, TestContext, classify_jump_probe, plan_probe, web_endpoint,
};

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

/// The outcome of one test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    /// The connection answered.
    Ok,
    /// The connection did not answer, or answered with an error.
    Failed,
    /// Nothing was probed; the `skipped` detail says why.
    Skipped,
}

/// Result of testing a single connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// ID of the connection that was tested
    pub connection_id: Uuid,
    /// Name of the connection
    pub connection_name: String,
    /// The outcome
    pub status: TestStatus,
    /// Whether the test passed: true only for [`TestStatus::Ok`]. Kept for
    /// consumers that predate `status`; a skipped test is not a success.
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
            status: TestStatus::Ok,
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
            status: TestStatus::Failed,
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

    /// Creates a result for a connection that was not probed.
    ///
    /// A serial device has no network address, a Zero Trust provider other than
    /// AWS SSM has a resource ID instead of a host, a jump host may need an
    /// interactive login, a client CLI may be missing. The result is
    /// [`TestStatus::Skipped`] with `success: false`, no error, and a `skipped`
    /// detail holding `reason`.
    #[must_use]
    pub fn skipped(connection_id: Uuid, connection_name: String, reason: &str) -> Self {
        Self {
            connection_id,
            connection_name,
            status: TestStatus::Skipped,
            success: false,
            latency_ms: None,
            error: None,
            details: HashMap::new(),
        }
        .with_detail("skipped", reason)
    }

    /// Returns true if the connection was not probed (see [`Self::skipped`])
    #[must_use]
    pub const fn is_skipped(&self) -> bool {
        matches!(self.status, TestStatus::Skipped)
    }

    /// The reason a skipped test gives, if it was skipped.
    #[must_use]
    pub fn skip_reason(&self) -> Option<&str> {
        self.details.get("skipped").map(String::as_str)
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
        matches!(self.status, TestStatus::Ok)
    }

    /// Returns true if the test failed; a skipped test did not
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        matches!(self.status, TestStatus::Failed)
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
    /// Number of connections that were not probed; neither passed nor failed
    #[serde(default)]
    pub skipped: usize,
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
            skipped: 0,
            results: Vec::new(),
        }
    }

    /// Creates a test summary from a list of results
    #[must_use]
    pub fn from_results(results: Vec<TestResult>) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.is_success()).count();
        let skipped = results.iter().filter(|r| r.is_skipped()).count();
        let failed = total - passed - skipped;

        Self {
            total,
            passed,
            failed,
            skipped,
            results,
        }
    }

    /// Adds a test result to the summary
    pub fn add_result(&mut self, result: TestResult) {
        match result.status {
            TestStatus::Ok => self.passed += 1,
            TestStatus::Failed => self.failed += 1,
            TestStatus::Skipped => self.skipped += 1,
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

    /// Returns the pass rate as a percentage (0.0 to 100.0) of the tests
    /// that ran; skipped ones are left out
    #[must_use]
    pub fn pass_rate(&self) -> f64 {
        let ran = self.passed + self.failed;
        if ran == 0 {
            return 100.0;
        }
        (self.passed as f64 / ran as f64) * 100.0
    }

    /// Returns a summary string
    #[must_use]
    pub fn summary_string(&self) -> String {
        format!(
            "Total: {}, Passed: {}, Failed: {}, Skipped: {} ({:.1}% pass rate)",
            self.total,
            self.passed,
            self.failed,
            self.skipped,
            self.pass_rate()
        )
    }

    /// Returns only the failed results
    #[must_use]
    pub fn failed_results(&self) -> Vec<&TestResult> {
        self.results.iter().filter(|r| r.is_failure()).collect()
    }

    /// Returns only the successful results
    #[must_use]
    pub fn successful_results(&self) -> Vec<&TestResult> {
        self.results.iter().filter(|r| r.is_success()).collect()
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

    /// Tests a single connection on its own, with no other connections or
    /// groups to resolve a jump host from.
    ///
    /// A connection editor that tests an unsaved connection uses this; see
    /// [`Self::test_connection_in`] for the full resolution.
    pub async fn test_connection(&self, connection: &Connection) -> TestResult {
        let network = crate::config::NetworkSettings::default();
        let ctx = TestContext {
            connections: &[],
            groups: &[],
            network: &network,
        };
        self.test_connection_in(connection, &ctx).await
    }

    /// Tests a single connection, reaching it the way a session would.
    ///
    /// The probe comes from [`plan_probe`]: through the connection's jump host
    /// (its own, a group's or the global one, after failing over to a
    /// reachable fallback), with `kubectl` for Kubernetes, with `aws ssm` for
    /// an AWS Session Manager target, otherwise a TCP dial, plus the banner
    /// exchange for SSH.
    pub async fn test_connection_in(
        &self,
        connection: &Connection,
        ctx: &TestContext<'_>,
    ) -> TestResult {
        let routed = self.with_reachable_gateway(connection, ctx).await;
        let id = connection.id;
        let name = connection.name.clone();
        let protocol = connection.protocol.to_string();
        let result = match plan_probe(&routed, ctx, self.timeout.as_secs()) {
            ProbePlan::Skip(reason) => TestResult::skipped(id, name, &reason),
            ProbePlan::Invalid(reason) => {
                TestResult::from_error(id, name, &TestError::InvalidConfig(reason))
            }
            ProbePlan::Tcp {
                host,
                port,
                ssh_banner,
            } => self.probe_tcp(id, name, &host, port, ssh_banner).await,
            ProbePlan::ThroughJump(probe) => self.probe_through_jump(id, name, &probe).await,
            ProbePlan::Kubectl { args } => self.probe_kubectl(id, name, &args).await,
            ProbePlan::AwsSsm { args } => self.probe_aws_ssm(id, name, &args).await,
        };
        result.with_detail("protocol", protocol)
    }

    /// A copy of `connection` routed through the first reachable of its
    /// gateways, when it has fallback jump hosts, as `run-script` does.
    async fn with_reachable_gateway(
        &self,
        connection: &Connection,
        ctx: &TestContext<'_>,
    ) -> Connection {
        let mut routed = connection.clone();
        let probes = gateway_probes_by(
            connection,
            |id| ctx.connections.iter().find(|c| c.id == id),
            ctx.groups,
            ctx.network,
        );
        if probes.is_empty() {
            return routed;
        }
        let secs = u32::try_from(self.timeout.as_secs())
            .unwrap_or(u32::MAX)
            .max(1);
        if let Ok(Some(selection)) =
            tokio::task::spawn_blocking(move || probe_gateways(&probes, secs)).await
        {
            use_gateway(&mut routed, selection.gateway_id);
        }
        routed
    }

    /// A TCP dial, and for SSH the banner exchange.
    async fn probe_tcp(
        &self,
        id: Uuid,
        name: String,
        host: &str,
        port: u16,
        ssh_banner: bool,
    ) -> TestResult {
        let start = std::time::Instant::now();
        match self.test_port(host, port).await {
            Ok(latency) => {
                let latency_ms = latency.as_millis() as u64;
                if !ssh_banner {
                    return TestResult::success(id, name, latency_ms);
                }
                match self.ssh_banner(host, port).await {
                    Ok(()) => TestResult::success(id, name, latency_ms)
                        .with_detail("handshake", "verified"),
                    Err(e) => TestResult::from_error(id, name, &e).with_detail("port_open", "true"),
                }
            }
            Err(e) => {
                let elapsed = start.elapsed().as_millis() as u64;
                TestResult::from_error(id, name, &e).with_detail("elapsed_ms", elapsed.to_string())
            }
        }
    }

    /// `ssh -W` from the jump host nearest the target.
    async fn probe_through_jump(&self, id: Uuid, name: String, probe: &JumpProbe) -> TestResult {
        let Some(ssh) = crate::which::find_in_path("ssh") else {
            return TestResult::skipped(
                id,
                name,
                "ssh is not installed, so the jump host cannot be used",
            )
            .with_detail("jump_host", &probe.jump_host);
        };
        let start = std::time::Instant::now();
        let (exit_code, stderr) = match self.run_jump_ssh(ssh, &probe.args).await {
            Ok(run) => run,
            Err(e) => {
                return TestResult::from_error(id, name, &e)
                    .with_detail("jump_host", &probe.jump_host);
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        let jump = probe.jump_host.as_str();
        let target = probe.target.as_str();
        let result = match classify_jump_probe(exit_code, &stderr) {
            JumpOutcome::Reachable => TestResult::success(id, name, latency_ms)
                .with_detail("jump_host_status", "ok")
                .with_detail("target_status", "ok"),
            JumpOutcome::TargetFailed(reason) => TestResult::failure(
                id,
                name,
                format!("Target {target} unreachable from jump host {jump}: {reason}"),
            )
            .with_detail("jump_host_status", "ok")
            .with_detail("target_status", "failed"),
            JumpOutcome::JumpFailed(reason) => {
                TestResult::failure(id, name, format!("Jump host {jump} unreachable: {reason}"))
                    .with_detail("jump_host_status", "failed")
            }
            JumpOutcome::JumpNeedsInteraction(reason) => TestResult::skipped(
                id,
                name,
                &format!(
                    "jump host {jump} needs an interactive login ({reason}); \
                     the test logs in with keys or the agent only"
                ),
            )
            .with_detail("jump_host_status", "skipped"),
            JumpOutcome::TimedOut { authenticated } => {
                let secs = self.timeout.as_secs();
                if authenticated {
                    TestResult::failure(
                        id,
                        name,
                        format!("Target {target} did not answer jump host {jump} within {secs} s"),
                    )
                    .with_detail("jump_host_status", "ok")
                    .with_detail("target_status", "failed")
                } else {
                    TestResult::failure(
                        id,
                        name,
                        format!("Jump host {jump} did not answer within {secs} s"),
                    )
                    .with_detail("jump_host_status", "failed")
                }
            }
        };
        result
            .with_detail("jump_host", jump)
            .with_detail("target", target)
    }

    /// Runs the jump probe's `ssh` until it exits or the test timeout passes.
    /// Returns its exit code (`None` at the timeout) and what it wrote to
    /// standard error so far.
    async fn run_jump_ssh(
        &self,
        ssh: std::path::PathBuf,
        args: &[String],
    ) -> TestResult2<(Option<i32>, String)> {
        let mut command = tokio::process::Command::new(ssh);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| TestError::IoError(e.to_string()))?;
        let mut stderr_pipe = child.stderr.take();
        let mut stderr = Vec::new();
        let exit_code = {
            let read = async {
                if let Some(pipe) = stderr_pipe.as_mut() {
                    // A read error leaves what was read; the exit code decides.
                    pipe.read_to_end(&mut stderr).await.ok();
                }
            };
            tokio::pin!(read);
            let wait = child.wait();
            tokio::pin!(wait);
            let deadline = tokio::time::sleep(self.timeout);
            tokio::pin!(deadline);
            let mut read_done = false;
            let status = loop {
                tokio::select! {
                    () = &mut read, if !read_done => read_done = true,
                    status = &mut wait => break status.ok(),
                    () = &mut deadline => break None,
                }
            };
            // A ProxyCommand child can hold stderr open after ssh exits.
            if status.is_some() && !read_done {
                timeout(Duration::from_millis(500), &mut read).await.ok();
            }
            // A signal has no code; read it as a failed run, not a timeout.
            status.map(|s| s.code().unwrap_or(-1))
        };
        if exit_code.is_none() {
            child.start_kill().ok();
        }
        Ok((exit_code, String::from_utf8_lossy(&stderr).into_owned()))
    }

    /// `kubectl get pod`, or `kubectl auth can-i create pods` for busybox mode.
    async fn probe_kubectl(&self, id: Uuid, name: String, args: &[String]) -> TestResult {
        let start = std::time::Instant::now();
        match self.run_tool("kubectl", args).await {
            ToolRun::Missing => TestResult::skipped(id, name, "kubectl is not installed"),
            ToolRun::Failed(error) => TestResult::from_error(id, name, &error),
            ToolRun::Exited { code: 0, .. } => {
                TestResult::success(id, name, start.elapsed().as_millis() as u64)
            }
            ToolRun::Exited { stdout, stderr, .. } => {
                let reason = first_line(&stderr)
                    .or_else(|| first_line(&stdout))
                    .unwrap_or("kubectl failed");
                // `auth can-i` answers `no` on stdout with exit status 1.
                let reason = if reason == "no" {
                    "not allowed to create pods"
                } else {
                    reason
                };
                TestResult::failure(id, name, format!("kubectl: {reason}"))
            }
        }
    }

    /// `aws ssm describe-instance-information`, reading the `PingStatus`.
    async fn probe_aws_ssm(&self, id: Uuid, name: String, args: &[String]) -> TestResult {
        let start = std::time::Instant::now();
        match self.run_tool("aws", args).await {
            ToolRun::Missing => TestResult::skipped(id, name, "the aws CLI is not installed"),
            ToolRun::Failed(error) => TestResult::from_error(id, name, &error),
            ToolRun::Exited {
                code: 0, stdout, ..
            } => match stdout.trim() {
                "Online" => TestResult::success(id, name, start.elapsed().as_millis() as u64)
                    .with_detail("ping_status", "Online"),
                "" | "None" => TestResult::failure(
                    id,
                    name,
                    "The instance is not registered with AWS Systems Manager",
                ),
                status => TestResult::failure(id, name, format!("SSM agent PingStatus: {status}"))
                    .with_detail("ping_status", status),
            },
            ToolRun::Exited { stderr, .. } => TestResult::failure(
                id,
                name,
                format!("aws: {}", first_line(&stderr).unwrap_or("command failed")),
            ),
        }
    }

    /// Runs `program` with `args`, no input, under the test timeout.
    async fn run_tool(&self, program: &str, args: &[String]) -> ToolRun {
        let Some(path) = crate::which::find_in_path(program) else {
            return ToolRun::Missing;
        };
        let mut command = tokio::process::Command::new(path);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => return ToolRun::Failed(TestError::IoError(e.to_string())),
        };
        match timeout(self.timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => ToolRun::Exited {
                code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
            Ok(Err(e)) => ToolRun::Failed(TestError::IoError(e.to_string())),
            Err(_) => ToolRun::Failed(TestError::Timeout(self.timeout.as_secs())),
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
        let start = std::time::Instant::now();

        let timeout_secs = self.timeout.as_secs();

        match timeout(self.timeout, connect_tcp(host, port, false)).await {
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
        self.ssh_banner(&connection.host, connection.port).await
    }

    /// The banner exchange of [`Self::test_ssh`] with `host:port`.
    async fn ssh_banner(&self, host: &str, port: u16) -> TestResult2<()> {
        let dial = connect_tcp(host, port, false);
        let stream = match timeout(self.timeout, dial).await {
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

    /// Tests `targets` concurrently, each with [`Self::test_connection_in`].
    pub async fn test_batch_in(
        &self,
        targets: &[Connection],
        ctx: &TestContext<'_>,
    ) -> TestSummary {
        use futures::stream::{self, StreamExt};

        let results: Vec<TestResult> = stream::iter(targets)
            .map(|conn| self.test_connection_in(conn, ctx))
            .buffer_unordered(self.concurrency)
            .collect()
            .await;

        TestSummary::from_results(results)
    }
}

/// How a client CLI run by a probe ended.
enum ToolRun {
    /// The program is not installed.
    Missing,
    /// It could not be started, or did not finish in time.
    Failed(TestError),
    /// It exited.
    Exited {
        code: i32,
        stdout: String,
        stderr: String,
    },
}

/// The first non-empty line of `text`, trimmed.
fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
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

    #[tokio::test]
    async fn zero_trust_and_serial_are_skipped_not_probed() {
        let tester = ConnectionTester::with_timeout(Duration::from_secs(1));

        let serial = Connection::new_serial("console".to_string(), "/dev/ttyUSB0".to_string());
        let result = tester.test_connection(&serial).await;
        assert!(result.is_skipped() && !result.is_success() && !result.is_failure());
        assert!(!result.success);
        assert_eq!(result.status, TestStatus::Skipped);
        assert!(result.error.is_none());
        assert_eq!(
            result.skip_reason(),
            Some("serial device, no network address to probe")
        );
    }

    #[test]
    fn a_skipped_result_is_neither_passed_nor_failed_in_a_summary() {
        let summary = TestSummary::from_results(vec![
            TestResult::success(Uuid::new_v4(), "ok".to_string(), 5),
            TestResult::failure(Uuid::new_v4(), "bad".to_string(), "Error"),
            TestResult::skipped(Uuid::new_v4(), "ssm".to_string(), "why"),
        ]);
        assert_eq!(
            (
                summary.total,
                summary.passed,
                summary.failed,
                summary.skipped
            ),
            (3, 1, 1, 1)
        );
        assert!((summary.pass_rate() - 50.0).abs() < 0.01);

        let mut incremental = TestSummary::new();
        for result in summary.results {
            incremental.add_result(result);
        }
        assert_eq!(incremental.skipped, 1);
        assert_eq!(incremental.failed, 1);

        let only_skipped = TestSummary::from_results(vec![TestResult::skipped(
            Uuid::new_v4(),
            "s".to_string(),
            "why",
        )]);
        assert!(only_skipped.all_passed() && !only_skipped.has_failures());
    }

    #[test]
    fn status_serializes_in_lowercase() {
        let result = TestResult::skipped(Uuid::nil(), "s".to_string(), "why");
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "skipped");
        assert_eq!(json["success"], false);
    }
}
