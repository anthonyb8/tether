//! The server half: `relay`, which the laptop's `connect` runs over SSH, and
//! the commands (`open`, `status`) that reach the laptop through it.
//!
//! The relay owns a Unix socket only this user can open. Each connection to
//! it carries one [`Request`] line and gets one [`Ack`] line back. The relay
//! answers a status request itself and forwards the rest to the laptop on
//! stdout, matching the laptop's acks on stdin back by id.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::protocol::{Ack, Envelope, Request, check_url};

/// How long the relay waits for the laptop to act on a request.
const LAPTOP_TIMEOUT: Duration = Duration::from_secs(2);
/// How often a replaced relay checks whether the socket is free to take back.
const RECLAIM_EVERY: Duration = Duration::from_secs(1);
/// How long a command waits for the relay, a little over the laptop's budget.
const RELAY_TIMEOUT: Duration = Duration::from_secs(3);

/// `$XDG_RUNTIME_DIR/tether.sock`; without that variable, which a session
/// started outside PAM lacks, the same `/run/user/<uid>` it would name, so
/// the relay and the commands agree; failing that `~/.cache/tether/tether.sock`.
pub fn socket_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("tether.sock"));
    }
    // SAFETY: getuid cannot fail.
    let run = PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }));
    if run.is_dir() {
        return Ok(run.join("tether.sock"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cache/tether/tether.sock"))
}

/// Runs the relay until the laptop disconnects, which closes stdin.
pub fn relay(laptop: String) -> Result<()> {
    let path = socket_path()?;
    let dir = path.parent().context("socket path has no directory")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("could not create {}", dir.display()))?;
    // A newer relay replaces an older one, so the laptop that connected last
    // gets the tabs. The older relay keeps running for its laptop's proxy and
    // takes the socket back once the newer one has gone.
    let relay = Arc::new(Relay {
        laptop,
        since: Instant::now(),
        stdout: Mutex::new(io::stdout()),
        waiting: Mutex::new(HashMap::new()),
        next_id: Mutex::new(0),
        socket: path.clone(),
        inode: AtomicU64::new(0),
    });
    // An older relay can take the path back between our removing and
    // binding it, so take it again.
    let mut bound = Ok(());
    for _ in 0..3 {
        let _ = fs::remove_file(&path);
        bound = relay.listen();
        if bound.is_ok() {
            break;
        }
    }
    bound?;
    let watching = Arc::clone(&relay);
    thread::spawn(move || {
        loop {
            thread::sleep(RECLAIM_EVERY);
            if !watching.socket.exists() {
                let _ = watching.listen();
            }
        }
    });

    // The laptop's acks, until it goes away.
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(ack) = serde_json::from_str::<Ack>(&line) else {
            continue;
        };
        if let Some(id) = ack.id
            && let Some(waiter) = relay.waiting.lock().unwrap().remove(&id)
        {
            let _ = waiter.send(ack);
        }
    }

    // Remove the socket unless a newer relay has already replaced it.
    if fs::metadata(&path).is_ok_and(|m| m.ino() == relay.inode.load(Ordering::SeqCst)) {
        let _ = fs::remove_file(&path);
    }
    Ok(())
}

struct Relay {
    laptop: String,
    since: Instant,
    stdout: Mutex<io::Stdout>,
    waiting: Mutex<HashMap<u64, Sender<Ack>>>,
    next_id: Mutex<u64>,
    socket: PathBuf,
    /// The inode of the socket this relay last bound, to tell it from a
    /// newer relay's.
    inode: AtomicU64,
}

impl Relay {
    /// Binds the socket and serves each connection to it on its own thread.
    /// Fails when another relay bound it first.
    fn listen(self: &Arc<Self>) -> Result<()> {
        let listener = UnixListener::bind(&self.socket)
            .with_context(|| format!("could not listen on {}", self.socket.display()))?;
        fs::set_permissions(&self.socket, fs::Permissions::from_mode(0o600))?;
        self.inode
            .store(fs::metadata(&self.socket)?.ino(), Ordering::SeqCst);
        let relay = Arc::clone(self);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let relay = Arc::clone(&relay);
                thread::spawn(move || relay.serve(stream));
            }
        });
        Ok(())
    }

    fn serve(&self, stream: UnixStream) {
        let _ = stream.set_read_timeout(Some(RELAY_TIMEOUT));
        let mut line = String::new();
        let Ok(mut writer) = stream.try_clone() else {
            return;
        };
        if BufReader::new(stream).read_line(&mut line).is_err() {
            return;
        }
        let ack = match serde_json::from_str::<Request>(&line) {
            Ok(request) => self.handle(request),
            Err(err) => Ack::error(format!("bad request: {err}")),
        };
        let _ = writeln!(writer, "{}", serde_json::to_string(&ack).unwrap());
    }

    fn handle(&self, request: Request) -> Ack {
        if request == Request::Status {
            return Ack {
                laptop: Some(self.laptop.clone()),
                connected_secs: Some(self.since.elapsed().as_secs()),
                ..Ack::ok()
            };
        }
        let id = {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            *next
        };
        let (tx, rx) = mpsc::channel();
        self.waiting.lock().unwrap().insert(id, tx);
        let line = serde_json::to_string(&Envelope { id, request }).unwrap();
        let sent = {
            let mut out = self.stdout.lock().unwrap();
            writeln!(out, "{line}").and_then(|()| out.flush())
        };
        let ack = match sent {
            Ok(()) => rx
                .recv_timeout(LAPTOP_TIMEOUT)
                .unwrap_or_else(|_| Ack::error("the laptop did not answer")),
            Err(_) => Ack::error("the laptop is no longer connected"),
        };
        self.waiting.lock().unwrap().remove(&id);
        ack
    }
}

/// Sends one request to the relay and returns its answer, or `None` when no
/// relay is running, which means no laptop is connected.
fn ask(request: &Request) -> Result<Option<Ack>> {
    let path = socket_path()?;
    let stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err).with_context(|| format!("could not reach {}", path.display())),
    };
    stream.set_read_timeout(Some(RELAY_TIMEOUT))?;
    writeln!(&stream, "{}", serde_json::to_string(request)?)?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .context("the relay did not answer")?;
    if line.is_empty() {
        bail!("the relay closed the connection without answering");
    }
    Ok(Some(
        serde_json::from_str(&line).context("the relay's answer was not understood")?,
    ))
}

/// Sends a request the laptop acts on, failing with its reason if it did not.
pub fn send(request: Request) -> Result<()> {
    match ask(&request)? {
        None => bail!("no browser connected"),
        Some(Ack {
            error: Some(reason),
            ..
        }) => bail!("{reason}"),
        Some(_) => Ok(()),
    }
}

pub fn open(url: String) -> Result<()> {
    check_url(&url).map_err(anyhow::Error::msg)?;
    send(Request::Open { url })
}

pub fn notify(text: String) -> Result<()> {
    send(Request::Notify { text })
}

pub fn status() -> Result<()> {
    let Some(ack) = ask(&Request::Status)? else {
        bail!("no laptop connected");
    };
    let laptop = ack.laptop.unwrap_or_else(|| "a laptop".into());
    let since = ack.connected_secs.unwrap_or(0);
    println!("connected: {laptop}, for {}", duration(since));
    Ok(())
}

/// A short human duration: "45s", "12m", "3h 5m", "2d 4h".
fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{secs}s"),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_reads_at_a_glance() {
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(12 * 60 + 5), "12m");
        assert_eq!(duration(3 * 3600 + 5 * 60), "3h 5m");
        assert_eq!(duration(2 * 86_400 + 4 * 3600 + 59), "2d 4h");
    }
}
