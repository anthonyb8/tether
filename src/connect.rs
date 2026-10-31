//! `connect`: the laptop's one SSH connection to the server, carrying a SOCKS
//! proxy whose `localhost` is the server's, kept up across sleep, network
//! changes and server restarts.

use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::config::Settings;

/// A connection that stayed up this long was healthy, so the next drop starts
/// the backoff from the bottom again.
const STABLE: Duration = Duration::from_secs(30);

pub fn connect(host: Option<String>) -> Result<()> {
    let settings = Settings::load(host)?;
    let mut backoff = Backoff::default();
    loop {
        ensure_port_free(settings.port)?;
        eprintln!(
            "tether: connecting to {}, SOCKS proxy on 127.0.0.1:{}",
            settings.host, settings.port
        );
        let started = Instant::now();
        let status = ssh(&settings)
            .status()
            .context("could not run ssh; is OpenSSH installed?")?;
        if started.elapsed() >= STABLE {
            backoff = Backoff::default();
        }
        let wait = backoff.next_wait();
        eprintln!(
            "tether: connection to {} ended ({status}); reconnecting in {}s",
            settings.host,
            wait.as_secs()
        );
        thread::sleep(wait);
    }
}

/// The ssh that carries the proxy. Keepalives notice a dead link (a laptop
/// waking from sleep, a dropped Wi-Fi) within about 30 seconds, and
/// ExitOnForwardFailure makes a proxy that cannot listen fail rather than
/// connect without it. BatchMode keeps a run with no terminal, such as a user
/// service, from hanging on a password prompt.
fn ssh(settings: &Settings) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.arg("-N")
        .args(["-D", &format!("127.0.0.1:{}", settings.port)])
        .args(["-o", "ExitOnForwardFailure=yes"])
        .args(["-o", "ServerAliveInterval=10"])
        .args(["-o", "ServerAliveCountMax=3"])
        .args(["-o", "ConnectTimeout=10"])
        .args(["-o", "BatchMode=yes"])
        .arg(&settings.host)
        .stdin(Stdio::null());
    cmd
}

/// Refuses a proxy port something else holds, most often another `tether
/// connect`, rather than retrying a forward that can never succeed.
fn ensure_port_free(port: u16) -> Result<()> {
    if TcpListener::bind(("127.0.0.1", port)).is_err() {
        bail!(
            "127.0.0.1:{port} is in use, likely by another tether connect; stop it or set TETHER_PORT"
        );
    }
    Ok(())
}

/// Waits between reconnects: 1 s, doubling to a 30 s ceiling.
#[derive(Default)]
struct Backoff {
    attempts: u32,
}

impl Backoff {
    const CEILING: Duration = Duration::from_secs(30);

    fn next_wait(&mut self) -> Duration {
        let wait = Duration::from_secs(1 << self.attempts.min(5)).min(Self::CEILING);
        self.attempts += 1;
        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_to_a_ceiling() {
        let mut backoff = Backoff::default();
        let waits: Vec<u64> = (0..8).map(|_| backoff.next_wait().as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30, 30]);
    }
}
