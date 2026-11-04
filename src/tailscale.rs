//! Sharing local ports with the tailnet through `tailscale serve`, or with the
//! internet through `tailscale funnel`.
//!
//! tether only drives the `tailscale` CLI, so whatever Tailscale version the
//! machine has keeps working and there is no daemon API to track.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// The only HTTPS ports Funnel can listen on, in the order tether takes them.
const FUNNEL_PORTS: [u16; 3] = [443, 8443, 10000];

/// A port shared over HTTPS: the port it listens on, its address, what it
/// forwards to, and whether Funnel puts it on the internet rather than only
/// the tailnet.
#[derive(Debug, PartialEq, Eq)]
pub struct Share {
    pub port: u16,
    pub origin: String,
    pub target: String,
    pub public: bool,
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

/// Puts `http://localhost:<port>` on the internet through Funnel and returns
/// the HTTPS port it listens on: the one already in use if the port is public,
/// else the first of [`FUNNEL_PORTS`] nothing listens on.
///
/// As with [`serve`], Tailscale's own output, such as the link to enable
/// Funnel in the tailnet policy, goes to stderr.
pub fn funnel(port: u16) -> Result<u16> {
    let status = serve_status()?;
    let listen = funnel_port(&shares_from(&status), &listeners_from(&status), port)?;
    let status = Command::new("tailscale")
        .args(["funnel", "--bg", "--yes", &format!("--https={listen}")])
        .arg(format!("http://localhost:{port}"))
        .stdin(Stdio::null())
        .stdout(io::stderr())
        .status()
        .map_err(not_installed)?;
    if !status.success() {
        bail!("tailscale funnel failed for port {port}");
    }
    Ok(listen)
}

/// Stops sharing a local port: its tailnet share, which listens on the port
/// itself, and its public share, which listens on a Funnel port and forwards
/// to it. Unsharing a port that is not shared is an error, so a typo does not
/// pass silently.
pub fn unserve(port: u16) -> Result<()> {
    let shares = shares()?;
    let mut off = Vec::new();
    if shares
        .iter()
        .any(|share| !share.public && share.port == port)
    {
        off.push(("serve", port));
    }
    for share in &shares {
        if share.public && local_port(&share.target) == Some(port) {
            off.push(("funnel", share.port));
        }
    }
    if off.is_empty() {
        bail!("port {port} is not shared");
    }
    off.dedup();
    for (command, listen) in off {
        run(&[command, &format!("--https={listen}"), "off"])?;
    }
    Ok(())
}

/// Every port this machine shares over HTTPS, with the tailnet or publicly.
pub fn shares() -> Result<Vec<Share>> {
    Ok(shares_from(&serve_status()?))
}

fn serve_status() -> Result<ServeStatus> {
    let out = run(&["serve", "status", "--json"])?;
    serde_json::from_slice(&out.stdout).context("could not read tailscale serve status")
}

/// The Funnel port to share `port` on, given what is shared and every port
/// something listens on.
fn funnel_port(shares: &[Share], listeners: &BTreeSet<u16>, port: u16) -> Result<u16> {
    if let Some(share) = shares
        .iter()
        .find(|share| share.public && local_port(&share.target) == Some(port))
    {
        return Ok(share.port);
    }
    FUNNEL_PORTS
        .into_iter()
        .find(|listen| !listeners.contains(listen))
        .context(
            "Funnel can only listen on ports 443, 8443 and 10000, and all three are taken; see tether shares, and free one with tether unshare or tailscale funnel --https=<port> off",
        )
}

/// The port a share forwards to, when its target is on this machine.
fn local_port(target: &str) -> Option<u16> {
    let rest = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https+insecure://"))
        .or_else(|| target.strip_prefix("https://"))
        .unwrap_or(target);
    let authority = rest.split('/').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    if !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
        return None;
    }
    port.parse().ok()
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
    /// Every port something listens on, keyed by the port number.
    #[serde(rename = "TCP", default)]
    tcp: BTreeMap<String, TcpHandler>,
    #[serde(default)]
    web: BTreeMap<String, WebServer>,
    /// The `<host>:<port>` listeners Funnel puts on the internet.
    #[serde(default)]
    allow_funnel: BTreeMap<String, bool>,
    /// Listeners held by `tailscale serve` or `funnel` running in a terminal.
    #[serde(default)]
    foreground: BTreeMap<String, ServeStatus>,
}

