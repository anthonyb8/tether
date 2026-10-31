//! The laptop's tether browser: a Chromium-family browser or Firefox with its
//! own profile whose traffic, `localhost` included, goes through the SOCKS
//! proxy to the server. The rest of the laptop's browsing is untouched.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::{self, Settings};
use crate::spawn::detached;

/// Chromium-family browsers by command name, in order of preference.
const CHROMIUM_COMMANDS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "brave",
    "brave-browser",
    "microsoft-edge",
];

/// The same browsers as macOS installs them.
const CHROMIUM_APPS: &[&str] = &[
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

/// Firefox by command name, then as macOS installs it, used when no
/// Chromium-family browser is.
const FIREFOX: &[&str] = &[
    "firefox",
    "/Applications/Firefox.app/Contents/MacOS/firefox",
];

/// Opens the tether browser at `url`, or a new window without one. A profile
/// that is already open takes the URL as a new tab.
pub fn launch(settings: &Settings, url: Option<&str>) -> Result<()> {
    let browser = find(settings.browser.as_deref())?;
    let mut cmd = Command::new(&browser);
    if is_firefox(&browser) {
        let profile = config::dir()?.join("firefox");
        firefox_profile(&profile, settings.port)?;
        cmd.arg("--profile").arg(&profile);
        if let Some(url) = url {
            cmd.args(["--new-tab", url]);
        }
    } else {
        cmd.args(chromium_args(
            &config::dir()?.join("chromium"),
            settings.port,
        ));
        if let Some(url) = url {
            cmd.arg(url);
        }
    }
    detached(&mut cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not run {}", browser.display()))?;
    Ok(())
}

/// The flags that make a Chromium profile the tether browser.
fn chromium_args(profile: &Path, port: u16) -> Vec<String> {
    vec![
        format!("--user-data-dir={}", profile.display()),
        format!("--proxy-server=socks5://127.0.0.1:{port}"),
        // Removes Chromium's built-in bypass for localhost, which is the
        // point: without it localhost would stay on the laptop. A socks5
        // proxy also resolves names on the server.
        "--proxy-bypass-list=<-loopback>".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
    ]
}

fn is_firefox(browser: &Path) -> bool {
    browser
        .file_name()
        .is_some_and(|name| name.to_string_lossy().to_lowercase().contains("firefox"))
}

/// Writes the profile's `user.js`, which Firefox applies at every start, so a
/// changed port takes effect without touching the profile by hand.
fn firefox_profile(profile: &Path, port: u16) -> Result<()> {
    std::fs::create_dir_all(profile)
        .with_context(|| format!("could not create {}", profile.display()))?;
    let prefs = format!(
        r#"// Written by tether at every launch; edits here are lost.
user_pref("network.proxy.type", 1);
user_pref("network.proxy.socks", "127.0.0.1");
user_pref("network.proxy.socks_port", {port});
user_pref("network.proxy.socks_version", 5);
user_pref("network.proxy.socks_remote_dns", true);
user_pref("network.proxy.socks5_remote_dns", true);
user_pref("network.proxy.no_proxies_on", "");
// Without this Firefox never proxies localhost, which would stay the laptop's.
user_pref("network.proxy.allow_hijacking_localhost", true);
user_pref("browser.shell.checkDefaultBrowser", false);
"#
    );
    let path = profile.join("user.js");
    std::fs::write(&path, prefs).with_context(|| format!("could not write {}", path.display()))
}

/// The configured browser, else the first Chromium-family one installed,
/// else Firefox.
fn find(configured: Option<&str>) -> Result<PathBuf> {
    if let Some(browser) = configured {
        return which(browser)
            .with_context(|| format!("the configured browser {browser:?} was not found"));
    }
    CHROMIUM_COMMANDS
        .iter()
        .find_map(|name| which(name))
        .or_else(|| {
            CHROMIUM_APPS
                .iter()
                .map(PathBuf::from)
                .find(|path| path.is_file())
        })
        .or_else(|| FIREFOX.iter().find_map(|name| which(name)))
        .context(
            "no supported browser found; install Chromium, Chrome or Firefox, or set browser in ~/.config/tether/config.toml",
        )
}

/// A command name looked up on PATH, or a path that exists.
fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// `tether browser`: makes sure `connect` is running, starting it in the
/// background when it is not, then opens a tether browser window.
pub fn browser(host: Option<String>) -> Result<()> {
    let settings = Settings::load(host.clone())?;
    if !crate::connect::proxy_up(settings.port) {
        start_connect(host, settings.port)?;
    }
    launch(&settings, None)
}

fn start_connect(host: Option<String>, port: u16) -> Result<()> {
    let log_path = log_path()?;
    let log = std::fs::File::create(&log_path)
        .with_context(|| format!("could not write {}", log_path.display()))?;
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("connect").args(host);
    let mut child = detached(&mut cmd)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .context("could not start tether connect")?;
    eprintln!(
        "tether: started tether connect in the background, logging to {}",
        log_path.display()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if crate::connect::proxy_up(port) {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            bail!(
                "tether connect exited ({status}); see {}",
                log_path.display()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!(
        "the proxy did not come up within 15 seconds; see {}",
        log_path.display()
    )
}

/// Where a background `connect` logs: `~/.cache/tether/connect.log`.
fn log_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let dir = PathBuf::from(home).join(".cache/tether");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("connect.log"))
}
