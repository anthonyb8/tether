//! `connect`, run as a user runs it, against a stand-in `ssh`.

mod common;

use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use common::{Sandbox, stderr, stdout};

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
        .tether(&["connect", "--foreground", "devbox"])
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
        .tether(&["connect", "--foreground"])
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
    let mut child = sandbox
        .tether(&["connect", "--foreground"])
        .spawn()
        .unwrap();
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
        .tether(&["connect", "--foreground"])
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
        .tether(&["connect", "--foreground"])
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
    sandbox.fake("systemctl", "exit 1");
    sandbox.fake("launchctl", "exit 1");
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
    let manager = sandbox.calls("systemctl") + &sandbox.calls("launchctl");
    assert!(
        !manager.contains("restart") && !manager.contains("bootstrap"),
        "started the background connect: {manager}"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while sandbox.calls("chromium").is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let launched = sandbox.calls("chromium");
    assert!(
        launched.contains(&format!("--proxy-server=socks5://127.0.0.1:{port}")),
        "{launched}"
    );
}

#[test]
fn connect_shows_the_servers_notification_and_acks_it() {
    let sandbox = Sandbox::new("connect-notify");
    let acks = sandbox.dir.join("acks");
    sandbox.fake(
        "ssh",
        &format!(
            r#"echo '{{"id":9,"type":"notify","text":"tests passed \"quoted\""}}'
read ack; echo "$ack" >> '{}'
exit 255"#,
            acks.display()
        ),
    );
    sandbox.fake("notify-send", "");
    sandbox.fake("osascript", "");
    let mut child = sandbox
        .tether(&["connect", "--foreground"])
        .env("TETHER_PORT", free_port().to_string())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !acks.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();

    let shown = if cfg!(target_os = "macos") {
        sandbox.calls("osascript")
    } else {
        sandbox.calls("notify-send")
    };
    assert!(shown.ends_with("tests passed \"quoted\"\n"), "{shown}");
    assert_eq!(std::fs::read_to_string(&acks).unwrap(), "{\"id\":9}\n");
}

#[test]
fn a_firefox_laptop_gets_a_proxied_profile_and_the_tab() {
    let sandbox = Sandbox::new("connect-firefox");
    let acks = sandbox.dir.join("acks");
    sandbox.fake(
        "ssh",
        &format!(
            r#"echo '{{"id":1,"type":"open","url":"http://localhost:3000/"}}'
read ack; echo "$ack" >> '{}'
exit 255"#,
            acks.display()
        ),
    );
    sandbox.fake("firefox", "");
    std::fs::create_dir_all(sandbox.dir.join(".config/tether")).unwrap();
    sandbox.write(".config/tether/config.toml", "browser = \"firefox\"\n");
    let port = free_port();
    let mut child = sandbox
        .tether(&["connect", "--foreground"])
        .env("TETHER_PORT", port.to_string())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !acks.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();

    let profile = sandbox.dir.join(".config/tether/firefox");
    assert_eq!(
        sandbox.calls("firefox"),
        format!(
            "--profile {} --new-tab http://localhost:3000/\n",
            profile.display()
        )
    );
    let prefs = std::fs::read_to_string(profile.join("user.js")).unwrap();
    assert!(
        prefs.contains(&format!(
            r#"user_pref("network.proxy.socks_port", {port});"#
        )),
        "{prefs}"
    );
    assert!(
        prefs.contains(r#"user_pref("network.proxy.allow_hijacking_localhost", true);"#),
        "{prefs}"
    );
    assert_eq!(std::fs::read_to_string(&acks).unwrap(), "{\"id\":1}\n");
}

/// Runs `tether status` in the sandbox, with the runtime directory a
/// foreground connect in it uses.
fn status(sandbox: &Sandbox, port: u16) -> std::process::Output {
    sandbox
        .tether(&["status"])
        .env("TETHER_PORT", port.to_string())
        .env("XDG_RUNTIME_DIR", sandbox.dir.join("run"))
        .output()
        .unwrap()
}

fn wait_for(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(50));
    }
}

/// An ssh that logs in and opens the proxy, as the real one does, until
/// killed. Its pid is written to `ssh.pid`.
fn listening_ssh(sandbox: &Sandbox) {
    sandbox.fake(
        "ssh",
        &format!(
            r#"echo $$ > '{}'
port=$(echo "$*" | sed -n 's/.*-D 127.0.0.1:\([0-9]*\).*/\1/p')
exec python3 -c "import socket,time; s=socket.socket(); s.bind(('127.0.0.1', $port)); s.listen(); time.sleep(30)""#,
            sandbox.dir.join("ssh.pid").display()
        ),
    );
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .status()
        .unwrap()
        .success()
}

#[test]
fn status_on_the_laptop_shows_the_connection() {
    let sandbox = Sandbox::new("connect-status");
    std::fs::create_dir_all(sandbox.dir.join("run")).unwrap();
    listening_ssh(&sandbox);
    let port = free_port();

    let out = status(&sandbox, port);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("this machine is not connected to a server"),
        "{}",
        stderr(&out)
    );

    let mut child = sandbox
        .tether(&["connect", "--foreground", "devbox"])
        .env("TETHER_PORT", port.to_string())
        .env("XDG_RUNTIME_DIR", sandbox.dir.join("run"))
        .env_remove("INVOCATION_ID")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for(|| status(&sandbox, port).status.success());
    let out = status(&sandbox, port);
    assert!(
        stdout(&out)
            .starts_with("connected to devbox, in a terminal (tether connect --foreground), for "),
        "{}",
        stdout(&out)
    );

    child.kill().unwrap();
    child.wait().unwrap();
    let ssh = std::fs::read_to_string(sandbox.dir.join("ssh.pid")).unwrap();
    let _ = std::process::Command::new("kill").arg(ssh.trim()).status();
}

#[test]
fn status_on_the_laptop_names_a_port_held_by_something_else() {
    let sandbox = Sandbox::new("connect-status-other");
    std::fs::create_dir_all(sandbox.dir.join("run")).unwrap();
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let out = status(&sandbox, port);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains(&format!("something else holds 127.0.0.1:{port}")),
        "{}",
        stderr(&out)
    );
}

#[test]
fn stopping_connect_stops_its_ssh() {
    let sandbox = Sandbox::new("connect-stops-ssh");
    std::fs::create_dir_all(sandbox.dir.join("run")).unwrap();
    listening_ssh(&sandbox);
    let port = free_port();
    let mut child = sandbox
        .tether(&["connect", "--foreground"])
        .env("TETHER_PORT", port.to_string())
        .env("XDG_RUNTIME_DIR", sandbox.dir.join("run"))
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for(|| sandbox.dir.join("ssh.pid").exists());
    let ssh = std::fs::read_to_string(sandbox.dir.join("ssh.pid")).unwrap();
    wait_for(|| TcpListener::bind(("127.0.0.1", port)).is_err());

    // As `kill` or a closed terminal would, without reaching ssh directly.
    std::process::Command::new("kill")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    child.wait().unwrap();
    wait_for(|| !alive(&ssh));
    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "the proxy port is still held"
    );
}