/// A listener; tether needs only its port, which is its key.
#[derive(Deserialize)]
struct TcpHandler {}

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

/// Every port something listens on, foreground sessions included, so a
/// public share never takes one.
fn listeners_from(status: &ServeStatus) -> BTreeSet<u16> {
    let web = status.web.keys().filter_map(|hp| hp.rsplit_once(':'));
    let mut ports: BTreeSet<u16> = status
        .tcp
        .keys()
        .map(String::as_str)
        .chain(web.map(|(_, port)| port))
        .filter_map(|port| port.parse().ok())
        .collect();
    for session in status.foreground.values() {
        ports.extend(listeners_from(session));
    }
    ports
}

fn shares_from(status: &ServeStatus) -> Vec<Share> {
    let mut shares = Vec::new();
    for (host_port, server) in &status.web {
        let Some((host, port)) = host_port.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse() else { continue };
        let public = status.allow_funnel.get(host_port).copied().unwrap_or(false);
        for (path, handler) in &server.handlers {
            let Some(target) = &handler.proxy else {
                continue;
            };
            let mut origin = origin(host, port);
            if path != "/" {
                origin.push_str(path);
            }
            shares.push(Share {
                port,
                origin,
                target: target.clone(),
                public,
            });
        }
    }
    shares
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_needs_tailscale_connected() {
        let json = br#"{"BackendState":"Stopped","Self":{"DNSName":""},"CertDomains":null}"#;
        let err = host_from_status(json).unwrap_err().to_string();
        assert!(err.contains("tailscale up"), "{err}");
    }

    fn parse(json: &[u8]) -> ServeStatus {
        serde_json::from_slice(json).unwrap()
    }

    fn share(port: u16, target: &str, public: bool) -> Share {
        let origin = origin("box.tail1.ts.net", port);
        let target = target.into();
        Share {
            port,
            origin,
            target,
            public,
        }
    }

    #[test]
    fn shares_lists_each_proxied_port_and_marks_funnel_ones_public() {
        let json = br#"{
            "TCP": {"443": {"HTTPS": true}, "5173": {"HTTPS": true}},
            "Web": {
                "box.tail1.ts.net:443": {"Handlers": {"/": {"Proxy": "http://localhost:3000"}}},
                "box.tail1.ts.net:5173": {"Handlers": {"/": {"Proxy": "http://localhost:5173"}, "/files": {"Path": "/srv"}}}
            },
            "AllowFunnel": {"box.tail1.ts.net:443": true}
        }"#;
        assert_eq!(
            shares_from(&parse(json)),
            vec![
                share(443, "http://localhost:3000", true),
                share(5173, "http://localhost:5173", false),
            ]
        );
    }

    #[test]
    fn shares_is_empty_with_nothing_served() {
        assert_eq!(shares_from(&parse(b"{}")), vec![]);
    }

    #[test]
    fn a_public_share_takes_the_first_funnel_port_nothing_listens_on() {
        // 443 is a tailnet share someone else made; 8443 is held by a
        // tailscale serve running in a terminal.
        let json = br#"{
            "TCP": {"443": {"HTTPS": true}},
            "Web": {"box.tail1.ts.net:443": {"Handlers": {"/": {"Proxy": "http://localhost:3000"}}}},
            "Foreground": {"abc": {"TCP": {"8443": {"HTTPS": true}}}}
        }"#;
        let status = parse(json);
        let listeners = listeners_from(&status);
        assert_eq!(
            funnel_port(&shares_from(&status), &listeners, 5173).unwrap(),
            10000
        );
    }

    #[test]
    fn a_port_already_public_keeps_its_funnel_port() {
        let shares = [share(8443, "http://localhost:5173", true)];
        let listeners = BTreeSet::from([8443]);
        assert_eq!(funnel_port(&shares, &listeners, 5173).unwrap(), 8443);
    }

    #[test]
    fn a_public_share_fails_when_every_funnel_port_is_taken() {
        let listeners = BTreeSet::from(FUNNEL_PORTS);
        let err = funnel_port(&[], &listeners, 5173).unwrap_err().to_string();
        assert!(err.contains("443, 8443 and 10000"), "{err}");
    }

    #[test]
    fn only_a_target_on_this_machine_has_a_local_port() {
        assert_eq!(local_port("http://localhost:5173"), Some(5173));
        assert_eq!(local_port("http://127.0.0.1:5173/app"), Some(5173));
        assert_eq!(local_port("http://other:5173"), None);
    }
}
