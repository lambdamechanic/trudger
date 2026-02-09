use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::config::Config;
use crate::logger::{sanitize_log_value, Logger};
use crate::shell::{
    run_shell_command_capture, run_shell_command_status, CommandEnv, CommandResult,
};
use crate::task_types::{Phase, TaskId, TaskStatus};
use crate::tmux::TmuxState;

#[derive(Debug)]
pub(crate) struct RuntimeState {
    pub(crate) config: Config,
    pub(crate) config_path: PathBuf,
    pub(crate) prompt_trudge: String,
    pub(crate) prompt_review: String,
    pub(crate) logger: Logger,
    pub(crate) tmux: TmuxState,
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    pub(crate) manual_tasks: Vec<TaskId>,
    pub(crate) completed_tasks: Vec<TaskId>,
    pub(crate) needs_human_tasks: Vec<TaskId>,
    pub(crate) current_task_id: Option<TaskId>,
    pub(crate) current_task_show: Option<String>,
    pub(crate) current_task_status: Option<TaskStatus>,
}

#[derive(Debug)]
pub(crate) struct Quit {
    pub(crate) code: i32,
    #[allow(dead_code)]
    pub(crate) reason: String,
}

impl Quit {
    pub(crate) fn exit_code(&self) -> ExitCode {
        // Process exit statuses are the low 8 bits on Unix (same behavior as `std::process::exit`).
        // This is intentionally not clamped: values outside 0..=255 wrap.
        ExitCode::from((self.code & 0xFF) as u8)
    }
}

pub(crate) fn quit(logger: &Logger, reason: &str, code: i32) -> Quit {
    let sanitized = if reason.trim().is_empty() {
        "unknown".to_string()
    } else {
        sanitize_log_value(reason)
    };
    logger.log_transition(&format!("quit reason={}", sanitized));
    Quit {
        code,
        reason: reason.to_string(),
    }
}

pub(crate) fn validate_config(config: &Config, manual_tasks: &[TaskId]) -> Result<(), String> {
    if config.agent_command.trim().is_empty() {
        return Err("agent_command must not be empty.".to_string());
    }
    if config.agent_review_command.trim().is_empty() {
        return Err("agent_review_command must not be empty.".to_string());
    }

    let next_task = config.commands.next_task.as_deref().unwrap_or("").trim();
    if next_task.is_empty() {
        if manual_tasks.is_empty() {
            return Err(
                "commands.next_task must not be empty.\nMigration: add commands.next_task to your config (required when no manual task IDs). See README.md or sample_configuration/*.yml.".to_string(),
            );
        }
        eprintln!(
            "Warning: commands.next_task is empty; manual task IDs provided, continuing without next_task."
        );
    }

    if config.commands.task_show.trim().is_empty() {
        return Err("commands.task_show must not be empty.".to_string());
    }
    if config.commands.task_status.trim().is_empty() {
        return Err("commands.task_status must not be empty.".to_string());
    }
    if config.commands.task_update_in_progress.trim().is_empty() {
        return Err("commands.task_update_in_progress must not be empty.".to_string());
    }
    if config.commands.reset_task.trim().is_empty() {
        return Err("commands.reset_task must not be empty.".to_string());
    }
    if config.hooks.on_completed.trim().is_empty() {
        return Err("hooks.on_completed must not be empty.".to_string());
    }
    if config.hooks.on_requires_human.trim().is_empty() {
        return Err("hooks.on_requires_human must not be empty.".to_string());
    }

    Ok(())
}

fn build_command_env(
    state: &RuntimeState,
    task_id: Option<&TaskId>,
    prompt: Option<String>,
    review_prompt: Option<String>,
) -> CommandEnv {
    fn join_task_ids(tasks: &[TaskId]) -> String {
        let mut out = String::new();
        for (index, task) in tasks.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(task.as_str());
        }
        out
    }

    let completed = if state.completed_tasks.is_empty() {
        None
    } else {
        Some(join_task_ids(&state.completed_tasks))
    };
    let needs_human = if state.needs_human_tasks.is_empty() {
        None
    } else {
        Some(join_task_ids(&state.needs_human_tasks))
    };

    CommandEnv {
        cwd: None,
        config_path: state.config_path.display().to_string(),
        scratch_dir: None,
        task_id: task_id.map(|value| value.to_string()).or_else(|| {
            state
                .current_task_id
                .as_ref()
                .map(|value| value.to_string())
        }),
        task_show: state.current_task_show.clone(),
        task_status: state
            .current_task_status
            .as_ref()
            .map(|value| value.as_str().to_string()),
        prompt,
        review_prompt,
        completed,
        needs_human,
    }
}

