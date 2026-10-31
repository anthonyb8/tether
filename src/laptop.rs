//! What the laptop does with each request the server sends it.

use std::process::{Command, Stdio};

use crate::protocol::{Ack, Request, check_url};

pub fn handle(request: Request) -> Ack {
    match request {
        Request::Open { url } => match check_url(&url) {
            Ok(()) => open(&url),
            Err(reason) => Ack::error(reason),
        },
        // A newer server asking an older laptop for something it cannot do.
        Request::Notify { .. } => {
            Ack::error("this laptop's tether cannot show notifications; update it")
        }
        Request::Status => Ack::error("status is answered by the relay"),
    }
}

/// Opens a URL with the desktop's opener. The opener is not waited on: some
/// stay in the foreground until the browser they started exits.
fn open(url: &str) -> Ack {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            Ack::ok()
        }
        Err(err) => Ack::error(format!("could not run {opener}: {err}")),
    }
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
