//! Runs the built tether as a user does, with stand-ins for the programs it
//! drives (`tailscale`, `ssh`) first on PATH. Each stand-in records its
//! arguments, one call per line, for the test to check.

#![allow(dead_code)] // each test binary uses its own part of this

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

pub struct Sandbox {
    pub dir: PathBuf,
}

impl Sandbox {
    /// An empty sandbox, its directory doubling as HOME so no real config is read.
    pub fn new(test: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("tether-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).unwrap();
        Sandbox { dir }
    }

    /// Puts a stand-in `name` on PATH that logs its arguments to
    /// `<name>.calls`, then runs `body` as sh.
    pub fn fake(&self, name: &str, body: &str) {
        let script = self.dir.join("bin").join(name);
        let calls = self.dir.join(format!("{name}.calls"));
        fs::write(
            &script,
            format!("#!/bin/sh\necho \"$*\" >> '{}'\n{body}\n", calls.display()),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }

    pub fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    pub fn calls(&self, name: &str) -> String {
        fs::read_to_string(self.dir.join(format!("{name}.calls"))).unwrap_or_default()
    }

    pub fn tether(&self, args: &[&str]) -> Command {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tether"));
        cmd.args(args)
            .env("PATH", path)
            .env("HOME", &self.dir)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_RUNTIME_DIR")
            .env_remove("TETHER_HOST")
            .env_remove("TETHER_PORT");
        cmd
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.tether(args).output().unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
