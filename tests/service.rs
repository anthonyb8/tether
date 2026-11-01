//! `tether connect` in the background, run as a user runs it, against a
//! stand-in for the system's service manager (systemctl on Linux, launchctl
//! on macOS). Starting the service brings the proxy port up, as the real
//! service's ssh would.

mod common;

use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use common::{Sandbox, stderr, stdout};

const MANAGER: &str = if cfg!(target_os = "macos") {
    "launchctl"
} else {
    "systemctl"
};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A sandbox whose service manager starts a listener on TETHER_PORT when
/// asked to start the service, and reports nothing running before that.
struct Laptop {
    sandbox: Sandbox,
    port: u16,
}

impl Laptop {
    fn new(test: &str) -> Laptop {
        let sandbox = Sandbox::new(test);
        let pids = sandbox.dir.join("pids");
        let body = format!(
            r#"case "$*" in
  *restart*|*bootstrap*)
    python3 -c "import socket,time; s=socket.socket(); s.bind(('127.0.0.1', $TETHER_PORT)); s.listen(); time.sleep(30)" >/dev/null 2>&1 &
    echo $! >> '{}' ;;
  *is-active*|print*) exit 1 ;;
esac"#,
            pids.display()
        );
        sandbox.fake("systemctl", &body);
        sandbox.fake("launchctl", &body);
        Laptop {
            sandbox,
            port: free_port(),
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.sandbox
            .tether(args)
            .env("TETHER_PORT", self.port.to_string())
            .output()
            .unwrap()
    }

    fn service_file(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.sandbox
                .dir
                .join("Library/LaunchAgents/dev.tether.connect.plist")
        } else {
            self.sandbox
                .dir
                .join(".config/systemd/user/tether-connect.service")
        }
    }
}

impl Drop for Laptop {
    fn drop(&mut self) {
        let pids = std::fs::read_to_string(self.sandbox.dir.join("pids")).unwrap_or_default();
        for pid in pids.split_whitespace() {
            let _ = std::process::Command::new("kill").arg(pid).status();
        }
    }
}

#[test]
fn connect_runs_in_the_background_and_returns_once_connected() {
    let laptop = Laptop::new("service-start");
    let out = laptop.run(&["connect", "devbox"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).starts_with("tether: connected to devbox, running in the background\n"),
        "{}",
        stdout(&out)
    );

    // The service runs this very binary in the foreground, with the host and
    // port it was given.
    let service = std::fs::read_to_string(laptop.service_file()).unwrap();
    let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_tether")).unwrap();
    if cfg!(target_os = "macos") {
        assert!(
            service.contains(&format!(
                "<string>{}</string>\n    <string>connect</string>\n    <string>--foreground</string>\n    <string>devbox</string>",
                exe.display()
            )),
            "{service}"
        );
        assert!(
            service.contains(&format!("<string>{}</string>", laptop.port)),
            "{service}"
        );
        assert!(laptop.sandbox.calls("launchctl").contains("bootstrap gui/"));
    } else {
        assert!(
            service.contains(&format!(
                "ExecStart=\"{}\" connect --foreground devbox\n",
                exe.display()
            )),
            "{service}"
        );
        assert!(
            service.contains(&format!("Environment=TETHER_PORT={}\n", laptop.port)),
            "{service}"
        );
        let calls = laptop.sandbox.calls("systemctl");
        assert!(
            calls.contains("--user enable tether-connect.service"),
            "{calls}"
        );
        assert!(
            calls.contains("--user restart tether-connect.service"),
            "{calls}"
        );
    }
}

#[test]
fn connect_stop_removes_the_background_connect() {
    let laptop = Laptop::new("service-stop");
    assert!(laptop.run(&["connect"]).status.success());

    let out = laptop.run(&["connect", "--stop"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!laptop.service_file().exists());
    let calls = laptop.sandbox.calls(MANAGER);
    if cfg!(target_os = "macos") {
        assert!(calls.contains("bootout gui/"), "{calls}");
    } else {
        assert!(
            calls.contains("--user disable --now tether-connect.service"),
            "{calls}"
        );
    }

    let out = laptop.run(&["connect", "--stop"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "tether: tether connect is not running in the background\n"
    );
}

#[test]
fn connect_refuses_while_a_foreground_connect_holds_the_port() {
    let laptop = Laptop::new("service-busy");
    let _foreground = TcpListener::bind(("127.0.0.1", laptop.port)).unwrap();
    let out = laptop.run(&["connect"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("stop it, then run tether connect again"),
        "{}",
        stderr(&out)
    );
    assert!(!laptop.service_file().exists());
}

#[test]
fn browser_starts_the_background_connect_when_nothing_is_connected() {
    let laptop = Laptop::new("service-browser");
    laptop.sandbox.fake("chromium", "");
    let out = laptop.run(&["browser", "devbox"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(laptop.service_file().exists());
    assert!(
        stdout(&out).contains("connected to devbox"),
        "{}",
        stdout(&out)
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while laptop.sandbox.calls("chromium").is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !laptop.sandbox.calls("chromium").is_empty(),
        "browser not opened"
    );
}
