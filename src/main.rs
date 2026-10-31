mod browser;
mod cli;
mod config;
mod connect;
mod laptop;
mod protocol;
mod relay;
mod share;
mod spawn;
mod tailscale;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};

/// The exit code of a subcommand that exists in the CLI but has no behaviour yet.
const NOT_BUILT: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Share { port } => share::share(port),
        Command::Unshare { port } => share::unshare(port),
        Command::Shares => share::shares(),
        Command::Origin { port } => share::origin(port),
        Command::Connect { host } => connect::connect(host),
        Command::Relay { from } => relay::relay(from),
        Command::Open { url } => relay::open(url),
        Command::Status => relay::status(),
        Command::Browser { host } => browser::browser(host),
        command => {
            eprintln!("tether: {} is not built yet", command.name());
            return ExitCode::from(NOT_BUILT);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("tether: {err:#}");
            ExitCode::FAILURE
        }
    }
}
