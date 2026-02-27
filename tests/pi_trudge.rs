use std::fs;
use std::path::Path;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn pi_trudge_binary_is_packaged_with_trudger() {
    let trudger = option_env!("CARGO_BIN_EXE_trudger").expect("CARGO_BIN_EXE_trudger");
    let pi_trudge = option_env!("CARGO_BIN_EXE_pi_trudge").expect("CARGO_BIN_EXE_pi_trudge");

    assert!(Path::new(trudger).exists());
    assert!(Path::new(pi_trudge).exists());
}

#[cfg(unix)]
#[test]
fn pi_trudge_uses_path_resolved_pi_command_without_resume_flags() {
    let pi_trudge = option_env!("CARGO_BIN_EXE_pi_trudge").expect("CARGO_BIN_EXE_pi_trudge");
    let temp = tempfile::tempdir().expect("temp dir");

    let pi_script = temp.path().join("pi");
    let call_log = temp.path().join("pi.invocations");

    let script = format!(
        "#!/bin/sh\nlog='{}'\nif [ \"$1\" = \"--prompt\" ]; then\n  for arg in \"$@\"; do\n    printf '%s\\n' \"$arg\" >> \"$log\"\n  done\n  exit 0\nfi\nexit 1\n",
        call_log.display()
    );
    fs::write(&pi_script, script).expect("write pi script");

    let mut perms = fs::metadata(&pi_script).expect("pi metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&pi_script, perms).expect("chmod pi script");

    let pi_path = temp.path().to_string_lossy().to_string();

    let output = Command::new(pi_trudge)
        .arg("--prompt-env")
        .arg("TRUDGER_AGENT_PROMPT")
        .env("PATH", &pi_path)
        .env("TRUDGER_AGENT_PROMPT", "z-ai prompt")
        .output()
        .expect("run pi_trudge");

    assert!(output.status.success());
    let lines: Vec<String> = fs::read_to_string(&call_log)
        .expect("read invocation log")
        .lines()
        .map(|line| line.to_string())
        .collect();

    assert!(
        lines.contains(&"--prompt".to_string()),
        "expected pi invocation to include --prompt, got: {:?}",
        lines
    );
    assert!(
        lines.contains(&"z-ai prompt".to_string()),
        "expected prompt payload to be forwarded, got: {:?}",
        lines
    );
    assert!(
        !lines
            .iter()
            .any(|line| line == "resume" || line == "--resume" || line == "resume --last"),
        "expected stateless invocation with no resume flags, got: {:?}",
        lines
    );
}
