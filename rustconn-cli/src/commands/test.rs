//! Test connection connectivity command.

use std::path::Path;

use crate::cli::OutputFormat;
use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Test connection command handler
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when connections cannot be loaded
/// - [`CliError::ConnectionNotFound`] when no connection matches `name`
///   (and `name` is not the special value `"all"`)
/// - [`CliError::TestFailed`] when the TCP probe fails or the host is unreachable
///   within `timeout` seconds
pub(super) fn cmd_test(
    config_path: Option<&Path>,
    name: &str,
    timeout: u64,
    format: OutputFormat,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    if connections.is_empty() {
        if name.eq_ignore_ascii_case("all") {
            match format {
                OutputFormat::Json => println!("[]"),
                _ => println!("No connections configured."),
            }
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
        let summary = runtime.block_on(tester.test_batch(&connections));

        match format {
            OutputFormat::Json => print_batch_json(&summary)?,
            OutputFormat::Csv => print_batch_csv(&summary),
            OutputFormat::Table => {
                println!("Testing {} connections...\n", connections.len());
                for result in &summary.results {
                    print_test_result_table(result);
                }
                println!();
                print_test_summary_table(&summary);
            }
        }

        if summary.has_failures() {
            return Err(CliError::TestFailed(format!(
                "{} of {} tests failed",
                summary.failed, summary.total
            )));
        }
    } else {
        let connection = find_connection(&connections, name)?;

        let result = runtime.block_on(tester.test_connection(connection));

        match format {
            OutputFormat::Json => print_single_json(&result)?,
            OutputFormat::Csv => print_single_csv(&result),
            OutputFormat::Table => {
                println!("Testing connection '{}'...\n", connection.name);
                print_test_result_table(&result);
            }
        }

        if result.is_failure() {
            return Err(CliError::TestFailed(
                result.error.unwrap_or_else(|| "Unknown error".to_string()),
            ));
        }
    }

    Ok(())
}

/// Print batch test results as JSON.
fn print_batch_json(summary: &rustconn_core::testing::TestSummary) -> Result<(), CliError> {
    let output = serde_json::json!({
        "total": summary.total,
        "passed": summary.passed,
        "failed": summary.failed,
        "pass_rate": summary.pass_rate(),
        "results": summary.results,
    });
    let json = serde_json::to_string_pretty(&output)
        .map_err(|e| CliError::TestFailed(format!("JSON serialization failed: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Print a single test result as JSON.
fn print_single_json(result: &rustconn_core::testing::TestResult) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(result)
        .map_err(|e| CliError::TestFailed(format!("JSON serialization failed: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Print batch test results as CSV.
fn print_batch_csv(summary: &rustconn_core::testing::TestSummary) {
    println!("connection,success,latency_ms,error");
    for result in &summary.results {
        print_csv_row(result);
    }
}

/// Print a single test result as CSV.
fn print_single_csv(result: &rustconn_core::testing::TestResult) {
    println!("connection,success,latency_ms,error");
    print_csv_row(result);
}

/// Print one CSV row for a test result.
fn print_csv_row(result: &rustconn_core::testing::TestResult) {
    let latency = result.latency_ms.map_or(String::new(), |l| l.to_string());
    let error = result.error.as_deref().unwrap_or("");
    println!(
        "{},{},{},{}",
        crate::format::escape_csv_field(&result.connection_name),
        result.success,
        latency,
        crate::format::escape_csv_field(error),
    );
}

/// Print a single test result with colors (table mode).
fn print_test_result_table(result: &rustconn_core::testing::TestResult) {
    use crate::color;

    if result.success {
        print!("{}{}✓{} ", color::green(), color::bold(), color::reset());
        print!("{}", result.connection_name);

        if let Some(latency) = result.latency_ms {
            print!(" {}({latency}ms){}", color::cyan(), color::reset());
        }

        if let Some(protocol) = result.details.get("protocol") {
            print!(" [{protocol}]");
        }

        println!();
    } else {
        print!("{}{}✗{} ", color::red(), color::bold(), color::reset());
        print!("{}", result.connection_name);

        if let Some(ref error) = result.error {
            print!(" {}- {error}{}", color::yellow(), color::reset());
        }

        println!();

        if !result.details.is_empty() {
            for (key, value) in &result.details {
                println!("    {key}: {value}");
            }
        }
    }
}

/// Print the test summary with colors (table mode).
fn print_test_summary_table(summary: &rustconn_core::testing::TestSummary) {
    use crate::color;

    println!("{}Test Summary:{}", color::bold(), color::reset());
    println!("  Total:  {}", summary.total);

    if summary.passed > 0 {
        println!(
            "  {}Passed: {}{}",
            color::green(),
            summary.passed,
            color::reset()
        );
    } else {
        println!("  Passed: {}", summary.passed);
    }

    if summary.failed > 0 {
        println!(
            "  {}Failed: {}{}",
            color::red(),
            summary.failed,
            color::reset()
        );
    } else {
        println!("  Failed: {}", summary.failed);
    }

    let pass_rate = summary.pass_rate();
    if pass_rate >= 100.0 {
        println!(
            "  {}Pass rate: {pass_rate:.1}%{}",
            color::green(),
            color::reset()
        );
    } else if pass_rate >= 50.0 {
        println!("  Pass rate: {pass_rate:.1}%");
    } else {
        println!(
            "  {}Pass rate: {pass_rate:.1}%{}",
            color::red(),
            color::reset()
        );
    }
}
