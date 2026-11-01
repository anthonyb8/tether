mod browser;
mod cli;
mod config;
mod connect;
mod laptop;
mod protocol;
mod relay;
mod service;
mod share;
mod spawn;
mod state;
mod tailscale;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Share { port } => share::share(port),
        Command::Unshare { port } => share::unshare(port),
        Command::Shares => share::shares(),
        Command::Origin { port } => share::origin(port),
        Command::Connect { stop: true, .. } => service::stop(),
        Command::Connect {
            host,
            foreground: true,
            ..
        } => connect::connect(host),
        Command::Connect { host, .. } => service::start(host),
        Command::Relay { from } => relay::relay(from),
        Command::Open { url } => relay::open(url),
        Command::Status => relay::status(),
        Command::Browser { host } => browser::browser(host),
        Command::Notify { text } => relay::notify(text.join(" ")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("tether: {err:#}");
            ExitCode::FAILURE
        }
    }
}
