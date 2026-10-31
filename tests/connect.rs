//! `connect`, run as a user runs it, against a stand-in `ssh`.

mod common;

use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use common::{Sandbox, stderr};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn connect_runs_the_socks_proxy_and_reconnects_when_it_drops() {
    let sandbox = Sandbox::new("connect-reconnect");
    // An ssh whose connection drops at once.
    sandbox.fake("ssh", "exit 255");
    let port = free_port().to_string();
    let mut child = sandbox
        .tether(&["connect", "devbox"])
        .env("TETHER_PORT", &port)
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while sandbox.calls("ssh").lines().count() < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();

    let calls = sandbox.calls("ssh");
    assert!(calls.lines().count() >= 2, "did not reconnect: {calls:?}");
    let first = calls.lines().next().unwrap();
    assert!(first.contains(&format!("-D 127.0.0.1:{port}")), "{first}");
    assert!(first.contains("ExitOnForwardFailure=yes"), "{first}");
    assert!(
        first.ends_with("devbox") || first.contains(" devbox "),
        "{first}"
    );
}

#[test]
fn connect_refuses_a_proxy_port_already_in_use() {
    let sandbox = Sandbox::new("connect-busy");
    sandbox.fake("ssh", "exit 255");
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port().to_string();

    let out = sandbox
        .tether(&["connect"])
        .env("TETHER_PORT", &port)
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains(&format!("127.0.0.1:{port} is in use")),
        "{}",
        stderr(&out)
    );
    assert_eq!(sandbox.calls("ssh"), "");
}

#[test]
fn connect_takes_the_host_from_the_config_file() {
    let sandbox = Sandbox::new("connect-config");
    sandbox.fake("ssh", "exit 255");
    std::fs::create_dir_all(sandbox.dir.join(".config/tether")).unwrap();
    let port = free_port();
    sandbox.write(
        ".config/tether/config.toml",
        &format!("host = \"from-config\"\nport = {port}\n"),
    );
    let mut child = sandbox.tether(&["connect"]).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while sandbox.calls("ssh").is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let calls = sandbox.calls("ssh");
    assert!(calls.contains(&format!("-D 127.0.0.1:{port}")), "{calls}");
    assert!(calls.contains("from-config"), "{calls}");
}

#[test]
fn connect_opens_what_the_server_asks_and_acks_it() {
    let sandbox = Sandbox::new("connect-open");
    let acks = sandbox.dir.join("acks");
    // An ssh whose relay sends one request, records the laptop's ack, then
    // drops.
    sandbox.fake(
        "ssh",
        &format!(
            r#"echo 'welcome from a noisy .bashrc'
echo '{{"id":4,"type":"open","url":"http://localhost:5173/"}}'
read ack; echo "$ack" >> '{}'
exit 255"#,
            acks.display()
        ),
    );
    sandbox.fake("chromium", "");
    let port = free_port();
    let mut child = sandbox
        .tether(&["connect"])
        .env("TETHER_PORT", port.to_string())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !acks.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();

    // The tether profile, proxied with localhost included, at the URL.
    let launched = sandbox.calls("chromium");
    let profile = sandbox.dir.join(".config/tether/chromium");
    assert!(
        launched.starts_with(&format!("--user-data-dir={}", profile.display())),
        "{launched}"
    );
    assert!(
        launched.contains(&format!("--proxy-server=socks5://127.0.0.1:{port}")),
        "{launched}"
    );
    assert!(
        launched.contains("--proxy-bypass-list=<-loopback>"),
        "{launched}"
    );
    assert!(
        launched.ends_with(" http://localhost:5173/\n"),
        "{launched}"
    );
    assert_eq!(std::fs::read_to_string(&acks).unwrap(), "{\"id\":4}\n");
}

#[test]
fn connect_keeps_the_proxy_when_the_server_has_no_tether() {
    let sandbox = Sandbox::new("connect-no-relay");
    // A server without tether: the remote shell cannot find it.
    sandbox.fake(
        "ssh",
        r#"case "$*" in *"tether relay"*) exit 127 ;; *) exit 255 ;; esac"#,
    );
    let mut child = sandbox
        .tether(&["connect"])
        .env("TETHER_PORT", free_port().to_string())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while sandbox.calls("ssh").lines().count() < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    let out = child.wait_with_output().unwrap();

    let calls = sandbox.calls("ssh");
    let second = calls.lines().nth(1).expect("did not retry");
    assert!(second.starts_with("-N "), "{second}");
    assert!(!second.contains("relay"), "{second}");
    assert!(
        stderr(&out).contains("tether is not installed on server"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn browser_opens_the_profile_through_a_running_connect() {
    let sandbox = Sandbox::new("browser-running");
    sandbox.fake("ssh", "exit 255");
    sandbox.fake("chromium", "");
    // A connect already holding the proxy port.
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = proxy.local_addr().unwrap().port();

    let out = sandbox
        .tether(&["browser"])
        .env("TETHER_PORT", port.to_string())
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    let deadline = Instant::now() + Duration::from_secs(5);
    while sandbox.calls("chromium").is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let launched = sandbox.calls("chromium");
    assert!(
        launched.contains(&format!("--proxy-server=socks5://127.0.0.1:{port}")),
        "{launched}"
    );
    assert_eq!(sandbox.calls("ssh"), "", "started a second connect");
}

#[test]
fn browser_starts_connect_in_the_background_when_it_is_not_running() {
    let sandbox = Sandbox::new("browser-cold");
    sandbox.fake("chromium", "");
    let pids = sandbox.dir.join("pids");
    // An ssh that brings the SOCKS port up as the real one would, recording
    // its own pid and its parent's (the background connect) for cleanup.
    sandbox.fake(
        "ssh",
        &format!(
            r#"echo "$$ $PPID" > '{}'
port=$(echo "$*" | sed -n 's/.*-D 127.0.0.1:\([0-9]*\).*/\1/p')
exec python3 -c "import socket,time; s=socket.socket(); s.bind(('127.0.0.1', $port)); s.listen(); time.sleep(30)""#,
            pids.display()
        ),
    );
    let port = free_port();

    let out = sandbox
        .tether(&["browser", "devbox"])
        .env("TETHER_PORT", port.to_string())
        .output()
        .unwrap();

    let started = std::fs::read_to_string(&pids).unwrap_or_default();
    for pid in started.split_whitespace() {
        let _ = std::process::Command::new("kill").arg(pid).status();
    }
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("started tether connect in the background"),
        "{}",
        stderr(&out)
    );
    assert!(
        sandbox.calls("ssh").contains(" devbox "),
        "{}",
        sandbox.calls("ssh")
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while sandbox.calls("chromium").is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(!sandbox.calls("chromium").is_empty(), "browser not opened");
}