fn run_config_command(
    state: &RuntimeState,
    command: &str,
    task_id: Option<&TaskId>,
    log_label: &str,
    args: &[String],
) -> Result<CommandResult, String> {
    let env = build_command_env(state, task_id, None, None);
    run_shell_command_capture(
        command,
        log_label,
        task_id.map(|value| value.as_str()).unwrap_or("none"),
        args,
        &env,
        &state.logger,
    )
}

fn run_config_command_status(
    state: &RuntimeState,
    command: &str,
    task_id: Option<&TaskId>,
    log_label: &str,
    args: &[String],
) -> Result<i32, String> {
    let env = build_command_env(state, task_id, None, None);
    run_shell_command_status(
        command,
        log_label,
        task_id.map(|value| value.as_str()).unwrap_or("none"),
        args,
        &env,
        &state.logger,
    )
}

fn run_agent_command(
    state: &RuntimeState,
    command: &str,
    log_label: &str,
    prompt: Option<String>,
    review_prompt: Option<String>,
    args: &[String],
) -> Result<i32, String> {
    let env = build_command_env(state, None, prompt, review_prompt);
    run_shell_command_status(command, log_label, "none", args, &env, &state.logger)
}

fn run_task_show(
    state: &mut RuntimeState,
    task_id: &TaskId,
    args: &[String],
) -> Result<(), String> {
    state.current_task_show = None;
    let output = run_config_command(
        state,
        &state.config.commands.task_show,
        Some(task_id),
        "task",
        args,
    )?;
    if output.exit_code != 0 {
        return Err(format!(
            "task_show failed with exit code {}",
            output.exit_code
        ));
    }
    state.current_task_show = Some(output.stdout);
    Ok(())
}

fn run_task_status(state: &mut RuntimeState, task_id: &TaskId) -> Result<(), String> {
    state.current_task_status = None;
    let output = run_config_command(
        state,
        &state.config.commands.task_status,
        Some(task_id),
        "task",
        &[],
    )?;
    if output.exit_code != 0 {
        return Err(format!(
            "task_status failed with exit code {}",
            output.exit_code
        ));
    }
    let token = output.stdout.split_whitespace().next().unwrap_or("");
    let parsed = TaskStatus::parse(token);
    if let Some(status) = parsed.as_ref() {
        if status.is_unknown() {
            state.logger.log_transition(&format!(
                "unknown_task_status task={} status={}",
                task_id,
                sanitize_log_value(status.as_str())
            ));
            return Err(format!(
                "unknown_task_status:{}:{}",
                task_id,
                sanitize_log_value(status.as_str())
            ));
        }
    }
    state.current_task_status = parsed;
    Ok(())
}

fn get_next_task_id(state: &RuntimeState) -> Result<Option<TaskId>, Quit> {
    let output = run_config_command(
        state,
        state.config.commands.next_task.as_deref().unwrap_or(""),
        None,
        "next-task",
        &[],
    )
    .map_err(|err| quit(&state.logger, &format!("next_task_failed:{err}"), 1))?;

    if output.exit_code == 1 {
        state.logger.log_transition("idle next_task_exit=1");
        return Err(quit(&state.logger, "no_next_task", 0));
    }
    if output.exit_code != 0 {
        eprintln!(
            "next_task command failed with exit code {}.",
            output.exit_code
        );
        return Err(quit(
            &state.logger,
            &format!("next_task_failed:{}", output.exit_code),
            output.exit_code,
        ));
    }

    let token = output.stdout.split_whitespace().next().unwrap_or("");
    if token.trim().is_empty() {
        return Ok(None);
    }
    let task_id = TaskId::try_from(token).map_err(|err| {
        eprintln!("next_task returned an invalid task id: {} ({})", token, err);
        quit(
            &state.logger,
            &format!("next_task_invalid_task_id:{err}"),
            1,
        )
    })?;
    Ok(Some(task_id))
}

