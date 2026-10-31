//! The messages that cross the SSH connection between the server's relay and
//! the laptop, one JSON object per line.
//!
//! A server command sends the relay a [`Request`] over the relay's socket. The
//! relay gives it an id and writes it to the laptop as an [`Envelope`]; the
//! laptop answers with an [`Ack`] carrying the same id.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Request {
    /// Open a URL in the laptop's tether browser.
    Open { url: String },
    /// Show a desktop notification on the laptop.
    Notify { text: String },
    /// Who is connected; answered by the relay itself.
    Status,
}

/// A request on its way to the laptop.
#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub id: u64,
    #[serde(flatten)]
    pub request: Request,
}

/// The laptop's answer to an [`Envelope`], or the relay's to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ack {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// `None` when it worked, else a one-line reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// For a status request: the laptop's name and how many seconds it has
    /// been connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub laptop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_secs: Option<u64>,
}

impl Ack {
    pub fn ok() -> Ack {
        Ack {
            id: None,
            error: None,
            laptop: None,
            connected_secs: None,
        }
    }

    pub fn error(reason: impl Into<String>) -> Ack {
        Ack {
            error: Some(reason.into()),
            ..Ack::ok()
        }
    }
}

/// Only web pages may be opened: a request cannot reach a file, a custom URL
/// scheme or a browser flag on the laptop.
pub fn check_url(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    let web = lower.starts_with("http://") || lower.starts_with("https://");
    if !web || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!(
            "only http and https URLs can be opened, not {url:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_urls_pass() {
        for ok in ["http://localhost:5173/", "HTTPS://example.com/a?b=c"] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "--disable-web-security",
            "localhost:5173",
            "http://a b",
            "http://a\nb",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }
}
