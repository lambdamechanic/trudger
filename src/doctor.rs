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
