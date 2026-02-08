use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::config::Config;
use crate::logger::sanitize_log_value;
use crate::logger::Logger;
use crate::run_loop::{quit, validate_config, Quit};
use crate::shell::{
    run_shell_command_capture, run_shell_command_status, CommandEnv, CommandResult,
};
use crate::task_types::{TaskId, TaskStatus};

#[derive(Debug, serde::Deserialize)]
struct DoctorIssueSnapshot {
    id: TaskId,
    #[serde(default)]
    status: String,
}

fn load_doctor_issue_statuses(path: &Path) -> Result<BTreeMap<TaskId, TaskStatus>, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("doctor failed to read issues {}: {}", path.display(), err))?;
    let reader = BufReader::new(file);
    let mut latest: BTreeMap<TaskId, TaskStatus> = BTreeMap::new();

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
        let status = TaskStatus::parse(&snapshot.status)
            .unwrap_or_else(|| TaskStatus::Unknown(snapshot.status.clone()));
        latest.insert(snapshot.id, status);
    }

    Ok(latest)
}

#[derive(Clone, Copy, Debug)]
struct DoctorTaskEnv<'a> {
    task_id: Option<&'a TaskId>,
    task_show: Option<&'a str>,
    task_status: Option<&'a TaskStatus>,
}

impl<'a> DoctorTaskEnv<'a> {
    fn none() -> Self {
        Self {
            task_id: None,
            task_show: None,
            task_status: None,
        }
    }

    fn for_task(task_id: &'a TaskId) -> Self {
        Self {
            task_id: Some(task_id),
            task_show: None,
            task_status: None,
        }
    }
}

#[derive(Debug)]
struct DoctorHookTask<'a> {
    id: &'a TaskId,
    show: &'a str,
    status: &'a TaskStatus,
}

#[derive(Debug)]
struct DoctorCtx<'a> {
    config: &'a Config,
    config_path: &'a Path,
    scratch_dir: &'a Path,
    scratch_path: &'a str,
    logger: &'a Logger,
}

impl DoctorCtx<'_> {
    fn env(&self, task: DoctorTaskEnv<'_>) -> CommandEnv {
        CommandEnv {
            cwd: Some(self.scratch_dir.to_path_buf()),
            config_path: self.config_path.display().to_string(),
            scratch_dir: Some(self.scratch_path.to_string()),
            task_id: task.task_id.map(|value| value.to_string()),
            task_show: task.task_show.map(|value| value.to_string()),
            task_status: task.task_status.map(|value| value.as_str().to_string()),
            prompt: None,
            review_prompt: None,
            completed: None,
            needs_human: None,
        }
    }

    fn run_capture(
        &self,
        command: &str,
        log_label: &str,
        task_token: &str,
        args: &[String],
        task: DoctorTaskEnv<'_>,
    ) -> Result<CommandResult, String> {
        let env = self.env(task);
        run_shell_command_capture(command, log_label, task_token, args, &env, self.logger)
    }

    fn run_status(
        &self,
        command: &str,
        log_label: &str,
        task_token: &str,
        args: &[String],
        task: DoctorTaskEnv<'_>,
    ) -> Result<i32, String> {
        let env = self.env(task);
        run_shell_command_status(command, log_label, task_token, args, &env, self.logger)
    }
}

