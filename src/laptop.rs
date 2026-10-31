//! What the laptop does with each request the server sends it.

use std::process::{Command, Stdio};

use crate::browser;
use crate::config::Settings;
use crate::protocol::{Ack, Request, check_url};

pub fn handle(request: Request, settings: &Settings) -> Ack {
    match request {
        Request::Open { url } => match check_url(&url) {
            Ok(()) => match browser::launch(settings, Some(&url)) {
                Ok(()) => Ack::ok(),
                Err(err) => Ack::error(format!("{err:#}")),
            },
            Err(reason) => Ack::error(reason),
        },
        Request::Notify { text } => match notify(&text) {
            Ok(()) => Ack::ok(),
            Err(reason) => Ack::error(reason),
        },
        Request::Status => Ack::error("status is answered by the relay"),
    }
}

/// Shows a desktop notification titled "tether". The text goes in as an
/// argument, never into a script, so nothing in it needs escaping.
fn notify(text: &str) -> Result<(), String> {
    let (program, mut cmd) = if cfg!(target_os = "macos") {
        let mut cmd = Command::new("osascript");
        cmd.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 1 of argv) with title \"tether\"",
            "-e",
            "end run",
            text,
        ]);
        ("osascript", cmd)
    } else {
        let mut cmd = Command::new("notify-send");
        cmd.args(["--app-name=tether", "tether", text]);
        ("notify-send", cmd)
    };
    let out = cmd.stdin(Stdio::null()).output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            format!("{program} is not installed on the laptop")
        } else {
            format!("could not run {program}: {err}")
        }
    })?;
    if !out.status.success() {
        let reason = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{program} failed: {}", reason.trim()));
    }
    Ok(())
}

/// This machine's name, as `status` on the server shows it.
pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its whole length, and gethostname
    // writes at most that many bytes.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return "laptop".into();
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name: String = String::from_utf8_lossy(&buf[..len])
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    if name.is_empty() {
        "laptop".into()
    } else {
        name
    }
}
