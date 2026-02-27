use std::process::{Command, Stdio};

use tempfile::TempDir;

#[test]
fn wizard_requires_tty_and_does_not_write() {
    let temp = TempDir::new().expect("temp dir");
    let config_path = temp.path().join("trudger.yml");

    let output = Command::new(env!("CARGO_BIN_EXE_trudger"))
        .arg("wizard")
        .arg("--config")
        .arg(&config_path)
        // Ensure non-TTY invocation regardless of how the test runner is invoked.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run trudger wizard");

    assert!(
        !output.status.success(),
        "expected non-zero exit code, got: {:?}",
        output.status.code()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("interactive terminal"),
        "expected TTY error, got: {stderr:?}"
    );
    assert!(
        !config_path.exists(),
        "wizard should not write config when non-interactive"
    );
}

#[test]
fn wizard_rejects_manual_task_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_trudger"))
        .arg("wizard")
        .arg("-t")
        .arg("tr-1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run trudger wizard -t tr-1");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not supported in wizard mode"),
        "expected wizard -t error, got: {stderr:?}"
    );
}

#[test]
fn wizard_rejects_profile_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_trudger"))
        .arg("wizard")
        .arg("-p")
        .arg("review")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run trudger wizard -p review");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("-p/--profile is not supported in wizard mode."),
        "expected wizard profile error, got: {stderr:?}"
    );
}

#[test]
fn wizard_rejects_positional_args() {
    let output = Command::new(env!("CARGO_BIN_EXE_trudger"))
        .arg("wizard")
        .arg("extra")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run trudger wizard extra");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unexpected argument") && stderr.contains("Usage: trudger wizard"),
        "expected wizard positional error, got: {stderr:?}"
    );
}

#[test]
fn cli_parse_error_prints_helpful_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_trudger"))
        .arg("--definitely-not-a-real-flag")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run trudger with invalid args");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage:") || stderr.contains("USAGE") || stderr.contains("error:"),
        "expected clap usage/error output, got: {stderr:?}"
    );
}