fn doctor_run_next_task(ctx: &DoctorCtx<'_>) -> Result<(), String> {
    let next_task = ctx
        .config
        .commands
        .next_task
        .as_deref()
        .unwrap_or("")
        .trim();
    if next_task.is_empty() {
        return Err("commands.next_task must not be empty.".to_string());
    }
    let output = ctx.run_capture(
        next_task,
        "doctor-next-task",
        "none",
        &[],
        DoctorTaskEnv::none(),
    )?;
    match output.exit_code {
        0 => {
            // Empty output is valid ("no tasks") in Trudger semantics.
            let token = output.stdout.split_whitespace().next().unwrap_or("");
            if !token.trim().is_empty() {
                // `TaskId` validation currently rejects only empty strings; the split token is
                // guaranteed to be non-empty after the trim check above.
                let _task_id = TaskId::try_from(token).unwrap();
            }
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

fn doctor_run_task_show(ctx: &DoctorCtx<'_>, task_id: &TaskId) -> Result<String, String> {
    let output = ctx.run_capture(
        &ctx.config.commands.task_show,
        "doctor-task-show",
        task_id.as_str(),
        &[],
        DoctorTaskEnv::for_task(task_id),
    )?;
    if output.exit_code != 0 {
        return Err(format!(
            "commands.task_show failed with exit code {}",
            output.exit_code
        ));
    }
    Ok(output.stdout)
}

fn doctor_run_task_status(ctx: &DoctorCtx<'_>, task_id: &TaskId) -> Result<TaskStatus, String> {
    let output = ctx.run_capture(
        &ctx.config.commands.task_status,
        "doctor-task-status",
        task_id.as_str(),
        &[],
        DoctorTaskEnv::for_task(task_id),
    )?;
    if output.exit_code != 0 {
        return Err(format!(
            "commands.task_status failed with exit code {}",
            output.exit_code
        ));
    }
    let token = output.stdout.split_whitespace().next().unwrap_or("");
    let Some(status) = TaskStatus::parse(token) else {
        return Err("commands.task_status returned an empty status.".to_string());
    };
    if status.is_unknown() {
        ctx.logger.log_transition(&format!(
            "unknown_task_status task={} status={}",
            task_id,
            sanitize_log_value(status.as_str())
        ));
    }
    Ok(status)
}

fn doctor_run_task_update_status(
    ctx: &DoctorCtx<'_>,
    task_id: &TaskId,
    status: TaskStatus,
) -> Result<(), String> {
    debug_assert!(
        !status.is_unknown(),
        "doctor_run_task_update_status must not be called with an unknown status"
    );
    let args = vec!["--status".to_string(), status.as_str().to_string()];
    let exit = ctx.run_status(
        &ctx.config.commands.task_update_in_progress,
        "doctor-task-update",
        task_id.as_str(),
        &args,
        DoctorTaskEnv::for_task(task_id),
    )?;
    if exit != 0 {
        return Err(format!(
            "commands.task_update_in_progress failed to set status {} (exit code {})",
            status.as_str(),
            exit
        ));
    }
    Ok(())
}

fn doctor_run_reset_task(ctx: &DoctorCtx<'_>, task_id: &TaskId) -> Result<(), String> {
    let exit = ctx.run_status(
        &ctx.config.commands.reset_task,
        "doctor-reset-task",
        task_id.as_str(),
        &[],
        DoctorTaskEnv::for_task(task_id),
    )?;
    if exit != 0 {
        return Err(format!(
            "commands.reset_task failed with exit code {}",
            exit
        ));
    }
    Ok(())
}

fn doctor_run_hook(
    hook_command: &str,
    ctx: &DoctorCtx<'_>,
    hook_name: &str,
    task: DoctorHookTask<'_>,
) -> Result<(), String> {
    let env = DoctorTaskEnv {
        task_id: Some(task.id),
        task_show: Some(task.show),
        task_status: Some(task.status),
    };
    let exit = ctx.run_status(hook_command, hook_name, task.id.as_str(), &[], env)?;
    if exit != 0 {
        return Err(format!("hook {} failed with exit code {}", hook_name, exit));
    }
    Ok(())
}

fn run_doctor_checks(ctx: &DoctorCtx<'_>) -> Result<(), String> {
    let beads_dir = ctx.scratch_dir.join(".beads");
    let issues_path = beads_dir.join("issues.jsonl");
    if !issues_path.is_file() {
        return Err(format!(
            "doctor scratch DB is missing {}.\nExpected hooks.on_doctor_setup to create $TRUDGER_DOCTOR_SCRATCH_DIR/.beads with issues.jsonl.",
            issues_path.display()
        ));
    }

    doctor_run_next_task(ctx)?;

    let statuses = load_doctor_issue_statuses(&issues_path)?;
    let any_task_id = statuses
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| "doctor scratch DB has no issues in issues.jsonl.".to_string())?;
    let task_id = statuses
        .iter()
        .find_map(|(id, status)| {
            if status.is_ready() {
                Some(id.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| any_task_id.clone());

    // If the scratch DB only contains closed tasks, we may end up mutating a closed task
    // (reset -> open, update -> in_progress, etc.). Don't reuse that same task for the "closed
    // parsing" verification.
    let closed_task_id = statuses.iter().find_map(|(id, status)| {
        if status == &TaskStatus::Closed && id != &task_id {
            Some(id.clone())
        } else {
            None
        }
    });

    // Verify reset -> ready/open parsing.
    doctor_run_reset_task(ctx, &task_id)?;
    let status = doctor_run_task_status(ctx, &task_id)?;
    if !status.is_ready() {
        return Err(format!(
            "doctor expected commands.task_status to return ready/open after reset_task, got '{}'.",
            status
        ));
    }

    // Verify show runs successfully (content is prompt-only in run mode).
    let show = doctor_run_task_show(ctx, &task_id)?;

    // Verify update -> in_progress parsing.
    doctor_run_task_update_status(ctx, &task_id, TaskStatus::InProgress)?;
    let status = doctor_run_task_status(ctx, &task_id)?;
    if status != TaskStatus::InProgress {
        return Err(format!(
            "doctor expected commands.task_status to return 'in_progress' after task_update_in_progress, got '{}'.",
            status
        ));
    }

    // Verify reset works again and yields ready/open.
    doctor_run_reset_task(ctx, &task_id)?;
    let status = doctor_run_task_status(ctx, &task_id)?;
    if !status.is_ready() {
        return Err(format!(
            "doctor expected commands.task_status to return ready/open after reset_task, got '{}'.",
            status
        ));
    }

    // Verify completion/escalation hooks are runnable in the scratch DB environment.
    doctor_run_hook(
        &ctx.config.hooks.on_completed,
        ctx,
        "doctor-hook-on-completed",
        DoctorHookTask {
            id: &task_id,
            show: &show,
            status: &status,
        },
    )?;
    doctor_run_hook(
        &ctx.config.hooks.on_requires_human,
        ctx,
        "doctor-hook-on-requires-human",
        DoctorHookTask {
            id: &task_id,
            show: &show,
            status: &status,
        },
    )?;

    // Verify closed parsing.
    match closed_task_id {
        Some(closed_task_id) => {
            let closed_status = doctor_run_task_status(ctx, &closed_task_id)?;
            if closed_status != TaskStatus::Closed {
                return Err(format!(
                    "doctor expected commands.task_status to return 'closed' for task {}, got '{}'.",
                    closed_task_id, closed_status
                ));
            }
        }
        None => {
            doctor_run_task_update_status(ctx, &task_id, TaskStatus::Closed)?;
            let closed_status = doctor_run_task_status(ctx, &task_id)?;
            if closed_status != TaskStatus::Closed {
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
        Ok(()) => {
            let ctx = DoctorCtx {
                config,
                config_path,
                scratch_dir: &scratch_dir,
                scratch_path: &scratch_path,
                logger,
            };
            run_doctor_checks(&ctx)
        }
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

    fn task(id: &str) -> TaskId {
        TaskId::try_from(id).expect("task id")
    }

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
            review_loop_limit: crate::task_types::ReviewLoopLimit::new(1)
                .expect("review_loop_limit"),
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

    fn doctor_ctx<'a>(
        config: &'a Config,
        config_path: &'a Path,
        scratch: &'a TempDir,
        scratch_path: &'a str,
        logger: &'a Logger,
    ) -> DoctorCtx<'a> {
        DoctorCtx {
            config,
            config_path,
            scratch_dir: scratch.path(),
            scratch_path,
            logger,
        }
    }

    #[test]
    fn run_doctor_checks_succeeds_when_only_closed_issue_exists() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"closed\"}\n");
        let status_path = scratch.path().join("status.txt");
        fs::write(&status_path, "closed\n").expect("write status");

        let status_path_str = status_path.display().to_string();
        let mut config = base_config();
        config.commands.next_task = Some("exit 1".to_string());
        config.commands.task_show = "printf 'SHOW'".to_string();
        config.commands.task_status = format!("cat \"{}\"", status_path_str);
        config.commands.reset_task = format!("printf 'open\\n' > \"{}\"", status_path_str);
        config.commands.task_update_in_progress = format!(
            "if [[ \"$1\" != \"--status\" || -z \"$2\" ]]; then exit 2; fi; printf '%s\\n' \"$2\" > \"{}\"",
            status_path_str
        );

        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);

        run_doctor_checks(&ctx).expect("expected doctor checks to succeed");
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
        assert!(statuses.contains_key(&task("tr-1")));
        assert!(statuses.contains_key(&task("tr-2")));
    }

    #[test]
    fn load_doctor_issue_statuses_treats_missing_status_as_unknown() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"tr-1\"}\n");
        let statuses =
            load_doctor_issue_statuses(&scratch.path().join(".beads").join("issues.jsonl"))
                .expect("load");
        let status = statuses.get(&task("tr-1")).expect("status");
        assert!(status.is_unknown());
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_next_task(&ctx).expect_err("expected empty next_task error");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        doctor_run_next_task(&ctx).expect("exit 1 should be ok");
    }

    #[test]
    fn doctor_run_next_task_accepts_nonempty_output() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.next_task = Some("printf 'tr-1\\n'".to_string());
        let logger = Logger::new(None);
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        doctor_run_next_task(&ctx).expect("non-empty output should be ok");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_next_task(&ctx).expect_err("expected exit code error");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_next_task(&ctx).expect_err("expected spawn error");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_task_show(&ctx, &task("tr-1")).expect_err("expected task_show error");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_task_show(&ctx, &task("tr-1")).expect_err("expected spawn error");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err =
            doctor_run_task_status(&ctx, &task("tr-1")).expect_err("expected task_status error");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err =
            doctor_run_task_status(&ctx, &task("tr-1")).expect_err("expected empty status error");
        assert!(err.contains("returned an empty status"));
    }

    #[test]
    fn doctor_run_task_status_logs_unknown_status() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let mut config = base_config();
        config.commands.task_status = "printf 'mystery\\n'".to_string();

        let log_path = temp.path().join("transitions.log");
        let logger = Logger::new(Some(log_path.clone()));
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };

        let status = doctor_run_task_status(&ctx, &task("tr-1")).expect("task_status");
        assert!(status.is_unknown());

        let contents = fs::read_to_string(&log_path).expect("read transitions");
        assert!(contents.contains("unknown_task_status task=tr-1 status=mystery"));
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_task_status(&ctx, &task("tr-1")).expect_err("expected spawn error");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_task_update_status(&ctx, &task("tr-1"), TaskStatus::Closed)
            .expect_err("expected update error");
        assert!(err.contains("failed to set status"));
    }

    #[test]
    fn doctor_run_task_update_status_debug_assert_panics_on_unknown_status() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let scratch = TempDir::new().expect("scratch");
        let config = base_config();
        let logger = Logger::new(None);
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = doctor_run_task_update_status(
                &ctx,
                &task("tr-1"),
                TaskStatus::Unknown("mystery".to_string()),
            );
        }));
        assert!(result.is_err());
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_task_update_status(&ctx, &task("tr-1"), TaskStatus::Closed)
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_reset_task(&ctx, &task("tr-1")).expect_err("expected reset error");
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
        let config_path = temp.path().join("trudger.yml");
        let scratch_path = scratch.path().display().to_string();
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let err = doctor_run_reset_task(&ctx, &task("tr-1")).expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn doctor_run_hook_errors_on_nonzero_exit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = TempDir::new().expect("scratch");
        let scratch_path = scratch.path().display().to_string();
        let config = base_config();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };
        let id = task("tr-1");
        let status = TaskStatus::Open;
        let err = doctor_run_hook(
            "exit 2",
            &ctx,
            "doctor-hook",
            DoctorHookTask {
                id: &id,
                show: "show",
                status: &status,
            },
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
        let scratch_path = scratch.path().display().to_string();
        let config = base_config();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = DoctorCtx {
            config: &config,
            config_path: &config_path,
            scratch_dir: scratch.path(),
            scratch_path: &scratch_path,
            logger: &logger,
        };

        env::set_var("PATH", temp.path());
        let id = task("tr-1");
        let status = TaskStatus::Open;
        let err = doctor_run_hook(
            "hook",
            &ctx,
            "doctor-hook",
            DoctorHookTask {
                id: &id,
                show: "show",
                status: &status,
            },
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected next_task error");
        assert!(err.contains("commands.next_task must not be empty"));
    }

    #[test]
    fn run_doctor_checks_propagates_issue_parse_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{bad json}\n");
        let config = base_config();
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected parse error");
        assert!(err.contains("doctor failed to parse issues"));
    }

    #[test]
    fn run_doctor_checks_errors_when_issue_db_is_empty() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("\n\n");
        let config = base_config();
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected empty db error");
        assert!(err.contains("doctor scratch DB has no issues"));
    }

    #[test]
    fn run_doctor_checks_selects_any_task_id_when_no_ready_tasks() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"blocked\"}\n");
        let mut config = base_config();
        config.commands.reset_task = "exit 2".to_string();
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected reset_task error");
        assert!(err.contains("commands.reset_task failed"));
    }

    #[test]
    fn run_doctor_checks_errors_when_status_after_reset_is_not_ready() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.task_status = "printf 'blocked\\n'".to_string();
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected status mismatch error");
        assert!(err.contains("expected commands.task_status to return ready/open"));
    }

    #[test]
    fn run_doctor_checks_propagates_task_status_errors_after_reset_task() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.task_status = "exit 2".to_string();
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected task_status error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected in_progress mismatch error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected task_status error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected reset_task error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected task_status error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected status mismatch error");
        assert!(err.contains("expected commands.task_status to return ready/open"));
    }

    #[test]
    fn run_doctor_checks_errors_when_reset_task_command_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let scratch = scratch_with_issues("{\"id\":\"a-task\",\"status\":\"open\"}\n");
        let mut config = base_config();
        config.commands.reset_task = "exit 2".to_string();
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected reset_task error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected task_show error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected update error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected hook error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected hook error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected closed status mismatch");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected closed status mismatch");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected task_status error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected task_update_in_progress error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        let err = run_doctor_checks(&ctx).expect_err("expected task_status error");
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
        let scratch_path = scratch.path().display().to_string();
        let config_path = scratch.path().join("trudger.yml");
        let logger = Logger::new(None);
        let ctx = doctor_ctx(&config, &config_path, &scratch, &scratch_path, &logger);
        run_doctor_checks(&ctx).expect("expected doctor checks to pass");
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