fn ensure_task_ready(state: &mut RuntimeState, task_id: &TaskId) -> Result<(), Quit> {
    run_task_status(state, task_id)
        .map_err(|err| quit(&state.logger, &format!("task_status_failed:{err}"), 1))?;
    let status = state
        .current_task_status
        .clone()
        .unwrap_or(TaskStatus::Unknown(String::new()));
    if status.is_ready() {
        return Ok(());
    }
    eprintln!("Task {} is not ready (status: {}).", task_id, status);
    Err(quit(
        &state.logger,
        &format!("task_not_ready:{}", task_id),
        1,
    ))
}

fn update_task_status(
    state: &RuntimeState,
    task_id: &TaskId,
    status: TaskStatus,
) -> Result<(), String> {
    let args = vec!["--status".to_string(), status.as_str().to_string()];
    let exit = run_config_command_status(
        state,
        &state.config.commands.task_update_in_progress,
        Some(task_id),
        "task",
        &args,
    )?;
    if exit != 0 {
        return Err(format!(
            "task_update_in_progress failed to set status {} (exit code {})",
            status.as_str(),
            exit
        ));
    }
    Ok(())
}

fn update_in_progress(state: &RuntimeState, task_id: &TaskId) -> Result<(), String> {
    update_task_status(state, task_id, TaskStatus::InProgress)
}

fn reset_task(state: &RuntimeState, task_id: &TaskId) -> Result<(), String> {
    let exit = run_config_command_status(
        state,
        &state.config.commands.reset_task,
        Some(task_id),
        "reset_task",
        &[],
    )?;
    if exit != 0 {
        return Err(format!("reset_task failed with exit code {}", exit));
    }
    Ok(())
}

fn task_status_token(state: &RuntimeState, task_id: &TaskId) -> Result<Option<TaskStatus>, String> {
    let output = run_config_command(
        state,
        &state.config.commands.task_status,
        Some(task_id),
        "task",
        &[],
    )?;
    if output.exit_code != 0 {
        return Err(format!(
            "task_status failed with exit code {}",
            output.exit_code
        ));
    }
    let token = output.stdout.split_whitespace().next().unwrap_or("");
    Ok(TaskStatus::parse(token))
}

pub(crate) fn reset_task_on_exit(state: &RuntimeState, result: &Result<(), Quit>) {
    if result.is_ok() {
        return;
    }
    let Some(task_id) = state.current_task_id.as_ref() else {
        return;
    };

    let status = match task_status_token(state, task_id) {
        Ok(status) => status,
        Err(err) => {
            eprintln!(
                "Warning: failed to check task status for task {}, skipping reset: {}",
                task_id, err
            );
            state.logger.log_transition(&format!(
                "reset_task_skip task={} reason=task_status_failed err={}",
                task_id,
                sanitize_log_value(&err)
            ));
            return;
        }
    };

    let Some(status) = status else {
        eprintln!(
            "Warning: commands.task_status returned an empty status for task {}, skipping reset.",
            task_id
        );
        state.logger.log_transition(&format!(
            "reset_task_skip task={} reason=task_status_empty",
            task_id
        ));
        return;
    };

    if status != TaskStatus::InProgress {
        state.logger.log_transition(&format!(
            "reset_task_skip task={} status={}",
            task_id, status
        ));
        return;
    }

    match reset_task(state, task_id) {
        Ok(()) => state
            .logger
            .log_transition(&format!("reset_task task={}", task_id)),
        Err(err) => {
            eprintln!("Failed to reset task {}: {}", task_id, err);
            state.logger.log_transition(&format!(
                "reset_task_failed task={} err={}",
                task_id,
                sanitize_log_value(&err)
            ));
        }
    }
}

fn check_interrupted(state: &RuntimeState) -> Result<(), Quit> {
    if state.interrupt_flag.load(Ordering::SeqCst) {
        return Err(quit(&state.logger, "interrupted", 130));
    }
    Ok(())
}

fn run_hook(
    state: &RuntimeState,
    hook_command: &str,
    task_id: &TaskId,
    hook_name: &str,
) -> Result<(), String> {
    if hook_command.trim().is_empty() {
        return Ok(());
    }

    let exit = run_config_command_status(state, hook_command, Some(task_id), hook_name, &[])?;
    if exit != 0 {
        return Err(format!("hook {} failed with exit code {}", hook_name, exit));
    }
    Ok(())
}

