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
