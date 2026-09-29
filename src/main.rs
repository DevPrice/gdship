use std::process::ExitCode;

use gdship::cli::Cli;
use gdship::report::Reporter;
use gdship::{Outcome, UsageError, run};

fn main() -> ExitCode {
    let cli = Cli::parse_args();
    let reporter = Reporter::from_env();
    match run(cli, reporter) {
        Ok(Outcome::Success) => ExitCode::SUCCESS,
        Ok(Outcome::Failure) => ExitCode::FAILURE,
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
