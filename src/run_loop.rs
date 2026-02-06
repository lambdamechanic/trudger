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
    pub(crate) manual_tasks: Vec<String>,
    pub(crate) completed_tasks: Vec<String>,
    pub(crate) needs_human_tasks: Vec<String>,
    pub(crate) current_task_id: Option<String>,
    pub(crate) current_task_show: Option<String>,
    pub(crate) current_task_status: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Quit {
    pub(crate) code: i32,
    #[allow(dead_code)]
    pub(crate) reason: String,
}

impl Quit {
    pub(crate) fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code as u8)
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

pub(crate) fn validate_config(config: &Config, manual_tasks: &[String]) -> Result<(), String> {
    if config.agent_command.trim().is_empty() {
        return Err("agent_command must not be empty.".to_string());
    }
    if config.agent_review_command.trim().is_empty() {
        return Err("agent_review_command must not be empty.".to_string());
    }
    if config.review_loop_limit < 1 {
        return Err(format!(
            "review_loop_limit must be a positive integer (got {}).",
            config.review_loop_limit
        ));
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
    task_id: Option<&str>,
    prompt: Option<String>,
    review_prompt: Option<String>,
) -> CommandEnv {
    let completed = if state.completed_tasks.is_empty() {
        None
    } else {
        Some(state.completed_tasks.join(","))
    };
    let needs_human = if state.needs_human_tasks.is_empty() {
        None
    } else {
        Some(state.needs_human_tasks.join(","))
    };

    CommandEnv {
        cwd: None,
        config_path: state.config_path.display().to_string(),
        scratch_dir: None,
        task_id: task_id
            .map(|value| value.to_string())
            .or_else(|| state.current_task_id.clone()),
        task_show: state.current_task_show.clone(),
        task_status: state.current_task_status.clone(),
        prompt,
        review_prompt,
        completed,
        needs_human,
    }
}

fn run_config_command(
    state: &RuntimeState,
    command: &str,
    task_id: Option<&str>,
    log_label: &str,
    args: &[String],
) -> Result<CommandResult, String> {
    let env = build_command_env(state, task_id, None, None);
    run_shell_command_capture(
        command,
        log_label,
        task_id.unwrap_or("none"),
        args,
        &env,
        &state.logger,
    )
}

fn run_config_command_status(
    state: &RuntimeState,
    command: &str,
    task_id: Option<&str>,
    log_label: &str,
    args: &[String],
) -> Result<i32, String> {
    let env = build_command_env(state, task_id, None, None);
    run_shell_command_status(
        command,
        log_label,
        task_id.unwrap_or("none"),
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

fn run_task_show(state: &mut RuntimeState, task_id: &str, args: &[String]) -> Result<(), String> {
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

fn run_task_status(state: &mut RuntimeState, task_id: &str) -> Result<(), String> {
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
    let status = output
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    state.current_task_status = if status.is_empty() {
        None
    } else {
        Some(status)
    };
    Ok(())
}

fn get_next_task_id(state: &RuntimeState) -> Result<String, Quit> {
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

    let task_id = output
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    Ok(task_id)
}

pub(crate) fn is_ready_status(status: &str) -> bool {
    status == "ready" || status == "open"
}

fn ensure_task_ready(state: &mut RuntimeState, task_id: &str) -> Result<(), Quit> {
    run_task_status(state, task_id)
        .map_err(|err| quit(&state.logger, &format!("task_status_failed:{err}"), 1))?;
    let status = state.current_task_status.clone().unwrap_or_default();
    if is_ready_status(&status) {
        return Ok(());
    }
    eprintln!("Task {} is not ready (status: {}).", task_id, status);
    Err(quit(
        &state.logger,
        &format!("task_not_ready:{}", task_id),
        1,
    ))
}

fn update_task_status(state: &RuntimeState, task_id: &str, status: &str) -> Result<(), String> {
    let args = vec!["--status".to_string(), status.to_string()];
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
            status, exit
        ));
    }
    Ok(())
}

fn update_in_progress(state: &RuntimeState, task_id: &str) -> Result<(), String> {
    update_task_status(state, task_id, "in_progress")
}

fn reset_task(state: &RuntimeState, task_id: &str) -> Result<(), String> {
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

pub(crate) fn reset_task_on_exit(state: &RuntimeState, result: &Result<(), Quit>) {
    if result.is_ok() {
        return;
    }
    let Some(task_id) = state.current_task_id.as_deref() else {
        return;
    };
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
    task_id: &str,
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
                let task_id = get_next_task_id(state)?;
                if task_id.trim().is_empty() {
                    state.logger.log_transition("idle no_task");
                    return Err(quit(&state.logger, "no_task", 0));
                }
                run_task_status(state, &task_id)
                    .map_err(|err| quit(&state.logger, &format!("task_status_failed:{err}"), 1))?;
                let status = state.current_task_status.clone().unwrap_or_default();
                if status.is_empty() {
                    eprintln!("Task {} missing status.", task_id);
                    return Err(quit(
                        &state.logger,
                        &format!("task_missing_status:{}", task_id),
                        1,
                    ));
                }
                if is_ready_status(&status) {
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

        if task_id.trim().is_empty() {
            state.logger.log_transition("idle no_task");
            return Err(quit(&state.logger, "no_task", 0));
        }

        state.current_task_id = Some(task_id.clone());
        state.current_task_show = None;
        state.current_task_status = None;
        let resume_args = vec!["resume".to_string(), "--last".to_string()];
        let mut review_loops: u64 = 0;

        loop {
            check_interrupted(state)?;
            state.tmux.update_name(
                "SOLVING",
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
                    "ERROR",
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }

            check_interrupted(state)?;
            if let Err(err) = run_task_show(state, &task_id, &[]) {
                state.tmux.update_name(
                    "ERROR",
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
                    "ERROR",
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
                "REVIEWING",
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
                    "ERROR",
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }

            check_interrupted(state)?;
            if let Err(_err) = run_agent_review(state) {
                state.tmux.update_name(
                    "ERROR",
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
            let status = state.current_task_status.clone().unwrap_or_default();
            state
                .logger
                .log_transition(&format!("review_state task={} status={}", task_id, status));

            if status.is_empty() {
                state.tmux.update_name(
                    "ERROR",
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
            }

            if status == "closed" {
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

            if status == "blocked" {
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
            if review_loops < state.config.review_loop_limit {
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
            if let Err(err) = update_task_status(state, &task_id, "blocked") {
                state.tmux.update_name(
                    "ERROR",
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }
            state.current_task_status = Some("blocked".to_string());

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

        let completed_env = state.completed_tasks.join(",");
        state
            .logger
            .log_transition(&format!("env TRUDGER_COMPLETED={}", completed_env));
        let needs_human_env = state.needs_human_tasks.join(",");
        env::set_var("TRUDGER_COMPLETED", &completed_env);
        env::set_var("TRUDGER_NEEDS_HUMAN", &needs_human_env);

        state.current_task_id = None;
        state.current_task_show = None;
        state.current_task_status = None;
    }
}
