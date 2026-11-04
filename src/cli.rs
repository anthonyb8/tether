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
    Share {
        /// The local port a dev server listens on
        #[arg(value_parser = port)]
        port: u16,
        /// Put it on the internet with Tailscale Funnel, on the first free of
        /// HTTPS ports 443, 8443 and 10000
        #[arg(long)]
        public: bool,
    },
    /// Stop sharing a port, with the tailnet and publicly
    Unshare {
        #[arg(value_parser = port)]
        port: u16,
    },
    /// List the ports being shared
    Shares,
    /// Print the HTTPS address a port is reached at once shared, for start scripts
    Origin {
        #[arg(value_parser = port)]
        port: u16,
    },
    /// Connect this laptop to the server in the background, from now and at every login
    Connect {
        /// The server's SSH host (default: TETHER_HOST, the config file, else "server")
        host: Option<String>,
        /// Run in this terminal instead, until Ctrl-C
        #[arg(long, conflicts_with = "stop")]
        foreground: bool,
        /// Disconnect, and stop connecting at login
        #[arg(long, conflicts_with = "host")]
        stop: bool,
    },
    /// Carry requests from the server to the connected laptop (started by connect)
    Relay {
        /// The laptop's name, shown by status
        #[arg(long, default_value = "laptop")]
        from: String,
    },
    /// Open a URL in the connected laptop's browser
    Open {
        /// An http or https URL, such as http://localhost:5173
        url: String,
    },
    /// Show a desktop notification on the connected laptop
    Notify {
        /// The message; several words are joined with spaces
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// Show whether a laptop is connected, from where and since when
    Status,
    /// Open a browser on the laptop whose localhost is the server's
    Browser {
        /// The server's SSH host (default: TETHER_HOST, the config file, else "server")
        host: Option<String>,
    },
}

/// A TCP port to share: 1 to 65535.
fn port(arg: &str) -> Result<u16, String> {
    match arg.parse::<u16>() {
        Ok(0) | Err(_) => Err(format!("{arg} is not a port (1 to 65535)")),
        Ok(port) => Ok(port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_must_be_1_to_65535() {
        for bad in ["0", "65536", "-1", "http"] {
            assert!(
                Cli::try_parse_from(["tether", "share", bad]).is_err(),
                "{bad} was accepted"
            );
        }
        assert!(Cli::try_parse_from(["tether", "share", "65535"]).is_ok());
    }
}
