//! Test connection connectivity command.

use std::path::Path;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Test connection command handler
pub fn cmd_test(config_path: Option<&Path>, name: &str, timeout: u64) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    if connections.is_empty() {
        if name.eq_ignore_ascii_case("all") {
            println!("No connections configured.");
            return Ok(());
        }
        return Err(CliError::ConnectionNotFound(name.to_string()));
    }

    let tester = rustconn_core::testing::ConnectionTester::with_timeout(
        std::time::Duration::from_secs(timeout),
    );

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::TestFailed(format!("Failed to create async runtime: {e}")))?;

    if name.eq_ignore_ascii_case("all") {
        println!("Testing {} connections...\n", connections.len());

        let summary = runtime.block_on(tester.test_batch(&connections));

        for result in &summary.results {
            print_test_result(result);
        }

        println!();
        print_test_summary(&summary);

        if summary.has_failures() {
            return Err(CliError::TestFailed(format!(
                "{} of {} tests failed",
                summary.failed, summary.total
            )));
        }
    } else {
        let connection = find_connection(&connections, name)?;

        println!("Testing connection '{}'...\n", connection.name);

        let result = runtime.block_on(tester.test_connection(connection));
        print_test_result(&result);

        if result.is_failure() {
            return Err(CliError::TestFailed(
                result.error.unwrap_or_else(|| "Unknown error".to_string()),
            ));
        }
    }

    Ok(())
}

/// Print a single test result with colors
fn print_test_result(result: &rustconn_core::testing::TestResult) {
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const YELLOW: &str = "\x1b[33m";
    const CYAN: &str = "\x1b[36m";
    const RESET: &str = "\x1b[0m";
    const BOLD: &str = "\x1b[1m";

    if result.success {
        print!("{GREEN}{BOLD}✓{RESET} ");
        print!("{}", result.connection_name);

        if let Some(latency) = result.latency_ms {
            print!(" {CYAN}({latency}ms){RESET}");
        }

        if let Some(protocol) = result.details.get("protocol") {
            print!(" [{protocol}]");
        }

        println!();
    } else {
        print!("{RED}{BOLD}✗{RESET} ");
        print!("{}", result.connection_name);

        if let Some(ref error) = result.error {
            print!(" {YELLOW}- {error}{RESET}");
        }

        println!();

        if !result.details.is_empty() {
            for (key, value) in &result.details {
                println!("    {key}: {value}");
            }
        }
    }
}

/// Print the test summary with colors
fn print_test_summary(summary: &rustconn_core::testing::TestSummary) {
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const RESET: &str = "\x1b[0m";
    const BOLD: &str = "\x1b[1m";

    println!("{BOLD}Test Summary:{RESET}");
    println!("  Total:  {}", summary.total);

    if summary.passed > 0 {
        println!("  {GREEN}Passed: {}{RESET}", summary.passed);
    } else {
        println!("  Passed: {}", summary.passed);
    }

    if summary.failed > 0 {
        println!("  {RED}Failed: {}{RESET}", summary.failed);
    } else {
        println!("  Failed: {}", summary.failed);
    }

    let pass_rate = summary.pass_rate();
    if pass_rate >= 100.0 {
        println!("  {GREEN}Pass rate: {pass_rate:.1}%{RESET}");
    } else if pass_rate >= 50.0 {
        println!("  Pass rate: {pass_rate:.1}%");
    } else {
        println!("  {RED}Pass rate: {pass_rate:.1}%{RESET}");
    }
}
