//! Where the laptop connects: the server's SSH host and the local SOCKS port.
//!
//! Each setting comes from, in order: the command line, the environment
//! (`TETHER_HOST`, `TETHER_PORT`), `~/.config/tether/config.toml`, then a
//! default that matches devproxy's, so moving over changes nothing.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

const DEFAULT_HOST: &str = "server";
const DEFAULT_PORT: u16 = 1080;

/// `~/.config/tether/config.toml`. Every key is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub browser: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub browser: Option<String>,
}

impl Settings {
    /// Settings for this run, with `host` given on the command line if any.
    pub fn load(host: Option<String>) -> Result<Settings> {
        let file = read_file()?;
        Settings::resolve(host, |name| std::env::var(name).ok(), file)
    }

    fn resolve(
        host: Option<String>,
        env: impl Fn(&str) -> Option<String>,
        file: File,
    ) -> Result<Settings> {
        let port = match env("TETHER_PORT").filter(|v| !v.is_empty()) {
            Some(port) => port
                .parse()
                .ok()
                .filter(|&port| port != 0)
                .with_context(|| format!("TETHER_PORT={port} is not a port (1 to 65535)"))?,
            None => file.port.unwrap_or(DEFAULT_PORT),
        };
        let host = host
            .or_else(|| env("TETHER_HOST").filter(|v| !v.is_empty()))
            .or(file.host)
            .unwrap_or_else(|| DEFAULT_HOST.to_owned());
        Ok(Settings {
            host,
            port,
            browser: file.browser,
        })
    }
}

/// tether's own config directory: `$XDG_CONFIG_HOME/tether`, else
/// `~/.config/tether`, on macOS too, where a CLI's dotfiles live by habit.
pub fn dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("tether"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/tether"))
}

fn read_file() -> Result<File> {
    let path = dir()?.join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("could not read {}", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
        Err(err) => Err(err).with_context(|| format!("could not read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn defaults_match_devproxy() {
        let s = Settings::resolve(None, env(&[]), File::default()).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("server", 1080));
    }

    #[test]
    fn the_command_line_beats_the_environment_which_beats_the_file() {
        let file = || File {
            host: Some("file-host".into()),
            port: Some(2000),
            browser: None,
        };
        let vars = [("TETHER_HOST", "env-host"), ("TETHER_PORT", "3000")];

        let s = Settings::resolve(Some("arg-host".into()), env(&vars), file()).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("arg-host", 3000));

        let s = Settings::resolve(None, env(&vars), file()).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("env-host", 3000));

        let s = Settings::resolve(None, env(&[]), file()).unwrap();
        assert_eq!((s.host.as_str(), s.port), ("file-host", 2000));
    }

    #[test]
    fn a_bad_tether_port_is_refused_by_name() {
        let err = Settings::resolve(None, env(&[("TETHER_PORT", "socks")]), File::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("TETHER_PORT=socks"), "{err}");
    }
}
