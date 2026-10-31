//! The server half, run as connect runs it: `tether relay` with the test
//! standing in for the laptop on its stdin and stdout, and `open` and
//! `status` run against it as an agent on the server would.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use common::{Sandbox, stderr, stdout};

struct Laptop {
    relay: Child,
    to_relay: ChildStdin,
    /// Each line the relay forwards, read on a thread so a request that never
    /// arrives fails the test rather than hanging it.
    from_relay: Receiver<String>,
}

impl Laptop {
    fn connect(sandbox: &Sandbox, name: &str) -> Laptop {
        let mut relay = sandbox
            .tether(&["relay", "--from", name])
            .env("XDG_RUNTIME_DIR", sandbox.dir.join("run"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (lines, from_relay) = mpsc::channel();
        let stdout = BufReader::new(relay.stdout.take().unwrap());
        thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                let _ = lines.send(line);
            }
        });
        let laptop = Laptop {
            to_relay: relay.stdin.take().unwrap(),
            from_relay,
            relay,
        };
        wait_for(|| status(sandbox).contains(name));
        laptop
    }

    /// Reads the next request the relay forwards, as the laptop would.
    fn next_request(&mut self) -> serde_json::Value {
        let line = self
            .from_relay
            .recv_timeout(Duration::from_secs(5))
            .expect("no request reached this laptop");
        serde_json::from_str(&line).unwrap()
    }

    fn has_no_request(&self) -> bool {
        self.from_relay.try_recv().is_err()
    }

    fn ack(&mut self, ack: serde_json::Value) {
        writeln!(self.to_relay, "{ack}").unwrap();
    }

    /// The laptop going away closes the relay's stdin.
    fn disconnect(mut self) {
        drop(self.to_relay);
        self.relay.wait().unwrap();
    }
}

fn sandbox(test: &str) -> Sandbox {
    let sandbox = Sandbox::new(test);
    std::fs::create_dir_all(sandbox.dir.join("run")).unwrap();
    sandbox
}

fn run(sandbox: &Sandbox, args: &[&str]) -> std::process::Output {
    sandbox
        .tether(args)
        .env("XDG_RUNTIME_DIR", sandbox.dir.join("run"))
        .output()
        .unwrap()
}

fn status(sandbox: &Sandbox) -> String {
    stdout(&run(sandbox, &["status"]))
}

fn wait_for(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn open_reaches_the_laptop_and_exits_0_on_its_ack() {
    let sandbox = sandbox("relay-open");
    let mut laptop = Laptop::connect(&sandbox, "mac");

    let open = {
        let sandbox_dir = sandbox.dir.clone();
        let mut cmd = sandbox.tether(&["open", "http://localhost:5173/"]);
        cmd.env("XDG_RUNTIME_DIR", sandbox_dir.join("run"));
        thread::spawn(move || cmd.output().unwrap())
    };
    let request = laptop.next_request();
    assert_eq!(request["type"], "open");
    assert_eq!(request["url"], "http://localhost:5173/");
    laptop.ack(serde_json::json!({ "id": request["id"] }));

    let out = open.join().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    laptop.disconnect();
}

#[test]
fn open_reports_the_laptops_reason_when_it_fails() {
    let sandbox = sandbox("relay-open-fails");
    let mut laptop = Laptop::connect(&sandbox, "mac");
    let open = {
        let mut cmd = sandbox.tether(&["open", "http://localhost:5173/"]);
        cmd.env("XDG_RUNTIME_DIR", sandbox.dir.join("run"));
        thread::spawn(move || cmd.output().unwrap())
    };
    let request = laptop.next_request();
    laptop.ack(serde_json::json!({ "id": request["id"], "error": "no browser installed" }));
    let out = open.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "tether: no browser installed\n");
    laptop.disconnect();
}

#[test]
fn open_gives_up_on_a_laptop_that_does_not_answer() {
    let sandbox = sandbox("relay-open-silent");
    let mut laptop = Laptop::connect(&sandbox, "mac");
    let started = Instant::now();
    let out = run(&sandbox, &["open", "http://localhost:5173/"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "tether: the laptop did not answer\n");
    assert!(started.elapsed() < Duration::from_secs(4));
    laptop.next_request();
    laptop.disconnect();
}

#[test]
fn with_no_laptop_open_and_status_exit_1_so_callers_fall_back() {
    let sandbox = sandbox("relay-none");
    let out = run(&sandbox, &["open", "http://localhost:5173/"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "tether: no browser connected\n");

    let out = run(&sandbox, &["status"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "tether: no laptop connected\n");
}

#[test]
fn open_refuses_anything_but_a_web_url_before_reaching_the_laptop() {
    let sandbox = sandbox("relay-file-url");
    let out = run(&sandbox, &["open", "file:///etc/passwd"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("only http and https"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn the_newest_laptop_gets_the_tabs_and_the_older_one_takes_over_when_it_leaves() {
    let sandbox = sandbox("relay-replace");
    let mut older = Laptop::connect(&sandbox, "linux-laptop");
    let mut newer = Laptop::connect(&sandbox, "mac");
    assert!(status(&sandbox).starts_with("connected: mac,"));
    open_on(&sandbox, &mut newer, "http://localhost:5173/");
    assert!(older.has_no_request(), "the older laptop got the tab too");

    newer.disconnect();
    wait_for(|| status(&sandbox).starts_with("connected: linux-laptop,"));
    open_on(&sandbox, &mut older, "http://localhost:3000/");
    older.disconnect();
    wait_for(|| run(&sandbox, &["status"]).status.code() == Some(1));
}

/// Runs `open` on the server and checks `laptop` is the one it reached.
fn open_on(sandbox: &Sandbox, laptop: &mut Laptop, url: &str) {
    let open = {
        let mut cmd = sandbox.tether(&["open", url]);
        cmd.env("XDG_RUNTIME_DIR", sandbox.dir.join("run"));
        thread::spawn(move || cmd.output().unwrap())
    };
    let request = laptop.next_request();
    assert_eq!(request["url"], url);
    laptop.ack(serde_json::json!({ "id": request["id"] }));
    let out = open.join().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn notify_sends_its_words_as_one_message() {
    let sandbox = sandbox("relay-notify");
    let mut laptop = Laptop::connect(&sandbox, "mac");
    let notify = {
        let mut cmd = sandbox.tether(&["notify", "build", "finished:", "ok"]);
        cmd.env("XDG_RUNTIME_DIR", sandbox.dir.join("run"));
        thread::spawn(move || cmd.output().unwrap())
    };
    let request = laptop.next_request();
    assert_eq!(request["type"], "notify");
    assert_eq!(request["text"], "build finished: ok");
    laptop.ack(serde_json::json!({ "id": request["id"] }));
    let out = notify.join().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    laptop.disconnect();
}
