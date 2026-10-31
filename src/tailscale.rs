//! Sharing local ports with the tailnet through `tailscale serve`.
//!
//! tether only drives the `tailscale` CLI, so whatever Tailscale version the
//! machine has keeps working and there is no daemon API to track.

use std::collections::BTreeMap;
use std::io;
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// A port shared with the tailnet: its HTTPS address and what it forwards to.
#[derive(Debug, PartialEq, Eq)]
pub struct Share {
    pub port: u16,
    pub origin: String,
    pub target: String,
}

/// The HTTPS address a port on this machine is reached at, given its tailnet
/// DNS name.
pub fn origin(host: &str, port: u16) -> String {
    if port == 443 {
        format!("https://{host}")
    } else {
        format!("https://{host}:{port}")
    }
}

/// This machine's tailnet DNS name, once Tailscale is running and can issue it
/// an HTTPS certificate.
pub fn host() -> Result<String> {
    let out = run(&["status", "--json"])?;
    host_from_status(&out.stdout)
}

/// Serves `http://localhost:<port>` on the tailnet at the same port over HTTPS.
///
/// Tailscale's own output, such as the link to enable Serve on a tailnet that
/// has not yet, goes to stderr so stdout stays the address alone.
pub fn serve(port: u16) -> Result<()> {
    let status = Command::new("tailscale")
        .args(["serve", "--bg", "--yes", &format!("--https={port}")])
        .arg(format!("http://localhost:{port}"))
        .stdin(Stdio::null())
        .stdout(io::stderr())
        .status()
        .map_err(not_installed)?;
    if !status.success() {
        bail!("tailscale serve failed for port {port}");
    }
    Ok(())
}

/// Stops serving a port. Unsharing a port that is not shared is an error, so
/// a typo does not pass silently.
pub fn unserve(port: u16) -> Result<()> {
    if !shares()?.iter().any(|share| share.port == port) {
        bail!("port {port} is not shared");
    }
    run(&["serve", &format!("--https={port}"), "off"])?;
    Ok(())
}

/// Every port this machine shares with the tailnet over HTTPS.
pub fn shares() -> Result<Vec<Share>> {
    let out = run(&["serve", "status", "--json"])?;
    shares_from_serve_status(&out.stdout)
}

fn run(args: &[&str]) -> Result<Output> {
    let out = Command::new("tailscale")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(not_installed)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("tailscale {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(out)
}

fn not_installed(err: io::Error) -> anyhow::Error {
    if err.kind() == io::ErrorKind::NotFound {
        anyhow::anyhow!("tailscale is not installed; see https://tailscale.com/download")
    } else {
        anyhow::Error::new(err).context("could not run tailscale")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Status {
    backend_state: String,
    #[serde(rename = "Self")]
    me: Option<Peer>,
    cert_domains: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Peer {
    #[serde(rename = "DNSName")]
    dns_name: String,
}

fn host_from_status(json: &[u8]) -> Result<String> {
    let status: Status = serde_json::from_slice(json).context("could not read tailscale status")?;
    if status.backend_state != "Running" {
        bail!(
            "tailscale is not connected (it is {}); run tailscale up",
            status.backend_state
        );
    }
    let host = status
        .me
        .map(|me| me.dns_name.trim_end_matches('.').to_owned())
        .filter(|name| !name.is_empty())
        .context(
            "this machine has no tailnet DNS name; turn on MagicDNS in the Tailscale admin console",
        )?;
    if !status.cert_domains.unwrap_or_default().contains(&host) {
        bail!(
            "this tailnet cannot issue HTTPS certificates; turn on HTTPS Certificates at https://login.tailscale.com/admin/dns"
        );
    }
    Ok(host)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ServeStatus {
    #[serde(default)]
    web: BTreeMap<String, WebServer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WebServer {
    #[serde(default)]
    handlers: BTreeMap<String, Handler>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Handler {
    proxy: Option<String>,
}

fn shares_from_serve_status(json: &[u8]) -> Result<Vec<Share>> {
    let status: ServeStatus =
        serde_json::from_slice(json).context("could not read tailscale serve status")?;
    let mut shares = Vec::new();
    for (host_port, server) in status.web {
        let Some((host, port)) = host_port.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse() else { continue };
        for (path, handler) in server.handlers {
            let Some(target) = handler.proxy else {
                continue;
            };
            let mut origin = origin(host, port);
            if path != "/" {
                origin.push_str(&path);
            }
            shares.push(Share {
                port,
                origin,
                target,
            });
        }
    }
    Ok(shares)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_is_the_dns_name_without_its_trailing_dot() {
        let json = br#"{"BackendState":"Running","Self":{"DNSName":"box.tail1.ts.net."},"CertDomains":["box.tail1.ts.net"]}"#;
        assert_eq!(host_from_status(json).unwrap(), "box.tail1.ts.net");
    }

    #[test]
    fn host_needs_https_certificates_on() {
        let json = br#"{"BackendState":"Running","Self":{"DNSName":"box.tail1.ts.net."},"CertDomains":null}"#;
        let err = host_from_status(json).unwrap_err().to_string();
        assert!(err.contains("HTTPS Certificates"), "{err}");
    }

    #[test]
    fn host_needs_tailscale_connected() {
        let json = br#"{"BackendState":"Stopped","Self":{"DNSName":""},"CertDomains":null}"#;
        let err = host_from_status(json).unwrap_err().to_string();
        assert!(err.contains("tailscale up"), "{err}");
    }

    #[test]
    fn shares_lists_each_proxied_port_with_its_origin() {
        let json = br#"{
            "TCP": {"443": {"HTTPS": true}, "5173": {"HTTPS": true}},
            "Web": {
                "box.tail1.ts.net:443": {"Handlers": {"/": {"Proxy": "http://localhost:3000"}}},
                "box.tail1.ts.net:5173": {"Handlers": {"/": {"Proxy": "http://localhost:5173"}, "/files": {"Path": "/srv"}}}
            }
        }"#;
        assert_eq!(
            shares_from_serve_status(json).unwrap(),
            vec![
                Share {
                    port: 443,
                    origin: "https://box.tail1.ts.net".into(),
                    target: "http://localhost:3000".into(),
                },
                Share {
                    port: 5173,
                    origin: "https://box.tail1.ts.net:5173".into(),
                    target: "http://localhost:5173".into(),
                },
            ]
        );
    }

    #[test]
    fn shares_is_empty_with_nothing_served() {
        assert_eq!(shares_from_serve_status(b"{}").unwrap(), vec![]);
    }
}
