//! Running `connect` in the background: a login service that keeps the
//! laptop connected without a terminal, a systemd user unit on Linux and a
//! launchd agent on macOS.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::config::Settings;
use crate::connect::proxy_up;

const UNIT: &str = "tether-connect.service";
const LABEL: &str = "dev.tether.connect";

/// How long to wait for a freshly started service's proxy to listen.
const START_TIMEOUT: Duration = Duration::from_secs(15);

/// The desktop session's variables a background connect needs to open a
/// browser window, show a notification and use the SSH agent.
const SESSION_VARS: &[&str] = &["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "SSH_AUTH_SOCK"];

/// `tether connect`: writes the service, (re)starts it, and returns once its
/// proxy is up.
pub fn start(host: Option<String>) -> Result<()> {
    let settings = Settings::load(host.clone())?;
    if proxy_up(settings.port) && !running() {
        bail!(
            "127.0.0.1:{} is in use, likely by a tether connect --foreground; stop it, then run tether connect again",
            settings.port
        );
    }
    // The host and port go into the service only when given on the command
    // line or in the environment, so a later edit to the config file still
    // applies.
    let host = host.or_else(|| std::env::var("TETHER_HOST").ok().filter(|h| !h.is_empty()));
    if let Some(host) = &host
        && !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_@:".contains(c))
    {
        bail!("{host:?} is not a host name");
    }
    let port = std::env::var("TETHER_PORT").ok().filter(|p| !p.is_empty());
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .context("could not find the tether binary")?;

    let file = file()?;
    std::fs::create_dir_all(file.parent().unwrap())?;
    let contents = if cfg!(target_os = "macos") {
        plist(&exe, host.as_deref(), port.as_deref(), &log()?)
    } else {
        unit(&exe, host.as_deref(), port.as_deref())
    };
    std::fs::write(&file, contents)
        .with_context(|| format!("could not write {}", file.display()))?;

    if cfg!(target_os = "macos") {
        // Unloading first makes a changed agent take effect; it fails
        // harmlessly when nothing was loaded.
        let _ = launchctl(&["bootout", &target()]);
        launchctl(&["bootstrap", &domain(), &file.to_string_lossy()])?;
    } else {
        systemctl(&["daemon-reload"])?;
        let session: Vec<&str> = SESSION_VARS
            .iter()
            .copied()
            .filter(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
            .collect();
        if !session.is_empty() {
            systemctl(&[&["import-environment"], session.as_slice()].concat())?;
        }
        systemctl(&["enable", UNIT])?;
        systemctl(&["restart", UNIT])?;
    }

    let deadline = Instant::now() + START_TIMEOUT;
    while !proxy_up(settings.port) {
        if Instant::now() >= deadline {
            bail!(
                "the connection to {} did not come up; see {}",
                settings.host,
                logs_hint()?
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
    println!(
        "tether: connected to {}, running in the background",
        settings.host
    );
    println!("  (starts at login; tether connect --stop to remove)");
    Ok(())
}

/// `tether connect --stop`: stops the service and removes it.
pub fn stop() -> Result<()> {
    let file = file()?;
    if !file.exists() {
        bail!("tether connect is not running in the background");
    }
    if cfg!(target_os = "macos") {
        let _ = launchctl(&["bootout", &target()]);
        std::fs::remove_file(&file)?;
    } else {
        systemctl(&["disable", "--now", UNIT])?;
        std::fs::remove_file(&file)?;
        systemctl(&["daemon-reload"])?;
    }
    println!("tether: disconnected; it no longer starts at login");
    Ok(())
}

/// Whether the service is up, so restarting it is ours to do.
fn running() -> bool {
    let status = if cfg!(target_os = "macos") {
        Command::new("launchctl")
            .args(["print", &target()])
            .output()
    } else {
        Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", UNIT])
            .output()
    };
    status.is_ok_and(|out| out.status.success())
}

fn systemctl(args: &[&str]) -> Result<()> {
    run("systemctl", &[&["--user"], args].concat())
}

fn launchctl(args: &[&str]) -> Result<()> {
    run("launchctl", args)
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "{program} was not found, so tether cannot run in the background; use tether connect --foreground"
                )
            } else {
                anyhow::Error::new(err).context(format!("could not run {program}"))
            }
        })?;
    if !out.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").context("HOME is not set")?,
    ))
}

/// Where the service is defined.
fn file() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        return Ok(home()?.join(format!("Library/LaunchAgents/{LABEL}.plist")));
    }
    let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home()?.join(".config"),
    };
    Ok(config.join("systemd/user").join(UNIT))
}

/// Where a background connect's messages go on macOS. On Linux they go to the
/// journal.
fn log() -> Result<PathBuf> {
    Ok(home()?.join("Library/Logs/tether-connect.log"))
}

fn logs_hint() -> Result<String> {
    Ok(if cfg!(target_os = "macos") {
        log()?.display().to_string()
    } else {
        format!("journalctl --user -u {UNIT}")
    })
}

/// launchd's domain for this user's agents.
fn domain() -> String {
    // SAFETY: getuid cannot fail.
    format!("gui/{}", unsafe { libc::getuid() })
}

fn target() -> String {
    format!("{}/{LABEL}", domain())
}

fn connect_args(host: Option<&str>) -> Vec<&str> {
    let mut args = vec!["connect", "--foreground"];
    args.extend(host);
    args
}

fn unit(exe: &std::path::Path, host: Option<&str>, port: Option<&str>) -> String {
    let exec = std::iter::once(format!("\"{}\"", exe.display()))
        .chain(connect_args(host).into_iter().map(String::from))
        .collect::<Vec<_>>()
        .join(" ");
    let env = port
        .map(|port| format!("Environment=TETHER_PORT={port}\n"))
        .unwrap_or_default();
    format!(
        "# Written by tether connect; tether connect --stop removes it.
[Unit]
Description=tether: connect this laptop to the server
After=network-online.target
Wants=network-online.target

[Service]
ExecStart={exec}
{env}# connect reconnects on its own; this restarts it only if it exits, such as
# when its proxy port is taken.
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
"
    )
}

fn plist(
    exe: &std::path::Path,
    host: Option<&str>,
    port: Option<&str>,
    log: &std::path::Path,
) -> String {
    let args: String = std::iter::once(exe.display().to_string())
        .chain(connect_args(host).into_iter().map(String::from))
        .map(|arg| format!("    <string>{}</string>\n", xml(&arg)))
        .collect();
    let env = port
        .map(|port| {
            format!(
                "  <key>EnvironmentVariables</key>\n  <dict>\n    <key>TETHER_PORT</key>\n    <string>{}</string>\n  </dict>\n",
                xml(port)
            )
        })
        .unwrap_or_default();
    let log = xml(&log.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by tether connect; tether connect --stop removes it. -->
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
{env}  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#
    )
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// On Linux, the desktop session's display and bus for a program started by
/// the background connect, which systemd may have started before the desktop
/// shared them. Read at each launch, so a desktop that shares them after the
/// service started is still found.
pub fn desktop_env(cmd: &mut Command) -> &mut Command {
    if cfg!(target_os = "macos")
        || ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
    {
        return cmd;
    }
    let Ok(out) = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdin(Stdio::null())
        .output()
    else {
        return cmd;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((name, value)) = line.split_once('=')
            && [
                "DISPLAY",
                "WAYLAND_DISPLAY",
                "XAUTHORITY",
                "DBUS_SESSION_BUS_ADDRESS",
            ]
            .contains(&name)
        {
            cmd.env(name, value);
        }
    }
    cmd
}
