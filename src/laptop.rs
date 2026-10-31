//! What the laptop does with each request the server sends it.

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
        // A newer server asking an older laptop for something it cannot do.
        Request::Notify { .. } => {
            Ack::error("this laptop's tether cannot show notifications; update it")
        }
        Request::Status => Ack::error("status is answered by the relay"),
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
