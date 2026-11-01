//! What a running `connect` tells `tether status` on the laptop: which server
//! it connects to, whether it runs as the background service, and since when
//! its proxy has been up.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{self, Settings};
use crate::connect::proxy_up;
use crate::relay::duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub background: bool,
    /// Seconds since the Unix epoch at which the proxy came up, while it is
    /// up.
    pub connected_since: Option<u64>,
}

fn path() -> Result<PathBuf> {
    Ok(config::runtime_dir()?.join("tether-connect.json"))
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Records the state. A failure only costs `status` its answer, so it is
/// not worth stopping the connection for.
pub fn write(state: &State) {
    let Ok(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let Ok(json) = serde_json::to_string(state) else {
        return;
    };
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The state of a `connect` that is still running.
fn read() -> Option<State> {
    let text = std::fs::read_to_string(path().ok()?).ok()?;
    let state: State = serde_json::from_str(&text).ok()?;
    // SAFETY: signal 0 only checks that the process exists.
    let alive = unsafe { libc::kill(state.pid as libc::pid_t, 0) } == 0;
    alive.then_some(state)
}

/// Whether this process is the background service rather than a terminal's.
pub fn in_background() -> bool {
    std::env::var_os("INVOCATION_ID").is_some()
        || std::env::var("XPC_SERVICE_NAME").is_ok_and(|name| name == crate::service::LABEL)
}

/// The laptop's half of `tether status`, for when no relay answered.
pub fn status() -> Result<()> {
    let Some(state) = read() else {
        let port = Settings::load(None)?.port;
        if proxy_up(port) {
            bail!(
                "not connected by tether, but something else holds 127.0.0.1:{port}, such as an older tether connect or devproxy; find it with: lsof -i :{port}"
            );
        }
        bail!(
            "no laptop connected, and this machine is not connected to a server; run tether connect on the laptop"
        );
    };
    let how = if state.background {
        "in the background"
    } else {
        "in a terminal (tether connect --foreground)"
    };
    match state.connected_since {
        Some(since) => {
            println!(
                "connected to {}, {how}, for {}",
                state.host,
                duration(now().saturating_sub(since))
            );
            Ok(())
        }
        None => {
            let log = if state.background {
                crate::service::logs_hint()?
            } else {
                "its terminal".into()
            };
            bail!(
                "connecting to {}, {how}, but not connected yet; see {log}",
                state.host
            )
        }
    }
}
