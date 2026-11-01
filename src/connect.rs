//! `connect`: the laptop's one SSH connection to the server. It carries a
//! SOCKS proxy whose `localhost` is the server's, and the server's relay,
//! whose requests (open a tab, show a notification) the laptop acts on. It is
//! kept up across sleep, network changes and server restarts.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::config::Settings;
use crate::laptop;
use crate::protocol::Envelope;
use crate::state::{self, State};

/// A connection that stayed up this long was healthy, so the next drop starts
/// the backoff from the bottom again.
const STABLE: Duration = Duration::from_secs(30);

/// The exit status a shell gives a command it cannot find: here, a server
/// without tether installed.
const NOT_FOUND: i32 = 127;

/// The running ssh's pid, for the signal handler; 0 between connections.
static SSH_PID: AtomicI32 = AtomicI32::new(0);

pub fn connect(host: Option<String>) -> Result<()> {
    let settings = Settings::load(host)?;
    let me = laptop::hostname();
    stop_ssh_with_us();
    let mut state = State {
        pid: std::process::id(),
        host: settings.host.clone(),
        port: settings.port,
        background: state::in_background(),
        connected_since: None,
    };
    let mut relay = true;
    let mut backoff = Backoff::default();
    loop {
        ensure_port_free(settings.port)?;
        eprintln!(
            "tether: connecting to {}, SOCKS proxy on 127.0.0.1:{}",
            settings.host, settings.port
        );
        state.connected_since = None;
        state::write(&state);
        let started = Instant::now();
        let status = session(&settings, relay.then_some(me.as_str()), &state)?;
        state.connected_since = None;
        state::write(&state);
        if relay && status.code() == Some(NOT_FOUND) {
            // Keep the proxy, which needs nothing on the server, rather than
            // lose it too.
            eprintln!(
                "tether: tether is not installed on {}, so it cannot open tabs here; proxy only until tether connect is restarted",
                settings.host
            );
            relay = false;
            continue;
        }
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

/// One ssh connection, from start to end. With `relay` (this laptop's
/// name), the server's relay runs over it and its requests are served.
fn session(settings: &Settings, relay: Option<&str>, state: &State) -> Result<ExitStatus> {
    let mut cmd = match relay {
        // A non-interactive SSH session often lacks the PATH entries
        // install.sh installs to, so name them.
        Some(me) => ssh(
            settings,
            Some(&format!(
                "sh -c 'PATH=\"$HOME/.local/bin:/usr/local/bin:$PATH\" exec tether relay --from {me}'"
            )),
        ),
        None => ssh(settings, None),
    };
    // The relay talks over ssh's stdin and stdout; without it, ssh needs
    // neither.
    if relay.is_some() {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let mut child = cmd
        .spawn()
        .context("could not run ssh; is OpenSSH installed?")?;
    SSH_PID.store(child.id() as i32, Ordering::SeqCst);

    // The proxy listening is the sign the connection is up: ssh opens it
    // only once it has logged in.
    let ended = Arc::new(AtomicBool::new(false));
    let watcher = {
        let ended = Arc::clone(&ended);
        let mut state = state.clone();
        thread::spawn(move || {
            while !ended.load(Ordering::SeqCst) {
                if proxy_up(state.port) {
                    state.connected_since = Some(state::now());
                    state::write(&state);
                    return;
                }
                thread::sleep(Duration::from_millis(250));
            }
        })
    };

    if relay.is_some() {
        serve(&mut child, settings)?;
    }
    let status = child.wait()?;
    SSH_PID.store(0, Ordering::SeqCst);
    ended.store(true, Ordering::SeqCst);
    let _ = watcher.join();
    Ok(status)
}

/// Acts on each request the server's relay sends until the connection ends.
fn serve(child: &mut Child, settings: &Settings) -> Result<()> {
    let mut to_server = child.stdin.take().unwrap();
    let from_server = BufReader::new(child.stdout.take().unwrap());
    for line in from_server.lines() {
        let Ok(line) = line else { break };
        // Anything else on stdout, such as a shell profile's greeting, is not
        // for tether.
        let Ok(envelope) = serde_json::from_str::<Envelope>(&line) else {
            continue;
        };
        let mut ack = laptop::handle(envelope.request, settings);
        ack.id = Some(envelope.id);
        let sent = writeln!(to_server, "{}", serde_json::to_string(&ack)?)
            .and_then(|()| to_server.flush());
        if sent.is_err() {
            break;
        }
    }
    Ok(())
}

/// Makes a signal that ends connect (Ctrl-C, a closed terminal, kill, the
/// service stopping) end its ssh too. Otherwise ssh can outlive it, holding
/// the proxy port, for as long as the browser keeps connections open.
fn stop_ssh_with_us() {
    extern "C" fn on_signal(signal: libc::c_int) {
        let pid = SSH_PID.load(Ordering::SeqCst);
        // SAFETY: kill and _exit are async-signal-safe.
        unsafe {
            if pid > 0 {
                libc::kill(pid, libc::SIGTERM);
            }
            libc::_exit(128 + signal);
        }
    }
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: on_signal only calls async-signal-safe functions.
        unsafe {
            libc::signal(signal, on_signal as *const () as libc::sighandler_t);
        }
    }
}

/// The ssh that carries the proxy and, given a remote command, the relay.
/// Keepalives notice a dead link (a laptop waking from sleep, a dropped
/// Wi-Fi) within about 30 seconds, and ExitOnForwardFailure makes a proxy
/// that cannot listen fail rather than connect without it. BatchMode keeps a
/// run with no terminal, such as a user service, from hanging on a password
/// prompt.
fn ssh(settings: &Settings, remote: Option<&str>) -> Command {
    let mut cmd = Command::new("ssh");
    if remote.is_none() {
        cmd.arg("-N");
    }
    cmd.args(["-D", &format!("127.0.0.1:{}", settings.port)])
        .args(["-o", "ExitOnForwardFailure=yes"])
        .args(["-o", "ServerAliveInterval=10"])
        .args(["-o", "ServerAliveCountMax=3"])
        .args(["-o", "ConnectTimeout=10"])
        .args(["-o", "BatchMode=yes"])
        .arg(&settings.host);
    if let Some(remote) = remote {
        cmd.arg(remote);
    }
    cmd
}

/// Whether something accepts connections on the proxy port, which on the
/// laptop means a `tether connect` is up.
pub fn proxy_up(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
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