fn run_agent_solve(state: &RuntimeState, args: &[String]) -> Result<(), String> {
    let exit = run_agent_command(
        state,
        &state.config.agent_command,
        "agent_solve",
        Some(state.prompt_trudge.clone()),
        None,
        args,
    )?;
    if exit != 0 {
        return Err(format!("agent_solve failed with exit code {}", exit));
    }
    Ok(())
}

fn run_agent_review(state: &RuntimeState) -> Result<(), String> {
    let args = vec!["resume".to_string(), "--last".to_string()];
    let exit = run_agent_command(
        state,
        &state.config.agent_review_command,
        "agent_review",
        None,
        Some(state.prompt_review.clone()),
        &args,
    )?;
    if exit != 0 {
        return Err(format!("agent_review failed with exit code {}", exit));
    }
    Ok(())
}

pub(crate) fn run_loop(state: &mut RuntimeState) -> Result<(), Quit> {
    check_interrupted(state)?;
    if !state.manual_tasks.is_empty() {
        for task_id in &state.manual_tasks.clone() {
            check_interrupted(state)?;
            ensure_task_ready(state, task_id)?;
        }
    }

    loop {
        check_interrupted(state)?;
        let task_id = if !state.manual_tasks.is_empty() {
            state.manual_tasks.remove(0)
        } else {
            let next_task_cmd = state.config.commands.next_task.as_deref().unwrap_or("");
            if next_task_cmd.trim().is_empty() {
                state
                    .logger
                    .log_transition("idle missing_next_task_command");
                return Err(quit(&state.logger, "missing_next_task_command", 0));
            }

            let skip_limit = env::var("TRUDGER_SKIP_NOT_READY_LIMIT")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value >= 1)
                .unwrap_or(5);

            let mut skip_count = 0usize;
            let selected = loop {
                check_interrupted(state)?;
                let task_id = match get_next_task_id(state)? {
                    Some(task_id) => task_id,
                    None => {
                        state.logger.log_transition("idle no_task");
                        return Err(quit(&state.logger, "no_task", 0));
                    }
                };
                run_task_status(state, &task_id)
                    .map_err(|err| quit(&state.logger, &format!("task_status_failed:{err}"), 1))?;
                let Some(status) = state.current_task_status.clone() else {
                    eprintln!("Task {} missing status.", task_id);
                    return Err(quit(
                        &state.logger,
                        &format!("task_missing_status:{}", task_id),
                        1,
                    ));
                };
                if status.is_ready() {
                    break task_id;
                }
                state.logger.log_transition(&format!(
                    "skip_not_ready task={} status={}",
                    task_id, status
                ));
                skip_count += 1;
                if skip_count >= skip_limit {
                    state
                        .logger
                        .log_transition(&format!("idle no_ready_task attempts={}", skip_count));
                    eprintln!("Task {} is not ready (status: {}).", task_id, status);
                    return Err(quit(&state.logger, "no_ready_task", 0));
                }
            };
            selected
        };

        state.current_task_id = Some(task_id.clone());
        state.current_task_show = None;
        state.current_task_status = None;
        let resume_args = vec!["resume".to_string(), "--last".to_string()];
        let mut review_loops: u64 = 0;

        loop {
            check_interrupted(state)?;
            state.tmux.update_name(
                Phase::Solving,
                &task_id,
                &state.completed_tasks,
                &state.needs_human_tasks,
            );
            state.logger.log_transition(&format!(
                "state=SOLVING task={} loop={}",
                task_id, review_loops
            ));

            if let Err(err) = update_in_progress(state, &task_id) {
                state.tmux.update_name(
                    Phase::Error,
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }

            check_interrupted(state)?;
            if let Err(err) = run_task_show(state, &task_id, &[]) {
                state.tmux.update_name(
                    Phase::Error,
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                state
                    .logger
                    .log_transition(&format!("error task={}", task_id));
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }

            check_interrupted(state)?;
            let solve_args: &[String] = if review_loops == 0 { &[] } else { &resume_args };
            if let Err(_err) = run_agent_solve(state, solve_args) {
                state.tmux.update_name(
                    Phase::Error,
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                state
                    .logger
                    .log_transition(&format!("solve_failed task={}", task_id));
                eprintln!("Agent solve failed for task {}.", task_id);
                return Err(quit(&state.logger, &format!("solve_failed:{}", task_id), 1));
            }

            state.tmux.update_name(
                Phase::Reviewing,
                &task_id,
                &state.completed_tasks,
                &state.needs_human_tasks,
            );
            state.logger.log_transition(&format!(
                "state=REVIEWING task={} loop={}",
                task_id, review_loops
            ));

            check_interrupted(state)?;
            if let Err(err) = run_task_show(state, &task_id, &[]) {
                state.tmux.update_name(
                    Phase::Error,
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }

            check_interrupted(state)?;
            if let Err(_err) = run_agent_review(state) {
                state.tmux.update_name(
                    Phase::Error,
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                state
                    .logger
                    .log_transition(&format!("review_failed task={}", task_id));
                eprintln!("Agent review failed for task {}.", task_id);
                return Err(quit(
                    &state.logger,
                    &format!("review_failed:{}", task_id),
                    1,
                ));
            }

            check_interrupted(state)?;
            run_task_status(state, &task_id)
                .map_err(|err| quit(&state.logger, &format!("task_status_failed:{err}"), 1))?;
            let Some(status) = state.current_task_status.clone() else {
                state.tmux.update_name(
                    Phase::Error,
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                state
                    .logger
                    .log_transition(&format!("review_state_missing task={}", task_id));
                eprintln!("Task {} missing status after review.", task_id);
                return Err(quit(
                    &state.logger,
                    &format!("task_missing_status_after_review:{}", task_id),
                    1,
                ));
            };
            state
                .logger
                .log_transition(&format!("review_state task={} status={}", task_id, status));

            if status == TaskStatus::Closed {
                state.completed_tasks.push(task_id.clone());
                state
                    .logger
                    .log_transition(&format!("completed task={}", task_id));
                if let Err(err) = run_hook(
                    state,
                    &state.config.hooks.on_completed,
                    &task_id,
                    "on_completed",
                ) {
                    return Err(quit(&state.logger, &format!("error:{err}"), 1));
                }
                break;
            }

            if status == TaskStatus::Blocked {
                state.needs_human_tasks.push(task_id.clone());
                state
                    .logger
                    .log_transition(&format!("needs_human task={}", task_id));
                if let Err(err) = run_hook(
                    state,
                    &state.config.hooks.on_requires_human,
                    &task_id,
                    "on_requires_human",
                ) {
                    return Err(quit(&state.logger, &format!("error:{err}"), 1));
                }
                break;
            }

            review_loops += 1;
            if review_loops < state.config.review_loop_limit.get() {
                state.logger.log_transition(&format!(
                    "review_loop_retry task={} loop={} limit={}",
                    task_id, review_loops, state.config.review_loop_limit
                ));
                continue;
            }

            state.logger.log_transition(&format!(
                "review_loop_exhausted task={} loops={} limit={}",
                task_id, review_loops, state.config.review_loop_limit
            ));
            if let Err(err) = update_task_status(state, &task_id, TaskStatus::Blocked) {
                state.tmux.update_name(
                    Phase::Error,
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }
            state.current_task_status = Some(TaskStatus::Blocked);

            state.needs_human_tasks.push(task_id.clone());
            state
                .logger
                .log_transition(&format!("needs_human task={}", task_id));
            if let Err(err) = run_hook(
                state,
                &state.config.hooks.on_requires_human,
                &task_id,
                "on_requires_human",
            ) {
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }
            break;
        }

        let completed_env = state
            .completed_tasks
            .iter()
            .map(|task| task.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let needs_human_env = state
            .needs_human_tasks
            .iter()
            .map(|task| task.as_str())
            .collect::<Vec<_>>()
            .join(",");
        state.logger.log_transition(&format!(
            "task_lists completed={} needs_human={}",
            completed_env, needs_human_env
        ));

        state.current_task_id = None;
        state.current_task_show = None;
        state.current_task_status = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn task(id: &str) -> TaskId {
        TaskId::try_from(id).expect("task id")
    }

    fn base_state(temp: &TempDir) -> RuntimeState {
        RuntimeState {
            config: Config {
                agent_command: "true".to_string(),
                agent_review_command: "true".to_string(),
                commands: crate::config::Commands {
                    next_task: None,
                    task_show: "true".to_string(),
                    task_status: "true".to_string(),
                    task_update_in_progress: "true".to_string(),
                    reset_task: "true".to_string(),
                },
                hooks: crate::config::Hooks {
                    on_completed: "true".to_string(),
                    on_requires_human: "true".to_string(),
                    on_doctor_setup: None,
                },
                review_loop_limit: crate::task_types::ReviewLoopLimit::new(1)
                    .expect("review_loop_limit"),
                log_path: None,
            },
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "prompt".to_string(),
            prompt_review: "review".to_string(),
            logger: Logger::new(None),
            tmux: TmuxState::disabled(),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            manual_tasks: Vec::new(),
            completed_tasks: Vec::new(),
            needs_human_tasks: Vec::new(),
            current_task_id: None,
            current_task_show: None,
            current_task_status: None,
        }
    }

    #[test]
    fn build_command_env_joins_completed_tasks_with_commas() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.completed_tasks = vec![task("tr-1"), task("tr-2")];

        let env = build_command_env(&state, None, None, None);
        assert_eq!(env.completed.as_deref(), Some("tr-1,tr-2"));
    }

    #[test]
    fn run_hook_is_noop_for_empty_command() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let state = base_state(&temp);

        run_hook(&state, "", &task("tr-1"), "hook").expect("hook should succeed");
    }

    #[test]
    fn task_status_errors_on_nonzero_exit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.task_status = "exit 2".to_string();

        let err =
            run_task_status(&mut state, &task("tr-1")).expect_err("expected task_status failure");
        assert!(err.contains("task_status failed"));
    }

    #[cfg(unix)]
    #[test]
    fn task_show_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);

        std::env::set_var("PATH", temp.path());
        let err = run_task_show(&mut state, &task("tr-1"), &[]).expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn task_status_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);

        std::env::set_var("PATH", temp.path());
        let err = run_task_status(&mut state, &task("tr-1")).expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn task_update_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let state = base_state(&temp);

        std::env::set_var("PATH", temp.path());
        let err = update_task_status(&state, &task("tr-1"), TaskStatus::InProgress)
            .expect_err("spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn reset_task_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let state = base_state(&temp);

        std::env::set_var("PATH", temp.path());
        let err = reset_task(&state, &task("tr-1")).expect_err("spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn agent_solve_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let state = base_state(&temp);

        std::env::set_var("PATH", temp.path());
        let err = run_agent_solve(&state, &[]).expect_err("spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn agent_review_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let state = base_state(&temp);

        std::env::set_var("PATH", temp.path());
        let err = run_agent_review(&state).expect_err("spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn next_task_spawn_errors_are_wrapped_in_quit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("next-task".to_string());

        std::env::set_var("PATH", temp.path());
        let quit = get_next_task_id(&state).expect_err("expected quit");
        assert_eq!(quit.code, 1);
        assert!(quit.reason.starts_with("next_task_failed:"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn next_task_invalid_task_id_is_wrapped_in_quit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf '$'".to_string());

        let quit = get_next_task_id(&state).expect_err("expected quit");
        assert_eq!(quit.code, 1);
        assert!(quit.reason.starts_with("next_task_invalid_task_id:"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn ensure_task_ready_wraps_task_status_errors_in_quit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.task_status = "exit 2".to_string();

        let quit = ensure_task_ready(&mut state, &task("tr-1")).expect_err("expected quit");
        assert_eq!(quit.code, 1);
        assert!(quit.reason.contains("task_status_failed:"));
    }

    #[cfg(unix)]
    #[test]
    fn run_hook_propagates_spawn_errors() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let state = base_state(&temp);

        std::env::set_var("PATH", temp.path());
        let err =
            run_hook(&state, "hook", &task("tr-1"), "hook").expect_err("expected spawn error");
        assert!(err.contains("Failed to run command"));

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn selection_task_status_errors_are_wrapped_in_quit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf 'tr-1'".to_string());
        state.config.commands.task_status = "exit 2".to_string();

        let quit = run_loop(&mut state).expect_err("expected quit");
        assert_eq!(quit.code, 1);
        assert!(quit.reason.contains("task_status_failed:"));
    }

    #[test]
    fn review_task_status_errors_are_wrapped_in_quit() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let marker = temp.path().join("status-marker");
        let marker_path = marker.display().to_string();
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf 'tr-1'".to_string());
        state.config.commands.task_show = "printf '[]\\n'".to_string();
        state.config.commands.task_status = format!(
            "if [ -f '{marker_path}' ]; then exit 2; fi; touch '{marker_path}'; printf 'open\\n'"
        );

        let quit = run_loop(&mut state).expect_err("expected quit");
        assert_eq!(quit.code, 1);
        assert!(quit.reason.contains("task_status_failed:"));
    }

    #[test]
    fn run_loop_interrupts_immediately_when_flag_is_set() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.interrupt_flag = Arc::new(AtomicBool::new(true));

        let quit = run_loop(&mut state).expect_err("expected interruption");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_interrupts_during_manual_task_precheck() {
        use std::time::Duration;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.task_status = "printf 'open\\n'".to_string();

        let interrupt_flag = Arc::new(AtomicBool::new(false));
        state.interrupt_flag = Arc::clone(&interrupt_flag);
        let filler = TaskId::try_from("x".repeat(20)).expect("task id");
        state.manual_tasks = vec![filler; 10_000];

        let setter = Arc::clone(&interrupt_flag);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1));
            setter.store(true, Ordering::SeqCst);
        });

        let quit = run_loop(&mut state).expect_err("expected interruption");
        handle.join().expect("join");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_interrupts_during_task_selection_loop() {
        use std::time::Duration;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf 'tr-1'".to_string());

        let interrupt_flag = Arc::new(AtomicBool::new(false));
        state.interrupt_flag = Arc::clone(&interrupt_flag);

        env::set_var("TRUDGER_SKIP_NOT_READY_LIMIT", "0".repeat(500_000));

        let setter = Arc::clone(&interrupt_flag);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1));
            setter.store(true, Ordering::SeqCst);
        });

        let quit = run_loop(&mut state).expect_err("expected interruption");
        handle.join().expect("join");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_interrupts_at_solving_loop_entry() {
        use std::time::Duration;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("sleep 0.05; printf 'tr-1'".to_string());
        state.config.commands.task_status = "printf 'open\\n'".to_string();

        let interrupt_flag = Arc::new(AtomicBool::new(false));
        state.interrupt_flag = Arc::clone(&interrupt_flag);

        let setter = Arc::clone(&interrupt_flag);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            setter.store(true, Ordering::SeqCst);
        });

        let quit = run_loop(&mut state).expect_err("expected interruption");
        handle.join().expect("join");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_interrupts_before_task_show_in_solving_phase() {
        use std::fs;
        use std::time::Duration;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf 'tr-1'".to_string());
        state.config.commands.task_status = "printf 'open\\n'".to_string();
        let update_started = temp.path().join("update-started");
        let update_gate = temp.path().join("update-gate");
        let started_path = update_started.display().to_string();
        let gate_path = update_gate.display().to_string();
        state.config.commands.task_update_in_progress = format!(
            "touch '{started_path}'; while [ ! -f '{gate_path}' ]; do sleep 0.01; done; true"
        );

        let interrupt_flag = Arc::new(AtomicBool::new(false));
        state.interrupt_flag = Arc::clone(&interrupt_flag);

        let setter = Arc::clone(&interrupt_flag);
        let handle = std::thread::spawn(move || {
            while !update_started.exists() {
                std::thread::sleep(Duration::from_millis(1));
            }
            setter.store(true, Ordering::SeqCst);
            fs::write(&update_gate, "").expect("open gate");
        });

        let quit = run_loop(&mut state).expect_err("expected interruption");
        handle.join().expect("join");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_interrupts_before_agent_solve() {
        use std::fs;
        use std::time::Duration;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf 'tr-1'".to_string());
        state.config.commands.task_status = "printf 'open\\n'".to_string();
        let show_started = temp.path().join("show-started");
        let show_gate = temp.path().join("show-gate");
        let started_path = show_started.display().to_string();
        let gate_path = show_gate.display().to_string();
        state.config.commands.task_show = format!(
            "touch '{started_path}'; while [ ! -f '{gate_path}' ]; do sleep 0.01; done; printf '[]\\n'"
        );

        let interrupt_flag = Arc::new(AtomicBool::new(false));
        state.interrupt_flag = Arc::clone(&interrupt_flag);

        let setter = Arc::clone(&interrupt_flag);
        let handle = std::thread::spawn(move || {
            while !show_started.exists() {
                std::thread::sleep(Duration::from_millis(1));
            }
            setter.store(true, Ordering::SeqCst);
            fs::write(&show_gate, "").expect("open gate");
        });

        let quit = run_loop(&mut state).expect_err("expected interruption");
        handle.join().expect("join");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_interrupts_before_task_show_in_review_phase() {
        use std::fs;
        use std::time::Duration;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf 'tr-1'".to_string());
        state.config.commands.task_status = "printf 'open\\n'".to_string();
        state.config.commands.task_show = "printf '[]\\n'".to_string();
        let solve_started = temp.path().join("solve-started");
        let solve_gate = temp.path().join("solve-gate");
        let started_path = solve_started.display().to_string();
        let gate_path = solve_gate.display().to_string();
        state.config.agent_command = format!(
            "touch '{started_path}'; while [ ! -f '{gate_path}' ]; do sleep 0.01; done; true"
        );

        let interrupt_flag = Arc::new(AtomicBool::new(false));
        state.interrupt_flag = Arc::clone(&interrupt_flag);

        let setter = Arc::clone(&interrupt_flag);
        let handle = std::thread::spawn(move || {
            while !solve_started.exists() {
                std::thread::sleep(Duration::from_millis(1));
            }
            setter.store(true, Ordering::SeqCst);
            fs::write(&solve_gate, "").expect("open gate");
        });

        let quit = run_loop(&mut state).expect_err("expected interruption");
        handle.join().expect("join");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_interrupts_before_agent_review() {
        use std::fs;
        use std::time::Duration;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let marker = temp.path().join("show-first");
        let marker_path = marker.display().to_string();
        let show_started = temp.path().join("review-show-started");
        let show_gate = temp.path().join("review-show-gate");
        let started_path = show_started.display().to_string();
        let gate_path = show_gate.display().to_string();
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf 'tr-1'".to_string());
        state.config.commands.task_status = "printf 'open\\n'".to_string();
        state.config.commands.task_show = format!(
            "if [ ! -f '{marker_path}' ]; then touch '{marker_path}'; printf '[]\\n'; exit 0; fi; \
             touch '{started_path}'; while [ ! -f '{gate_path}' ]; do sleep 0.01; done; printf '[]\\n'"
        );

        let interrupt_flag = Arc::new(AtomicBool::new(false));
        state.interrupt_flag = Arc::clone(&interrupt_flag);

        let setter = Arc::clone(&interrupt_flag);
        let handle = std::thread::spawn(move || {
            while !show_started.exists() {
                std::thread::sleep(Duration::from_millis(1));
            }
            setter.store(true, Ordering::SeqCst);
            fs::write(&show_gate, "").expect("open gate");
        });

        let quit = run_loop(&mut state).expect_err("expected interruption");
        handle.join().expect("join");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_interrupts_before_task_status_after_review() {
        use std::fs;
        use std::time::Duration;

        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("printf 'tr-1'".to_string());
        state.config.commands.task_status = "printf 'open\\n'".to_string();
        state.config.commands.task_show = "printf '[]\\n'".to_string();
        let review_started = temp.path().join("review-started");
        let review_gate = temp.path().join("review-gate");
        let started_path = review_started.display().to_string();
        let gate_path = review_gate.display().to_string();
        state.config.agent_review_command = format!(
            "touch '{started_path}'; while [ ! -f '{gate_path}' ]; do sleep 0.01; done; true"
        );

        let interrupt_flag = Arc::new(AtomicBool::new(false));
        state.interrupt_flag = Arc::clone(&interrupt_flag);

        let setter = Arc::clone(&interrupt_flag);
        let handle = std::thread::spawn(move || {
            while !review_started.exists() {
                std::thread::sleep(Duration::from_millis(1));
            }
            setter.store(true, Ordering::SeqCst);
            fs::write(&review_gate, "").expect("open gate");
        });

        let quit = run_loop(&mut state).expect_err("expected interruption");
        handle.join().expect("join");
        assert_eq!(quit.code, 130);
        assert_eq!(quit.reason, "interrupted");
    }
}
