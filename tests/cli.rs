//! The built binary, run the way a user runs it.

use std::process::Command;

#[test]
fn a_stubbed_subcommand_says_so_and_exits_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_tether"))
        .arg("share")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "tether: share is not built yet\n"
    );
}
