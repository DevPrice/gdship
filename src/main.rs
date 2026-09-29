use std::process::ExitCode;

use gdship::cli::Cli;
use gdship::report::Reporter;
use gdship::{UsageError, run};

fn main() -> ExitCode {
    let cli = Cli::parse_args();
    let reporter = Reporter::from_env();
    match run(cli, reporter) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            reporter.error(format!("{err:#}"));
            if err.is::<UsageError>() {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}
