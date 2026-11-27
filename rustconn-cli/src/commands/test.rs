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
/// - [`CliError::TestFailed`] when a probe fails or the host is unreachable
///   within `timeout` seconds. A skipped connection (nothing to probe, or a
///   jump host that needs an interactive login) is not a failure.
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

    // Groups are mandatory here, unlike settings: without them a group's
    // inherited jump host is lost and the connection would be dialled directly,
    // reporting a false failure (or a false pass) instead of a config error.
    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;
    let settings = config_manager.load_settings().unwrap_or_default();
    let ctx = rustconn_core::testing::TestContext {
        connections: &connections,
        groups: &groups,
        network: &settings.network,
    };

    let tester = rustconn_core::testing::ConnectionTester::with_timeout(
        std::time::Duration::from_secs(timeout),
    );

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::TestFailed(format!("Failed to create async runtime: {e}")))?;

    if name.eq_ignore_ascii_case("all") {
        let summary = runtime.block_on(tester.test_batch_in(&connections, &ctx));

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

        let result = runtime.block_on(tester.test_connection_in(connection, &ctx));

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
        "skipped": summary.skipped,
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
    println!("{CSV_HEADER}");
    for result in &summary.results {
        print_csv_row(result);
    }
}

/// Print a single test result as CSV.
fn print_single_csv(result: &rustconn_core::testing::TestResult) {
    println!("{CSV_HEADER}");
    print_csv_row(result);
}

/// Print one CSV row for a test result.
fn print_csv_row(result: &rustconn_core::testing::TestResult) {
    println!("{}", csv_row(result));
}

/// The CSV columns. `status` and `skip_reason` come last so that scripts
/// reading the first four keep working.
const CSV_HEADER: &str = "connection,success,latency_ms,error,status,skip_reason";

/// One CSV row for a test result.
fn csv_row(result: &rustconn_core::testing::TestResult) -> String {
    let latency = result.latency_ms.map_or(String::new(), |l| l.to_string());
    let error = result.error.as_deref().unwrap_or("");
    format!(
        "{},{},{},{},{},{}",
        crate::format::escape_csv_field(&result.connection_name),
        result.success,
        latency,
        crate::format::escape_csv_field(error),
        status_word(result.status),
        crate::format::escape_csv_field(result.skip_reason().unwrap_or("")),
    )
}

/// `ok`, `failed` or `skipped`, as in the JSON `status` field.
const fn status_word(status: rustconn_core::testing::TestStatus) -> &'static str {
    use rustconn_core::testing::TestStatus;
    match status {
        TestStatus::Ok => "ok",
        TestStatus::Failed => "failed",
        TestStatus::Skipped => "skipped",
    }
}

/// Print a single test result with colors (table mode).
fn print_test_result_table(result: &rustconn_core::testing::TestResult) {
    use crate::color;

    if result.is_skipped() {
        print!("{}{}-{} ", color::yellow(), color::bold(), color::reset());
        print!("{}", result.connection_name);
        if let Some(protocol) = result.details.get("protocol") {
            print!(" [{protocol}]");
        }
        let reason = result.skip_reason().unwrap_or("");
        println!(" {}SKIPPED: {reason}{}", color::yellow(), color::reset());
    } else if result.is_success() {
        print!("{}{}✓{} ", color::green(), color::bold(), color::reset());
        print!("{}", result.connection_name);

        if let Some(latency) = result.latency_ms {
            print!(" {}({latency}ms){}", color::cyan(), color::reset());
        }

        if let Some(protocol) = result.details.get("protocol") {
            print!(" [{protocol}]");
        }
        if let Some(jump) = result.details.get("jump_host") {
            print!(" via {jump}");
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

    if summary.skipped > 0 {
        println!(
            "  {}Skipped: {}{}",
            color::yellow(),
            summary.skipped,
            color::reset()
        );
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

#[cfg(test)]
mod tests {
    use rustconn_core::testing::TestResult;
    use uuid::Uuid;

    use super::{CSV_HEADER, csv_row};

    #[test]
    fn csv_marks_a_skip_apart_from_a_pass_and_a_failure() {
        assert_eq!(CSV_HEADER.split(',').count(), 6);
        let skipped = TestResult::skipped(Uuid::nil(), "ssm".to_string(), "no aws, CLI");
        assert_eq!(csv_row(&skipped), "ssm,false,,,skipped,\"no aws, CLI\"");
        let ok = TestResult::success(Uuid::nil(), "web".to_string(), 12);
        assert_eq!(csv_row(&ok), "web,true,12,,ok,");
        let failed = TestResult::failure(Uuid::nil(), "rdp".to_string(), "Connection refused");
        assert_eq!(csv_row(&failed), "rdp,false,,Connection refused,failed,");
    }
}
