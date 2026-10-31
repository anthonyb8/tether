mod cli;

use std::process::ExitCode;

use clap::Parser;

use cli::Cli;

/// The exit code of a subcommand that exists in the CLI but has no behaviour yet.
const NOT_BUILT: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();
    eprintln!("tether: {} is not built yet", cli.command.name());
    ExitCode::from(NOT_BUILT)
}
