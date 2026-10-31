//! `share`, `unshare`, `shares` and `origin`, run as a user runs them, against
//! a stand-in `tailscale` on PATH that answers from fixtures and records what
//! it was asked to do.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

const STATUS: &str = r#"{"BackendState":"Running","Self":{"DNSName":"box.tail1.ts.net."},"CertDomains":["box.tail1.ts.net"]}"#;
const SERVING_5173: &str =
    r#"{"Web":{"box.tail1.ts.net:5173":{"Handlers":{"/":{"Proxy":"http://localhost:5173"}}}}}"#;

struct FakeTailscale {
    dir: PathBuf,
}

impl FakeTailscale {
    fn new(test: &str, status: &str, serve_status: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tether-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("status.json"), status).unwrap();
        fs::write(dir.join("serve.json"), serve_status).unwrap();
        let script = dir.join("tailscale");
        fs::write(
            &script,
            format!(
                r#"#!/bin/sh
echo "$*" >> "{dir}/calls"
case "$*" in
  "status --json") cat "{dir}/status.json" ;;
  "serve status --json") cat "{dir}/serve.json" ;;
esac
"#,
                dir = dir.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        FakeTailscale { dir }
    }

    fn tether(&self, args: &[&str]) -> Output {
        let path = format!("{}:{}", self.dir.display(), std::env::var("PATH").unwrap());
        Command::new(env!("CARGO_BIN_EXE_tether"))
            .args(args)
            .env("PATH", path)
            .output()
            .unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.dir.join("calls")).unwrap_or_default()
    }
}

impl Drop for FakeTailscale {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn stdout(out: &Output) -> &str {
    std::str::from_utf8(&out.stdout).unwrap()
}

fn stderr(out: &Output) -> &str {
    std::str::from_utf8(&out.stderr).unwrap()
}

#[test]
fn share_serves_the_port_and_prints_its_https_address() {
    let ts = FakeTailscale::new("share", STATUS, "{}");
    let out = ts.tether(&["share", "5173"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "https://box.tail1.ts.net:5173\n");
    assert!(
        ts.calls()
            .contains("serve --bg --yes --https=5173 http://localhost:5173"),
        "{}",
        ts.calls()
    );
}

#[test]
fn share_refuses_a_tailnet_without_https_certificates() {
    let status =
        r#"{"BackendState":"Running","Self":{"DNSName":"box.tail1.ts.net."},"CertDomains":null}"#;
    let ts = FakeTailscale::new("share-nocert", status, "{}");
    let out = ts.tether(&["share", "5173"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).starts_with("tether: "), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("HTTPS Certificates"),
        "{}",
        stderr(&out)
    );
    assert!(!ts.calls().contains("serve --bg"), "{}", ts.calls());
}

#[test]
fn shares_lists_what_is_shared() {
    let ts = FakeTailscale::new("shares", STATUS, SERVING_5173);
    let out = ts.tether(&["shares"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "https://box.tail1.ts.net:5173 -> http://localhost:5173\n"
    );
}

#[test]
fn unshare_turns_a_shared_port_off() {
    let ts = FakeTailscale::new("unshare", STATUS, SERVING_5173);
    let out = ts.tether(&["unshare", "5173"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        ts.calls().contains("serve --https=5173 off"),
        "{}",
        ts.calls()
    );
}

#[test]
fn unshare_refuses_a_port_that_is_not_shared() {
    let ts = FakeTailscale::new("unshare-none", STATUS, SERVING_5173);
    let out = ts.tether(&["unshare", "3000"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "tether: port 3000 is not shared\n");
    assert!(!ts.calls().contains("off"), "{}", ts.calls());
}

#[test]
fn origin_prints_the_address_for_start_scripts() {
    let ts = FakeTailscale::new("origin", STATUS, "{}");
    let out = ts.tether(&["origin", "5173"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "https://box.tail1.ts.net:5173\n");
}
