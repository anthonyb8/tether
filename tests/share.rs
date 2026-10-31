//! `share`, `unshare`, `shares` and `origin`, run as a user runs them, against
//! a stand-in `tailscale` that answers from fixtures.

mod common;

use common::{Sandbox, stderr, stdout};

const STATUS: &str = r#"{"BackendState":"Running","Self":{"DNSName":"box.tail1.ts.net."},"CertDomains":["box.tail1.ts.net"]}"#;
const SERVING_5173: &str =
    r#"{"Web":{"box.tail1.ts.net:5173":{"Handlers":{"/":{"Proxy":"http://localhost:5173"}}}}}"#;

fn tailnet(test: &str, status: &str, serve_status: &str) -> Sandbox {
    let sandbox = Sandbox::new(test);
    let status = sandbox.write("status.json", status);
    let serve = sandbox.write("serve.json", serve_status);
    sandbox.fake(
        "tailscale",
        &format!(
            r#"case "$*" in
  "status --json") cat '{}' ;;
  "serve status --json") cat '{}' ;;
esac"#,
            status.display(),
            serve.display()
        ),
    );
    sandbox
}

#[test]
fn share_serves_the_port_and_prints_its_https_address() {
    let ts = tailnet("share", STATUS, "{}");
    let out = ts.run(&["share", "5173"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "https://box.tail1.ts.net:5173\n");
    let calls = ts.calls("tailscale");
    assert!(
        calls.contains("serve --bg --yes --https=5173 http://localhost:5173"),
        "{calls}"
    );
}

#[test]
fn share_refuses_a_tailnet_without_https_certificates() {
    let status =
        r#"{"BackendState":"Running","Self":{"DNSName":"box.tail1.ts.net."},"CertDomains":null}"#;
    let ts = tailnet("share-nocert", status, "{}");
    let out = ts.run(&["share", "5173"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).starts_with("tether: "), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("HTTPS Certificates"),
        "{}",
        stderr(&out)
    );
    assert!(!ts.calls("tailscale").contains("serve --bg"));
}

#[test]
fn shares_lists_what_is_shared() {
    let ts = tailnet("shares", STATUS, SERVING_5173);
    let out = ts.run(&["shares"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "https://box.tail1.ts.net:5173 -> http://localhost:5173\n"
    );
}

#[test]
fn unshare_turns_a_shared_port_off() {
    let ts = tailnet("unshare", STATUS, SERVING_5173);
    let out = ts.run(&["unshare", "5173"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(ts.calls("tailscale").contains("serve --https=5173 off"));
}

#[test]
fn unshare_refuses_a_port_that_is_not_shared() {
    let ts = tailnet("unshare-none", STATUS, SERVING_5173);
    let out = ts.run(&["unshare", "3000"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "tether: port 3000 is not shared\n");
    assert!(!ts.calls("tailscale").contains("off"));
}

#[test]
fn origin_prints_the_address_for_start_scripts() {
    let ts = tailnet("origin", STATUS, "{}");
    let out = ts.run(&["origin", "5173"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "https://box.tail1.ts.net:5173\n");
}
