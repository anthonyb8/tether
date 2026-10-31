//! The command line: every subcommand tether has, and its arguments.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "tether", version, about, propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Share a local port with your tailnet over HTTPS and print its address
    Share,
    /// Stop sharing a port
    Unshare,
    /// List the ports being shared
    Shares,
    /// Print the HTTPS address a shared port is reached at
    Origin,
    /// Connect this laptop to the server and serve its requests
    Connect,
    /// Carry requests from the server to the connected laptop (started by connect)
    Relay,
    /// Open a URL in the connected laptop's browser
    Open,
    /// Show a desktop notification on the connected laptop
    Notify,
    /// Show whether a laptop is connected, from where and since when
    Status,
    /// Open a browser on the laptop whose localhost is the server's
    Browser,
}

impl Command {
    /// The name the user typed for this subcommand.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Share => "share",
            Command::Unshare => "unshare",
            Command::Shares => "shares",
            Command::Origin => "origin",
            Command::Connect => "connect",
            Command::Relay => "relay",
            Command::Open => "open",
            Command::Notify => "notify",
            Command::Status => "status",
            Command::Browser => "browser",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBCOMMANDS: [&str; 10] = [
        "share", "unshare", "shares", "origin", "connect", "relay", "open", "notify", "status",
        "browser",
    ];

    #[test]
    fn every_subcommand_parses_and_reports_its_own_name() {
        for name in SUBCOMMANDS {
            let cli = Cli::try_parse_from(["tether", name]).unwrap();
            assert_eq!(cli.command.name(), name);
        }
    }

    #[test]
    fn an_unknown_subcommand_is_refused() {
        assert!(Cli::try_parse_from(["tether", "shar"]).is_err());
    }
}
