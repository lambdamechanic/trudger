use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::config::Config;
use crate::logger::Logger;
use crate::run_loop::{is_ready_status, quit, validate_config, Quit};
use crate::shell::{run_shell_command_capture, run_shell_command_status, CommandEnv};

#[derive(Debug, serde::Deserialize)]
struct DoctorIssueSnapshot {
    id: String,
    #[serde(default)]
    status: String,
}

fn load_doctor_issue_statuses(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("doctor failed to read issues {}: {}", path.display(), err))?;
    let reader = BufReader::new(file);
    let mut latest: BTreeMap<String, String> = BTreeMap::new();

    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|err| {
            format!(
                "doctor failed to read issues {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let snapshot: DoctorIssueSnapshot = serde_json::from_str(trimmed).map_err(|err| {
            format!(
                "doctor failed to parse issues {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        latest.insert(snapshot.id, snapshot.status);
    }

    Ok(latest)
}

fn build_doctor_env(
    config_path: &Path,
    scratch_path: &str,
    cwd: &Path,
    task_id: Option<&str>,
    task_show: Option<&str>,
    task_status: Option<&str>,
) -> CommandEnv {
    CommandEnv {
        cwd: Some(cwd.to_path_buf()),
        config_path: config_path.display().to_string(),
        scratch_dir: Some(scratch_path.to_string()),
        task_id: task_id.map(|value| value.to_string()),
        task_show: task_show.map(|value| value.to_string()),
        task_status: task_status.map(|value| value.to_string()),
        prompt: None,
        review_prompt: None,
        completed: None,
        needs_human: None,
    }
}

fn doctor_run_next_task(
    config: &Config,
    config_path: &Path,
    scratch_dir: &Path,
    scratch_path: &str,
    logger: &Logger,
) -> Result<(), String> {
    let next_task = config.commands.next_task.as_deref().unwrap_or("").trim();
    if next_task.is_empty() {
        return Err("commands.next_task must not be empty.".to_string());
    }
    let env = build_doctor_env(config_path, scratch_path, scratch_dir, None, None, None);
    let output =
        run_shell_command_capture(next_task, "doctor-next-task", "none", &[], &env, logger)?;
    match output.exit_code {
        0 => {
            // Empty output is valid ("no tasks") in Trudger semantics.
            let _task_id = output.stdout.split_whitespace().next().unwrap_or("");
        }
        1 => {
            // Exit 1 means "no selectable tasks" in Trudger semantics.
        }
        code => {
            return Err(format!("commands.next_task failed with exit code {}", code));
        }
    }
    Ok(())
}

fn doctor_run_task_show(
    config: &Config,
    config_path: &Path,
    scratch_dir: &Path,
    scratch_path: &str,
    task_id: &str,
    logger: &Logger,
) -> Result<String, String> {
    let env = build_doctor_env(
        config_path,
        scratch_path,
        scratch_dir,
        Some(task_id),
        None,
        None,
    );
    let output = run_shell_command_capture(
        &config.commands.task_show,
        "doctor-task-show",
        task_id,
        &[],
        &env,
        logger,
    )?;
    if output.exit_code != 0 {
        return Err(format!(
            "commands.task_show failed with exit code {}",
            output.exit_code
        ));
    }
    Ok(output.stdout)
}

fn doctor_run_task_status(
    config: &Config,
    config_path: &Path,
    scratch_dir: &Path,
    scratch_path: &str,
    task_id: &str,
    logger: &Logger,
) -> Result<String, String> {
    let env = build_doctor_env(
        config_path,
        scratch_path,
        scratch_dir,
        Some(task_id),
        None,
        None,
    );
    let output = run_shell_command_capture(
        &config.commands.task_status,
        "doctor-task-status",
        task_id,
        &[],
        &env,
        logger,
    )?;
    if output.exit_code != 0 {
        return Err(format!(
            "commands.task_status failed with exit code {}",
            output.exit_code
        ));
    }
    let status = output
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if status.is_empty() {
        return Err("commands.task_status returned an empty status.".to_string());
    }
    Ok(status)
}

fn doctor_run_task_update_status(
    config: &Config,
    config_path: &Path,
    scratch_dir: &Path,
    scratch_path: &str,
    task_id: &str,
    status: &str,
    logger: &Logger,
) -> Result<(), String> {
    let env = build_doctor_env(
        config_path,
        scratch_path,
        scratch_dir,
        Some(task_id),
        None,
        None,
    );
    let args = vec!["--status".to_string(), status.to_string()];
    let exit = run_shell_command_status(
        &config.commands.task_update_in_progress,
        "doctor-task-update",
        task_id,
        &args,
        &env,
        logger,
    )?;
    if exit != 0 {
        return Err(format!(
            "commands.task_update_in_progress failed to set status {} (exit code {})",
            status, exit
        ));
    }
    Ok(())
}

fn doctor_run_reset_task(
    config: &Config,
    config_path: &Path,
    scratch_dir: &Path,
    scratch_path: &str,
    task_id: &str,
    logger: &Logger,
) -> Result<(), String> {
    let env = build_doctor_env(
        config_path,
        scratch_path,
        scratch_dir,
        Some(task_id),
        None,
        None,
    );
    let exit = run_shell_command_status(
        &config.commands.reset_task,
        "doctor-reset-task",
        task_id,
        &[],
        &env,
        logger,
    )?;
    if exit != 0 {
        return Err(format!(
            "commands.reset_task failed with exit code {}",
            exit
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn doctor_run_hook(
    hook_command: &str,
    config_path: &Path,
    scratch_dir: &Path,
    scratch_path: &str,
    task_id: &str,
    task_show: &str,
    task_status: &str,
    hook_name: &str,
    logger: &Logger,
) -> Result<(), String> {
    let env = build_doctor_env(
        config_path,
        scratch_path,
        scratch_dir,
        Some(task_id),
        Some(task_show),
        Some(task_status),
    );
    let exit = run_shell_command_status(hook_command, hook_name, task_id, &[], &env, logger)?;
    if exit != 0 {
        return Err(format!("hook {} failed with exit code {}", hook_name, exit));
    }
    Ok(())
}

fn run_doctor_checks(
    config: &Config,
    config_path: &Path,
    scratch_dir: &Path,
    scratch_path: &str,
    logger: &Logger,
) -> Result<(), String> {
    let beads_dir = scratch_dir.join(".beads");
    let issues_path = beads_dir.join("issues.jsonl");
    if !issues_path.is_file() {
        return Err(format!(
            "doctor scratch DB is missing {}.\nExpected hooks.on_doctor_setup to create $TRUDGER_DOCTOR_SCRATCH_DIR/.beads with issues.jsonl.",
            issues_path.display()
        ));
    }

    doctor_run_next_task(config, config_path, scratch_dir, scratch_path, logger)?;

    let statuses = load_doctor_issue_statuses(&issues_path)?;
    let any_task_id = statuses
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| "doctor scratch DB has no issues in issues.jsonl.".to_string())?;
    let task_id = statuses
        .iter()
        .find_map(|(id, status)| {
            if is_ready_status(status) {
                Some(id.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| any_task_id.clone());

    let closed_task_id = statuses.iter().find_map(|(id, status)| {
        if status == "closed" {
            Some(id.clone())
        } else {
            None
        }
    });

    // Verify reset -> ready/open parsing.
    doctor_run_reset_task(
        config,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        logger,
    )?;
    let status = doctor_run_task_status(
        config,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        logger,
    )?;
    if !is_ready_status(&status) {
        return Err(format!(
            "doctor expected commands.task_status to return ready/open after reset_task, got '{}'.",
            status
        ));
    }

    // Verify show runs successfully (content is prompt-only in run mode).
    let show = doctor_run_task_show(
        config,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        logger,
    )?;

    // Verify update -> in_progress parsing.
    doctor_run_task_update_status(
        config,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        "in_progress",
        logger,
    )?;
    let status = doctor_run_task_status(
        config,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        logger,
    )?;
    if status != "in_progress" {
        return Err(format!(
            "doctor expected commands.task_status to return 'in_progress' after task_update_in_progress, got '{}'.",
            status
        ));
    }

    // Verify reset works again and yields ready/open.
    doctor_run_reset_task(
        config,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        logger,
    )?;
    let status = doctor_run_task_status(
        config,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        logger,
    )?;
    if !is_ready_status(&status) {
        return Err(format!(
            "doctor expected commands.task_status to return ready/open after reset_task, got '{}'.",
            status
        ));
    }

    // Verify completion/escalation hooks are runnable in the scratch DB environment.
    doctor_run_hook(
        &config.hooks.on_completed,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        &show,
        &status,
        "doctor-hook-on-completed",
        logger,
    )?;
    doctor_run_hook(
        &config.hooks.on_requires_human,
        config_path,
        scratch_dir,
        scratch_path,
        &task_id,
        &show,
        &status,
        "doctor-hook-on-requires-human",
        logger,
    )?;

    // Verify closed parsing.
    match closed_task_id {
        Some(closed_task_id) => {
            let closed_status = doctor_run_task_status(
                config,
                config_path,
                scratch_dir,
                scratch_path,
                &closed_task_id,
                logger,
            )?;
            if closed_status != "closed" {
                return Err(format!(
                    "doctor expected commands.task_status to return 'closed' for task {}, got '{}'.",
                    closed_task_id, closed_status
                ));
            }
        }
        None => {
            doctor_run_task_update_status(
                config,
                config_path,
                scratch_dir,
                scratch_path,
                &task_id,
                "closed",
                logger,
            )?;
            let closed_status = doctor_run_task_status(
                config,
                config_path,
                scratch_dir,
                scratch_path,
                &task_id,
                logger,
            )?;
            if closed_status != "closed" {
                return Err(format!(
                    "doctor expected commands.task_status to return 'closed' after setting status closed, got '{}'.",
                    closed_status
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn run_doctor_mode(
    config: &Config,
    config_path: &Path,
    logger: &Logger,
) -> Result<(), Quit> {
    let invocation_cwd = env::current_dir()
        .map_err(|err| quit(logger, &format!("doctor_invocation_cwd_failed:{err}"), 1))?;

    if let Err(message) = validate_config(config, &[]) {
        eprintln!("{}", message);
        return Err(quit(logger, &message, 1));
    }

    let hook = config
        .hooks
        .on_doctor_setup
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    if hook.is_empty() {
        let message = "hooks.on_doctor_setup must not be empty.".to_string();
        eprintln!("{}", message);
        return Err(quit(logger, &message, 1));
    }

    let scratch = tempfile::Builder::new()
        .prefix("trudger-doctor-")
        .tempdir()
        .map_err(|err| quit(logger, &format!("doctor_scratch_create_failed:{err}"), 1))?;
    let scratch_dir = scratch.path().to_path_buf();
    let scratch_path = scratch_dir.display().to_string();

    let env = CommandEnv {
        cwd: Some(invocation_cwd.clone()),
        config_path: config_path.display().to_string(),
        scratch_dir: Some(scratch_path.clone()),
        task_id: None,
        task_show: None,
        task_status: None,
        prompt: None,
        review_prompt: None,
        completed: None,
        needs_human: None,
    };

    let hook_exit = run_shell_command_status(&hook, "doctor-setup", "none", &[], &env, logger);
    let hook_result = match hook_exit {
        Ok(0) => Ok(()),
        Ok(exit) => Err(format!(
            "hooks.on_doctor_setup failed with exit code {}",
            exit
        )),
        Err(err) => Err(err),
    };

    let doctor_result = match hook_result {
        Ok(()) => run_doctor_checks(config, config_path, &scratch_dir, &scratch_path, logger),
        Err(err) => Err(err),
    };

    if let Err(err) = env::set_current_dir(&invocation_cwd) {
        let message = format!(
            "doctor failed to restore invocation working directory {}: {}",
            invocation_cwd.display(),
            err
        );
        eprintln!("{}", message);
        return Err(quit(logger, &message, 1));
    }
    let cleanup_result = scratch.close();

    if let Err(err) = cleanup_result {
        let message = format!("doctor scratch cleanup failed: {}", err);
        eprintln!("{}", message);
        return Err(quit(logger, &message, 1));
    }

    if let Err(err) = doctor_result {
        eprintln!("{}", err);
        return Err(quit(logger, &err, 1));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn base_config() -> Config {
        Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: crate::config::Commands {
                next_task: Some("exit 0".to_string()),
                task_show: "printf 'SHOW'".to_string(),
                task_status: "printf 'open\\n'".to_string(),
                task_update_in_progress: "exit 0".to_string(),
                reset_task: "exit 0".to_string(),
            },
            hooks: crate::config::Hooks {
                on_completed: "exit 0".to_string(),
                on_requires_human: "exit 0".to_string(),
                on_doctor_setup: Some("exit 0".to_string()),
            },
            review_loop_limit: 1,
            log_path: "".to_string(),
        }
    }

    fn scratch_with_issues(contents: &str) -> TempDir {
        let scratch = TempDir::new().expect("scratch");
        let beads_dir = scratch.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");
        fs::write(beads_dir.join("issues.jsonl"), contents).expect("write issues.jsonl");
        scratch
    }

    #[test]
    fn load_doctor_issue_statuses_errors_when_open_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("missing.jsonl");
        let err = load_doctor_issue_statuses(&path).expect_err("expected open error");
        assert!(err.contains("doctor failed to read issues"));
    }

    #[test]
    fn load_doctor_issue_statuses_errors_when_reading_lines_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let dir_path = temp.path().join("issues.jsonl");
        fs::create_dir_all(&dir_path).expect("create issues dir");
        let err = load_doctor_issue_statuses(&dir_path).expect_err("expected read error");
        assert!(err.contains("doctor failed to read issues"));
        assert!(err.contains("line 1"));
    }

    #[test]
    fn load_doctor_issue_statuses_skips_blank_lines() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues(
            r#"{"id":"tr-1","status":"open"}

{"id":"tr-2","status":"blocked"}
"#,
        );
        let statuses =
            load_doctor_issue_statuses(&scratch.path().join(".beads").join("issues.jsonl"))
                .expect("load");
        assert!(statuses.contains_key("tr-1"));
        assert!(statuses.contains_key("tr-2"));
    }

    #[test]
    fn load_doctor_issue_statuses_errors_on_invalid_json() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{bad json}\n");
        let err = load_doctor_issue_statuses(&scratch.path().join(".beads").join("issues.jsonl"))
            .expect_err("expected parse error");
        assert!(err.contains("doctor failed to parse issues"));
    }

    #[test]
    fn doctor_run_next_task_errors_when_command_is_empty() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.next_task = None;
        let logger = Logger::new(None);
        let err = doctor_run_next_task(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected empty next_task error");
        assert!(err.contains("commands.next_task must not be empty"));
    }

    #[test]
    fn doctor_run_next_task_accepts_exit_code_1() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.next_task = Some("exit 1".to_string());
        let logger = Logger::new(None);
        doctor_run_next_task(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect("exit 1 should be ok");
    }

    #[test]
    fn doctor_run_next_task_errors_on_other_nonzero_exit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.next_task = Some("exit 2".to_string());
        let logger = Logger::new(None);
        let err = doctor_run_next_task(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected exit code error");
        assert!(err.contains("exit code 2"));
    }

    #[cfg(unix)]
    #[test]
    fn doctor_run_next_task_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.next_task = Some("next-task".to_string());
        let logger = Logger::new(None);

        env::set_var("PATH", temp.path());
        let err = doctor_run_next_task(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn doctor_run_task_show_errors_on_nonzero_exit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.task_show = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = doctor_run_task_show(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            &logger,
        )
        .expect_err("expected task_show error");
        assert!(err.contains("commands.task_show failed"));
    }

    #[cfg(unix)]
    #[test]
    fn doctor_run_task_show_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.task_show = "task-show".to_string();
        let logger = Logger::new(None);

        env::set_var("PATH", temp.path());
        let err = doctor_run_task_show(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            &logger,
        )
        .expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn doctor_run_task_status_errors_on_nonzero_exit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.task_status = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = doctor_run_task_status(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            &logger,
        )
        .expect_err("expected task_status error");
        assert!(err.contains("commands.task_status failed"));
    }

    #[test]
    fn doctor_run_task_status_errors_on_empty_status() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.task_status = "true".to_string();
        let logger = Logger::new(None);
        let err = doctor_run_task_status(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            &logger,
        )
        .expect_err("expected empty status error");
        assert!(err.contains("returned an empty status"));
    }

    #[cfg(unix)]
    #[test]
    fn doctor_run_task_status_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.task_status = "task-status".to_string();
        let logger = Logger::new(None);

        env::set_var("PATH", temp.path());
        let err = doctor_run_task_status(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            &logger,
        )
        .expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn doctor_run_task_update_status_errors_on_nonzero_exit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.task_update_in_progress = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = doctor_run_task_update_status(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            "closed",
            &logger,
        )
        .expect_err("expected update error");
        assert!(err.contains("failed to set status"));
    }

    #[cfg(unix)]
    #[test]
    fn doctor_run_task_update_status_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.task_update_in_progress = "task-update".to_string();
        let logger = Logger::new(None);

        env::set_var("PATH", temp.path());
        let err = doctor_run_task_update_status(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            "closed",
            &logger,
        )
        .expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn doctor_run_reset_task_errors_on_nonzero_exit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.reset_task = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = doctor_run_reset_task(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            &logger,
        )
        .expect_err("expected reset error");
        assert!(err.contains("commands.reset_task failed"));
    }

    #[cfg(unix)]
    #[test]
    fn doctor_run_reset_task_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.reset_task = "reset-task".to_string();
        let logger = Logger::new(None);

        env::set_var("PATH", temp.path());
        let err = doctor_run_reset_task(
            &config,
            &temp.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            &logger,
        )
        .expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn doctor_run_hook_errors_on_nonzero_exit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = TempDir::new().expect("scratch");
        let logger = Logger::new(None);
        let err = doctor_run_hook(
            "exit 2",
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            "show",
            "open",
            "doctor-hook",
            &logger,
        )
        .expect_err("expected hook error");
        assert!(err.contains("hook doctor-hook failed"));
    }

    #[cfg(unix)]
    #[test]
    fn doctor_run_hook_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let logger = Logger::new(None);

        env::set_var("PATH", temp.path());
        let err = doctor_run_hook(
            "hook",
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            "tr-1",
            "show",
            "open",
            "doctor-hook",
            &logger,
        )
        .expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    fn status_queue_command(queue_path: &Path) -> String {
        let path = queue_path.display().to_string();
        format!(
            "queue=\"{path}\"; tmp=\"{path}.tmp\"; line=\"\"; \
             if [ -f \"$queue\" ]; then line=$(head -n 1 \"$queue\" || true); \
             tail -n +2 \"$queue\" > \"$tmp\" || true; mv \"$tmp\" \"$queue\"; \
             if [ -n \"$line\" ]; then printf '%s\\n' \"$line\"; fi; fi"
        )
    }

    fn status_queue_or_fail_command(queue_path: &Path) -> String {
        let path = queue_path.display().to_string();
        format!(
            "queue=\"{path}\"; tmp=\"{path}.tmp\"; line=\"\"; \
             if [ -f \"$queue\" ]; then line=$(head -n 1 \"$queue\" || true); \
             tail -n +2 \"$queue\" > \"$tmp\" || true; mv \"$tmp\" \"$queue\"; \
             if [ \"$line\" = \"FAIL\" ]; then exit 2; fi; \
             if [ -n \"$line\" ]; then printf '%s\\n' \"$line\"; fi; fi"
        )
    }

    #[test]
    fn run_doctor_checks_propagates_next_task_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.next_task = None;
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected next_task error");
        assert!(err.contains("commands.next_task must not be empty"));
    }

    #[test]
    fn run_doctor_checks_propagates_issue_parse_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{bad json}\n");
        let config = base_config();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected parse error");
        assert!(err.contains("doctor failed to parse issues"));
    }

    #[test]
    fn run_doctor_checks_errors_when_issue_db_is_empty() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("\n\n");
        let config = base_config();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected empty db error");
        assert!(err.contains("doctor scratch DB has no issues"));
    }

    #[test]
    fn run_doctor_checks_selects_any_task_id_when_no_ready_tasks() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"blocked\"}\n");
        let mut config = base_config();
        config.commands.reset_task = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected reset_task error");
        assert!(err.contains("commands.reset_task failed"));
    }

    #[test]
    fn run_doctor_checks_errors_when_status_after_reset_is_not_ready() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.task_status = "printf 'blocked\\n'".to_string();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected status mismatch error");
        assert!(err.contains("expected commands.task_status to return ready/open"));
    }

    #[test]
    fn run_doctor_checks_propagates_task_status_errors_after_reset_task() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.task_status = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected task_status error");
        assert!(err.contains("commands.task_status failed"));
    }

    #[test]
    fn run_doctor_checks_errors_when_status_after_update_is_not_in_progress() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nopen\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected in_progress mismatch error");
        assert!(err.contains("expected commands.task_status to return 'in_progress'"));
    }

    #[test]
    fn run_doctor_checks_propagates_task_status_errors_after_update() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nFAIL\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_or_fail_command(&queue_path);
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected task_status error");
        assert!(err.contains("commands.task_status failed"));
    }

    #[test]
    fn run_doctor_checks_propagates_reset_task_errors_after_update() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let reset_marker = scratch.path().join("reset-marker");
        let marker_path = reset_marker.display().to_string();

        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\n").expect("write queue");

        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        config.commands.reset_task =
            format!("if [ -f '{marker_path}' ]; then exit 2; fi; touch '{marker_path}'; exit 0");
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected reset_task error");
        assert!(err.contains("commands.reset_task failed"));
    }

    #[test]
    fn run_doctor_checks_propagates_task_status_errors_after_second_reset() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nFAIL\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_or_fail_command(&queue_path);
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected task_status error");
        assert!(err.contains("commands.task_status failed"));
    }

    #[test]
    fn run_doctor_checks_errors_when_status_after_second_reset_is_not_ready() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nblocked\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected status mismatch error");
        assert!(err.contains("expected commands.task_status to return ready/open"));
    }

    #[test]
    fn run_doctor_checks_errors_when_reset_task_command_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.reset_task = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected reset_task error");
        assert!(err.contains("commands.reset_task failed"));
    }

    #[test]
    fn run_doctor_checks_errors_when_task_show_command_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.task_status = "printf 'open\\n'".to_string();
        config.commands.task_show = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected task_show error");
        assert!(err.contains("commands.task_show failed"));
    }

    #[test]
    fn run_doctor_checks_errors_when_task_update_command_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.task_status = "printf 'open\\n'".to_string();
        config.commands.task_show = "printf 'SHOW'".to_string();
        config.commands.task_update_in_progress = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected update error");
        assert!(err.contains("task_update_in_progress failed"));
    }

    #[test]
    fn run_doctor_checks_errors_when_hook_on_completed_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nopen\nclosed\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        config.hooks.on_completed = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected hook error");
        assert!(err.contains("doctor-hook-on-completed"));
    }

    #[test]
    fn run_doctor_checks_errors_when_hook_on_requires_human_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nopen\nclosed\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        config.hooks.on_requires_human = "exit 2".to_string();
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected hook error");
        assert!(err.contains("doctor-hook-on-requires-human"));
    }

    #[test]
    fn run_doctor_checks_errors_when_closed_task_status_is_not_closed() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues(
            r#"{"id":"a-open","status":"open"}
{"id":"z-closed","status":"closed"}
"#,
        );
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nopen\nopen\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected closed status mismatch");
        assert!(err.contains("expected commands.task_status to return 'closed'"));
    }

    #[test]
    fn run_doctor_checks_errors_when_closed_status_after_setting_closed_is_wrong() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nopen\nopen\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected closed status mismatch");
        assert!(err.contains("doctor expected commands.task_status to return 'closed'"));
    }

    #[test]
    fn run_doctor_checks_propagates_task_status_errors_for_closed_tasks() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues(
            r#"{"id":"a-open","status":"open"}
{"id":"z-closed","status":"closed"}
"#,
        );
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nopen\nFAIL\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_or_fail_command(&queue_path);
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected task_status error");
        assert!(err.contains("commands.task_status failed"));
    }

    #[test]
    fn run_doctor_checks_propagates_task_update_errors_when_setting_closed() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let update_marker = scratch.path().join("update-marker");
        let marker_path = update_marker.display().to_string();

        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nopen\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        config.commands.task_update_in_progress =
            format!("if [ -f '{marker_path}' ]; then exit 2; fi; touch '{marker_path}'; exit 0");
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected task_update_in_progress error");
        assert!(err.contains("failed to set status closed"));
    }

    #[test]
    fn run_doctor_checks_propagates_task_status_errors_when_checking_closed_after_setting_closed() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nopen\nFAIL\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_or_fail_command(&queue_path);
        let logger = Logger::new(None);
        let err = run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect_err("expected task_status error");
        assert!(err.contains("commands.task_status failed"));
    }

    #[test]
    fn run_doctor_checks_succeeds_when_setting_closed_yields_closed_status() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let queue_path = scratch.path().join("status-queue.txt");
        fs::write(&queue_path, "open\nin_progress\nopen\nclosed\n").expect("write queue");
        let mut config = base_config();
        config.commands.task_status = status_queue_command(&queue_path);
        let logger = Logger::new(None);
        run_doctor_checks(
            &config,
            &scratch.path().join("trudger.yml"),
            scratch.path(),
            &scratch.path().display().to_string(),
            &logger,
        )
        .expect("expected doctor checks to pass");
    }

    #[test]
    fn run_doctor_mode_errors_when_validate_config_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut config = base_config();
        config.agent_command = "".to_string();
        let logger = Logger::new(None);
        let quit = run_doctor_mode(&config, temp.path(), &logger).expect_err("expected quit");
        assert_eq!(quit.code, 1);
    }

    #[test]
    fn run_doctor_mode_errors_when_setup_hook_is_empty() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut config = base_config();
        config.hooks.on_doctor_setup = None;
        let logger = Logger::new(None);
        let quit = run_doctor_mode(&config, temp.path(), &logger).expect_err("expected quit");
        assert_eq!(quit.code, 1);
    }

    #[cfg(unix)]
    #[test]
    fn run_doctor_mode_errors_when_invocation_cwd_is_missing() {
        use std::path::PathBuf;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let original_cwd = env::current_dir().expect("cwd");
        struct RestoreCwd(PathBuf);
        impl Drop for RestoreCwd {
            fn drop(&mut self) {
                let _ = env::set_current_dir(&self.0);
            }
        }
        let _restore = RestoreCwd(original_cwd);

        let missing_cwd = temp.path().join("missing-cwd");
        fs::create_dir_all(&missing_cwd).expect("create cwd");
        env::set_current_dir(&missing_cwd).expect("chdir");
        fs::remove_dir(&missing_cwd).expect("remove cwd");

        let config = base_config();
        let logger = Logger::new(None);
        let quit = run_doctor_mode(&config, temp.path(), &logger).expect_err("expected quit");
        assert_eq!(quit.code, 1);
        assert!(quit.reason.contains("doctor_invocation_cwd_failed"));
    }

    #[cfg(unix)]
    #[test]
    fn run_doctor_mode_errors_when_scratch_create_fails() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let tmpdir = temp.path().join("no-write");
        fs::create_dir_all(&tmpdir).expect("create tmpdir");
        fs::set_permissions(&tmpdir, fs::Permissions::from_mode(0o555)).expect("chmod tmpdir");

        env::set_var("TMPDIR", &tmpdir);
        let config = base_config();
        let logger = Logger::new(None);
        let quit = run_doctor_mode(&config, temp.path(), &logger).expect_err("expected quit");
        assert_eq!(quit.code, 1);

        env::remove_var("TMPDIR");
        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn run_doctor_mode_errors_when_setup_hook_exits_nonzero() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut config = base_config();
        config.hooks.on_doctor_setup = Some("exit 2".to_string());
        let logger = Logger::new(None);
        let quit = run_doctor_mode(&config, temp.path(), &logger).expect_err("expected quit");
        assert_eq!(quit.code, 1);
    }

    #[cfg(unix)]
    #[test]
    fn run_doctor_mode_propagates_setup_hook_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let config = base_config();
        let logger = Logger::new(None);

        env::set_var("PATH", temp.path());
        let quit = run_doctor_mode(&config, temp.path(), &logger).expect_err("expected quit");
        assert_eq!(quit.code, 1);

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn run_doctor_mode_errors_when_restore_invocation_cwd_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let original_cwd = env::current_dir().expect("cwd");
        let temp = TempDir::new().expect("temp dir");
        let invocation = temp.path().join("invocation");
        fs::create_dir_all(&invocation).expect("create invocation");
        env::set_current_dir(&invocation).expect("set invocation cwd");

        let mut config = base_config();
        let delete_target = invocation.display().to_string();
        config.hooks.on_doctor_setup = Some(format!(
            "mkdir -p \"$TRUDGER_DOCTOR_SCRATCH_DIR/.beads\"; printf '%s\\n' '{{\"id\":\"tr-1\",\"status\":\"open\"}}' > \"$TRUDGER_DOCTOR_SCRATCH_DIR/.beads/issues.jsonl\"; cd /; rm -rf \"{delete_target}\""
        ));
        config.commands.task_status = "printf 'open\\n'".to_string();
        let logger = Logger::new(None);

        let quit = run_doctor_mode(&config, temp.path(), &logger).expect_err("expected quit");
        assert_eq!(quit.code, 1);

        env::set_current_dir(&original_cwd).expect("restore cwd");
    }

    #[test]
    fn run_doctor_mode_errors_when_checks_fail() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let invocation = temp.path().join("invocation");
        fs::create_dir_all(&invocation).expect("create invocation");
        let original_cwd = env::current_dir().expect("cwd");
        env::set_current_dir(&invocation).expect("set invocation cwd");

        let mut config = base_config();
        config.hooks.on_doctor_setup = Some("exit 0".to_string());
        config.commands.next_task = None;
        let logger = Logger::new(None);
        let quit = run_doctor_mode(&config, temp.path(), &logger).expect_err("expected quit");
        assert_eq!(quit.code, 1);

        env::set_current_dir(&original_cwd).expect("restore cwd");
    }
}
