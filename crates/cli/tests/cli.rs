//! Integration tests at the CLI process boundary: run the compiled
//! `uncompose-project` binary and assert on exit code and stdout/stderr.

use std::process::Command;

use uncompose_project_core::tagline;

#[test]
fn prints_the_banner_and_exits_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_uncompose-project"))
        .output()
        .expect("failed to run the uncompose-project binary");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("uncompose-project — {}\n", tagline())
    );
    assert!(output.stderr.is_empty());
}
