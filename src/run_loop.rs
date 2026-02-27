use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::Value;

use crate::config::{Config, NotificationScope};
use crate::logger::{sanitize_log_value, Logger};
use crate::notification_payload::NotificationPayload;
use crate::shell::{
    run_shell_command_capture, run_shell_command_status, truncate_utf8_to_bytes, CommandEnv,
    CommandResult, TRUDGER_ENV_VALUE_MAX_BYTES,
};
use crate::task_types::{Phase, TaskId, TaskStatus};
use crate::tmux::TmuxState;

#[derive(Debug)]
pub(crate) struct RuntimeState {
    pub(crate) config: Config,
    pub(crate) config_path: PathBuf,
    pub(crate) invocation_folder: String,
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
    pub(crate) run_started_at: Instant,
    pub(crate) current_task_started_at: Option<Instant>,
    pub(crate) run_exit_code: i32,
}

#[derive(Debug, Default, Clone)]
struct AgentInvocationContext {
    profile: Option<String>,
    solve_invocation_id: Option<String>,
    review_invocation_id: Option<String>,
}

static AGENT_INVOCATION_CONTEXT: OnceLock<Mutex<AgentInvocationContext>> = OnceLock::new();

fn agent_invocation_context() -> &'static Mutex<AgentInvocationContext> {
    AGENT_INVOCATION_CONTEXT.get_or_init(|| Mutex::new(AgentInvocationContext::default()))
}

pub(crate) fn set_agent_invocation_context(
    profile: String,
    solve_invocation_id: String,
    review_invocation_id: String,
) {
    let mut context = agent_invocation_context()
        .lock()
        .expect("invocation context mutex");
    *context = AgentInvocationContext {
        profile: Some(profile),
        solve_invocation_id: Some(solve_invocation_id),
        review_invocation_id: Some(review_invocation_id),
    };
}

#[cfg(test)]
pub(crate) fn reset_agent_invocation_context() {
    let mut context = agent_invocation_context()
        .lock()
        .expect("invocation context mutex");
    *context = AgentInvocationContext::default();
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
    if config.commands.task_update_status.trim().is_empty() {
        return Err("commands.task_update_status must not be empty.".to_string());
    }
    if config.hooks.on_completed.trim().is_empty() {
        return Err("hooks.on_completed must not be empty.".to_string());
    }
    if config.hooks.on_requires_human.trim().is_empty() {
        return Err("hooks.on_requires_human must not be empty.".to_string());
    }

    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum NotificationEvent {
    RunStart,
    RunEnd,
    TaskStart,
    TaskEnd,
}

impl NotificationEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::RunStart => "run_start",
            Self::RunEnd => "run_end",
            Self::TaskStart => "task_start",
            Self::TaskEnd => "task_end",
        }
    }
}

fn first_non_empty_trimmed_line(value: &str) -> Option<String> {
    for line in value.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Common "first line" output for pretty-printed JSON. Skip these so a JSON payload doesn't
        // reduce the "description" to a single punctuation token.
        if matches!(trimmed, "[" | "]" | "{" | "}" | "}," | "],") {
            continue;
        }

        return Some(trimmed.to_string());
    }
    None
}

fn task_description_from_json(value: &Value) -> Option<String> {
    match value {
        Value::Array(entries) => entries.first().and_then(task_description_from_json),
        Value::Object(map) => {
            for key in ["title", "summary", "name"] {
                if let Some(Value::String(value)) = map.get(key) {
                    if !value.trim().is_empty() {
                        return Some(value.trim().to_string());
                    }
                }
            }

            // Common Jira-style payloads nest the human summary under `fields.summary`.
            if let Some(Value::Object(fields)) = map.get("fields") {
                for key in ["summary", "title", "name"] {
                    if let Some(Value::String(value)) = fields.get(key) {
                        if !value.trim().is_empty() {
                            return Some(value.trim().to_string());
                        }
                    }
                }
            }

            if let Some(Value::String(value)) = map.get("description") {
                return first_non_empty_trimmed_line(value);
            }

            None
        }
        _ => None,
    }
}

fn extract_task_description_from_task_show(task_show: &str) -> Option<String> {
    let trimmed = task_show.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            if let Some(description) = task_description_from_json(&value) {
                return Some(description);
            }
        }
    }

    first_non_empty_trimmed_line(task_show)
}

#[allow(clippy::too_many_arguments)]
fn build_command_env(
    state: &RuntimeState,
    task_id: Option<&TaskId>,
    agent_prompt: Option<String>,
    agent_phase: Option<String>,
    target_status: Option<String>,
    notify_event: Option<NotificationEvent>,
    agent_profile: Option<String>,
    agent_invocation_id: Option<String>,
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
        target_status,
        agent_prompt,
        agent_phase,
        completed,
        needs_human,
        notify_event: notify_event.map(|value| value.as_str().to_string()),
        notify_duration_ms: None,
        notify_folder: None,
        notify_exit_code: None,
        notify_task_id: None,
        notify_task_description: None,
        notify_message: None,
        notify_payload_path: None,
        agent_profile,
        agent_invocation_id,
    }
}

fn run_config_command(
    state: &RuntimeState,
    command: &str,
    task_id: Option<&TaskId>,
    log_label: &str,
    args: &[String],
) -> Result<CommandResult, String> {
    let env = build_command_env(state, task_id, None, None, None, None, None, None);
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
    target_status: Option<&str>,
    args: &[String],
) -> Result<i32, String> {
    let env = build_command_env(
        state,
        task_id,
        None,
        None,
        target_status.map(|value| value.to_string()),
        None,
        None,
        None,
    );
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
    agent_prompt: Option<String>,
    agent_phase: Option<String>,
) -> Result<i32, String> {
    let context = {
        let guard = agent_invocation_context()
            .lock()
            .expect("invocation context mutex");
        guard.clone()
    };

    let invocation_id = if agent_phase.as_deref() == Some("trudge_review") {
        context.review_invocation_id
    } else {
        context.solve_invocation_id
    };

    let env = build_command_env(
        state,
        None,
        agent_prompt,
        agent_phase,
        None,
        None,
        context.profile,
        invocation_id,
    );
    run_shell_command_status(command, log_label, "none", &[], &env, &state.logger)
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
    if state.current_task_id.as_ref() == Some(task_id) {
        let show = state.current_task_show.clone();
        let description = show
            .as_deref()
            .and_then(extract_task_description_from_task_show)
            .unwrap_or_default();
        state.logger.set_all_logs_task_show(show);
        state.logger.set_all_logs_task_description(description);
    }
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
    if state.current_task_id.as_ref() == Some(task_id) {
        state.logger.set_all_logs_task_status(
            state
                .current_task_status
                .as_ref()
                .map(|value| value.as_str()),
        );
    }
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
    let exit = run_config_command_status(
        state,
        &state.config.commands.task_update_status,
        Some(task_id),
        "task",
        Some(status.as_str()),
        &[],
    )?;
    if exit != 0 {
        return Err(format!(
            "task_update_status failed to set status {} (exit code {})",
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
    update_task_status(state, task_id, TaskStatus::Open)
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

    let exit = run_config_command_status(state, hook_command, Some(task_id), hook_name, None, &[])?;
    if exit != 0 {
        return Err(format!("hook {} failed with exit code {}", hook_name, exit));
    }
    Ok(())
}

fn should_dispatch_notification(state: &RuntimeState, event: NotificationEvent) -> bool {
    match state.config.hooks.effective_notification_scope() {
        Some(NotificationScope::TaskBoundaries) => {
            matches!(
                event,
                NotificationEvent::TaskStart | NotificationEvent::TaskEnd
            )
        }
        Some(NotificationScope::RunBoundaries) => {
            matches!(
                event,
                NotificationEvent::RunStart | NotificationEvent::RunEnd
            )
        }
        _ => false,
    }
}

pub(crate) fn dispatch_notification_hook(
    state: &RuntimeState,
    task_id: Option<&TaskId>,
    event: NotificationEvent,
) {
    let Some(hook_command) = state
        .config
        .hooks
        .on_notification
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    if !should_dispatch_notification(state, event) {
        return;
    }

    let mut env = build_command_env(state, task_id, None, None, None, Some(event), None, None);
    let notify_duration_ms = match event {
        NotificationEvent::RunStart | NotificationEvent::TaskStart => 0,
        NotificationEvent::RunEnd => state.run_started_at.elapsed().as_millis(),
        NotificationEvent::TaskEnd => state
            .current_task_started_at
            .map(|started_at| started_at.elapsed().as_millis())
            .unwrap_or(0),
    };
    let notify_task_id = task_id
        .or(state.current_task_id.as_ref())
        .map(|value| value.to_string())
        .unwrap_or_default();
    let notify_task_description = if notify_task_id.is_empty() {
        String::new()
    } else {
        state
            .current_task_show
            .as_deref()
            .and_then(extract_task_description_from_task_show)
            .unwrap_or_default()
    };
    let notify_exit_code =
        matches!(event, NotificationEvent::RunEnd).then_some(state.run_exit_code);

    env.notify_duration_ms = Some(notify_duration_ms.to_string());
    env.notify_folder = Some(state.invocation_folder.clone());
    env.notify_exit_code = notify_exit_code.map(|value| value.to_string());
    env.notify_task_id = Some(notify_task_id);
    env.notify_task_description = Some(notify_task_description);

    let task_token = task_id.map(|value| value.as_str()).unwrap_or("none");
    let payload_folder = env.notify_folder.clone().unwrap_or_default();
    let payload_task_id = env.notify_task_id.clone().unwrap_or_default();
    let payload_task_description = env.notify_task_description.clone().unwrap_or_default();
    let payload = NotificationPayload {
        event: event.as_str().to_string(),
        duration_ms: notify_duration_ms,
        folder: truncate_utf8_to_bytes(&payload_folder, TRUDGER_ENV_VALUE_MAX_BYTES).to_string(),
        exit_code: notify_exit_code,
        task_id: truncate_utf8_to_bytes(&payload_task_id, TRUDGER_ENV_VALUE_MAX_BYTES).to_string(),
        task_description: truncate_utf8_to_bytes(
            &payload_task_description,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        )
        .to_string(),
        message: None,
    };
    let payload_file = match payload.write_to_temp_file() {
        Ok(file) => file,
        Err(err) => {
            eprintln!("Warning: failed to prepare notification payload: {}.", err);
            state.logger.log_transition(&format!(
                "notification_hook_failed event={} task={} err={}",
                event.as_str(),
                task_token,
                sanitize_log_value(&err)
            ));
            return;
        }
    };
    env.notify_payload_path = Some(payload_file.path().display().to_string());

    match run_shell_command_status(
        hook_command,
        "on_notification",
        task_token,
        &[],
        &env,
        &state.logger,
    ) {
        Ok(0) => {}
        Ok(exit_code) => {
            eprintln!(
                "Warning: notification hook failed with exit code {}.",
                exit_code
            );
            state.logger.log_transition(&format!(
                "notification_hook_failed event={} task={} exit_code={}",
                event.as_str(),
                task_id.map(|value| value.as_str()).unwrap_or("none"),
                exit_code
            ));
        }
        Err(err) => {
            eprintln!("Warning: failed to run notification hook: {}.", err);
            state.logger.log_transition(&format!(
                "notification_hook_failed event={} task={} err={}",
                event.as_str(),
                task_id.map(|value| value.as_str()).unwrap_or("none"),
                sanitize_log_value(&err)
            ));
        }
    }
}

pub(crate) fn finish_current_task_context(state: &mut RuntimeState) {
    if let Some(task_id) = state.current_task_id.as_ref() {
        dispatch_notification_hook(state, Some(task_id), NotificationEvent::TaskEnd);
    }
    clear_current_task_context(state);
}

fn clear_current_task_context(state: &mut RuntimeState) {
    state.current_task_id = None;
    state.current_task_show = None;
    state.current_task_status = None;
    state.current_task_started_at = None;
    state.logger.set_all_logs_task_id(None);
}

fn run_agent_solve(state: &RuntimeState) -> Result<(), String> {
    let exit = run_agent_command(
        state,
        &state.config.agent_command,
        "agent_solve",
        Some(state.prompt_trudge.clone()),
        Some("trudge".to_string()),
    )?;
    if exit != 0 {
        return Err(format!("agent_solve failed with exit code {}", exit));
    }
    Ok(())
}

fn run_agent_review(state: &RuntimeState) -> Result<(), String> {
    let exit = run_agent_command(
        state,
        &state.config.agent_review_command,
        "agent_review",
        Some(state.prompt_review.clone()),
        Some("trudge_review".to_string()),
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
        state.logger.set_all_logs_task_id(Some(task_id.as_str()));
        state.current_task_started_at = Some(Instant::now());
        state.current_task_show = None;
        state.current_task_status = None;
        if state
            .config
            .hooks
            .on_notification
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            && should_dispatch_notification(state, NotificationEvent::TaskStart)
        {
            // Best-effort: attempt to populate task_show so task_start notifications can include a
            // useful `task_description` (for example a JSON `title` field) without making this an
            // additional failure point.
            let _ = run_task_show(state, &task_id, &[]);
        }
        dispatch_notification_hook(state, Some(&task_id), NotificationEvent::TaskStart);
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
            if let Err(_err) = run_agent_solve(state) {
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
                dispatch_notification_hook(state, Some(&task_id), NotificationEvent::TaskEnd);
                state.current_task_id = None;
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
                dispatch_notification_hook(state, Some(&task_id), NotificationEvent::TaskEnd);
                state.current_task_id = None;
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
            dispatch_notification_hook(state, Some(&task_id), NotificationEvent::TaskEnd);
            state.current_task_id = None;
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

        clear_current_task_context(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    fn task(id: &str) -> TaskId {
        TaskId::try_from(id).expect("task id")
    }

    fn hook_env_value(contents: &str, key: &str) -> Option<String> {
        let prefix = format!("env {}=", key);
        contents
            .lines()
            .find_map(|line| line.strip_prefix(&prefix).map(|value| value.to_string()))
    }

    fn hook_env_is_set(contents: &str, key: &str) -> Option<bool> {
        let prefix = format!("envset {}=", key);
        contents
            .lines()
            .find_map(|line| line.strip_prefix(&prefix).map(|value| value == "1"))
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
                    task_update_status: "true".to_string(),
                },
                hooks: crate::config::Hooks {
                    on_completed: "true".to_string(),
                    on_requires_human: "true".to_string(),
                    on_doctor_setup: None,
                    on_notification: None,
                    on_notification_scope: None,
                },
                review_loop_limit: crate::task_types::ReviewLoopLimit::new(1)
                    .expect("review_loop_limit"),
                log_path: None,
            },
            config_path: temp.path().join("trudger.yml"),
            invocation_folder: std::env::current_dir()
                .ok()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
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
            run_started_at: Instant::now(),
            current_task_started_at: None,
            run_exit_code: 0,
        }
    }

    fn setup_notification_hook_fixture(temp: &TempDir) -> PathBuf {
        let hook_log = temp.path().join("hook.log");
        let fixtures_bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        std::env::set_var("HOOK_MOCK_LOG", &hook_log);
        hook_log
    }

    #[test]
    fn build_command_env_joins_completed_tasks_with_commas() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.completed_tasks = vec![task("tr-1"), task("tr-2")];

        let env = build_command_env(&state, None, None, None, None, None, None, None);
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
    fn dispatch_notification_hook_is_noop_when_missing() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = temp.path().join("hook.log");
        std::env::set_var("HOOK_MOCK_LOG", &hook_log);

        let state = base_state(&temp);
        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        assert!(
            !hook_log.exists(),
            "notification hook should not run when missing"
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_sets_payload_path_env() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);

        let mut state = base_state(&temp);
        state.config.hooks.on_notification = Some("hook".to_string());
        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        assert!(
            hook_contents.contains("hook args_count=0 args="),
            "notification hook should still run without positional args, got:\n{hook_contents}"
        );
        assert!(
            hook_contents.contains("envset TRUDGER_NOTIFY_PAYLOAD_PATH=1"),
            "notification hook should expose payload path via env, got:\n{hook_contents}"
        );
        assert!(
            hook_contents.contains("notify_payload {\"event\":\"task_end\""),
            "notification hook payload should include task_end event, got:\n{hook_contents}"
        );
        assert!(
            hook_contents.contains("env TRUDGER_NOTIFY_EVENT=task_end"),
            "compat notify env fields should remain set for now, got:\n{hook_contents}"
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_sets_payload_fields_for_task_event() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);

        let mut state = base_state(&temp);
        state.config.hooks.on_notification = Some("hook".to_string());
        state.current_task_show = Some(" \n  Task summary line  \nsecond line".to_string());
        state.current_task_started_at = Some(Instant::now() - Duration::from_millis(20));

        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_EVENT").as_deref(),
            Some("task_end")
        );
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_TASK_ID").as_deref(),
            Some("tr-1")
        );
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_TASK_DESCRIPTION").as_deref(),
            Some("Task summary line")
        );
        assert!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_DURATION_MS")
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|value| value > 0),
            "expected task_end duration to be > 0, got:\n{hook_contents}"
        );
        assert_eq!(
            hook_env_is_set(&hook_contents, "TRUDGER_NOTIFY_EXIT_CODE"),
            Some(false)
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_uses_invocation_folder_not_current_dir() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let original_cwd = std::env::current_dir().expect("cwd");
        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);
        let invocation = temp.path().join("invocation");
        let other = temp.path().join("other");
        std::fs::create_dir_all(&invocation).expect("create invocation dir");
        std::fs::create_dir_all(&other).expect("create other dir");

        let mut state = base_state(&temp);
        state.config.hooks.on_notification = Some("hook".to_string());
        state.invocation_folder = invocation.display().to_string();

        std::env::set_current_dir(&other).expect("chdir");
        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        let expected = invocation.display().to_string();
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_FOLDER").as_deref(),
            Some(expected.as_str()),
            "expected TRUDGER_NOTIFY_FOLDER to use invocation cwd, got:\n{hook_contents}"
        );

        std::env::set_current_dir(&original_cwd).expect("restore cwd");
        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_extracts_title_from_json_task_show() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);

        let mut state = base_state(&temp);
        state.config.hooks.on_notification = Some("hook".to_string());
        state.current_task_show = Some(
            r#"
[
  {
    "title": "My Title",
    "description": "More details"
  }
]
"#
            .to_string(),
        );

        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_TASK_DESCRIPTION").as_deref(),
            Some("My Title")
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_uses_empty_description_for_whitespace_task_show() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);

        let mut state = base_state(&temp);
        state.config.hooks.on_notification = Some("hook".to_string());
        state.current_task_show = Some(" \n \t\n".to_string());

        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_TASK_ID").as_deref(),
            Some("tr-1")
        );
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_TASK_DESCRIPTION").as_deref(),
            Some("")
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_uses_empty_description_when_task_show_missing() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);

        let mut state = base_state(&temp);
        state.config.hooks.on_notification = Some("hook".to_string());
        state.current_task_show = None;

        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_TASK_ID").as_deref(),
            Some("tr-1")
        );
        assert_eq!(
            hook_env_value(&hook_contents, "TRUDGER_NOTIFY_TASK_DESCRIPTION").as_deref(),
            Some("")
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_run_start_and_run_end_payload_semantics() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);

        let mut state = base_state(&temp);
        state.config.hooks.on_notification = Some("hook".to_string());
        state.config.hooks.on_notification_scope = Some(NotificationScope::RunBoundaries);
        state.run_started_at = Instant::now() - Duration::from_millis(25);
        state.run_exit_code = 17;

        dispatch_notification_hook(&state, None, NotificationEvent::RunStart);
        dispatch_notification_hook(&state, None, NotificationEvent::RunEnd);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        let entries: Vec<&str> = hook_contents
            .split("hook args_count=0 args=\n")
            .filter(|entry| !entry.trim().is_empty())
            .collect();
        assert_eq!(entries.len(), 2, "expected run_start and run_end entries");

        let run_start = entries[0];
        assert_eq!(
            hook_env_value(run_start, "TRUDGER_NOTIFY_EVENT").as_deref(),
            Some("run_start")
        );
        assert_eq!(
            hook_env_value(run_start, "TRUDGER_NOTIFY_DURATION_MS").as_deref(),
            Some("0")
        );
        assert_eq!(
            hook_env_is_set(run_start, "TRUDGER_NOTIFY_EXIT_CODE"),
            Some(false)
        );
        assert_eq!(
            hook_env_value(run_start, "TRUDGER_NOTIFY_TASK_ID").as_deref(),
            Some("")
        );
        assert_eq!(
            hook_env_value(run_start, "TRUDGER_NOTIFY_TASK_DESCRIPTION").as_deref(),
            Some("")
        );

        let run_end = entries[1];
        assert_eq!(
            hook_env_value(run_end, "TRUDGER_NOTIFY_EVENT").as_deref(),
            Some("run_end")
        );
        assert!(
            hook_env_value(run_end, "TRUDGER_NOTIFY_DURATION_MS")
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|value| value > 0),
            "expected run_end duration > 0, got:\n{run_end}"
        );
        assert_eq!(
            hook_env_value(run_end, "TRUDGER_NOTIFY_EXIT_CODE").as_deref(),
            Some("17")
        );
        assert_eq!(
            hook_env_is_set(run_end, "TRUDGER_NOTIFY_EXIT_CODE"),
            Some(true)
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_respects_scope() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);

        let mut state = base_state(&temp);
        state.config.hooks.on_notification = Some("hook".to_string());

        dispatch_notification_hook(&state, None, NotificationEvent::RunStart);
        assert!(
            !hook_log.exists(),
            "run_start should not fire in default task_boundaries scope"
        );

        state.config.hooks.on_notification_scope = Some(NotificationScope::RunBoundaries);
        dispatch_notification_hook(&state, None, NotificationEvent::RunStart);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        assert!(
            hook_contents.contains("env TRUDGER_NOTIFY_EVENT=run_start"),
            "run_start should fire for run_boundaries scope, got:\n{hook_contents}"
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn dispatch_notification_hook_nonzero_exit_is_fail_open() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");

        let mut state = base_state(&temp);
        state.logger = Logger::new(Some(log_path.clone()));
        state.config.hooks.on_notification = Some("exit 7".to_string());

        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        let log_contents = std::fs::read_to_string(&log_path).expect("read log");
        assert!(
            log_contents.contains("notification_hook_failed event=task_end task=tr-1 exit_code=7"),
            "expected fail-open transition, got:\n{log_contents}"
        );

        crate::unit_tests::reset_test_env();
    }

    #[cfg(unix)]
    #[test]
    fn dispatch_notification_hook_spawn_error_is_fail_open() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        std::env::set_var("PATH", temp.path());

        let mut state = base_state(&temp);
        state.logger = Logger::new(Some(log_path.clone()));
        state.config.hooks.on_notification = Some("hook".to_string());

        dispatch_notification_hook(&state, Some(&task("tr-1")), NotificationEvent::TaskEnd);

        let log_contents = std::fs::read_to_string(&log_path).expect("read log");
        assert!(
            log_contents.contains(
                "notification_hook_failed event=task_end task=tr-1 err=Failed to run command"
            ),
            "expected fail-open spawn-error transition, got:\n{log_contents}"
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn run_loop_continues_when_notification_hook_fails() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let next_task_queue = temp.path().join("next-task-queue.txt");
        let status_queue = temp.path().join("status-queue.txt");
        std::fs::write(&next_task_queue, "tr-1\n\n").expect("write next-task queue");
        std::fs::write(&status_queue, "ready\nclosed\n").expect("write status queue");

        let fixtures_bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        std::env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
        std::env::set_var("TASK_STATUS_QUEUE", &status_queue);
        std::env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");

        let mut state = base_state(&temp);
        state.logger = Logger::new(Some(log_path.clone()));
        state.config.commands.next_task = Some("next-task".to_string());
        state.config.commands.task_show = "task-show \"$@\"".to_string();
        state.config.commands.task_status = "task-status".to_string();
        state.config.commands.task_update_status = "task-update \"$@\"".to_string();
        state.config.hooks.on_completed = "true".to_string();
        state.config.hooks.on_requires_human = "true".to_string();
        state.config.hooks.on_notification = Some("exit 7".to_string());

        let result = run_loop(&mut state).expect_err("expected graceful idle exit");
        assert_eq!(result.code, 0);
        assert_eq!(result.reason, "no_task");
        assert_eq!(state.completed_tasks, vec![task("tr-1")]);

        let log_contents = std::fs::read_to_string(&log_path).expect("read log");
        assert!(
            log_contents.contains("completed task=tr-1"),
            "task should still complete, got:\n{log_contents}"
        );
        assert!(
            log_contents
                .contains("notification_hook_failed event=task_start task=tr-1 exit_code=7"),
            "notification failure should be surfaced without aborting, got:\n{log_contents}"
        );

        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn run_loop_dispatches_task_boundary_notifications_once_each() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let hook_log = setup_notification_hook_fixture(&temp);
        let next_task_queue = temp.path().join("next-task-queue.txt");
        let status_queue = temp.path().join("status-queue.txt");
        std::fs::write(&next_task_queue, "tr-1\n\n").expect("write next-task queue");
        std::fs::write(&status_queue, "ready\nclosed\n").expect("write status queue");

        std::env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
        std::env::set_var("TASK_STATUS_QUEUE", &status_queue);
        std::env::set_var("TASK_SHOW_OUTPUT", "Task title\nmore details");

        let mut state = base_state(&temp);
        state.config.commands.next_task = Some("next-task".to_string());
        state.config.commands.task_show = "task-show \"$@\"".to_string();
        state.config.commands.task_status = "task-status".to_string();
        state.config.commands.task_update_status = "task-update \"$@\"".to_string();
        state.config.hooks.on_completed = "true".to_string();
        state.config.hooks.on_requires_human = "true".to_string();
        state.config.hooks.on_notification = Some("hook".to_string());

        let result = run_loop(&mut state).expect_err("expected graceful idle exit");
        assert_eq!(result.code, 0);
        assert_eq!(result.reason, "no_task");
        assert_eq!(state.completed_tasks, vec![task("tr-1")]);

        let hook_contents = std::fs::read_to_string(&hook_log).expect("read hook log");
        assert_eq!(
            hook_contents
                .matches("env TRUDGER_NOTIFY_EVENT=task_start")
                .count(),
            1,
            "expected exactly one task_start notification, got:\n{hook_contents}"
        );
        assert_eq!(
            hook_contents
                .matches("env TRUDGER_NOTIFY_EVENT=task_end")
                .count(),
            1,
            "expected exactly one task_end notification, got:\n{hook_contents}"
        );
        assert_eq!(
            hook_contents
                .matches("env TRUDGER_NOTIFY_EVENT=run_start")
                .count(),
            0,
            "task_boundaries scope should not emit run_start, got:\n{hook_contents}"
        );
        assert_eq!(
            hook_contents
                .matches("env TRUDGER_NOTIFY_EVENT=run_end")
                .count(),
            0,
            "task_boundaries scope should not emit run_end, got:\n{hook_contents}"
        );
        assert_eq!(
            hook_contents
                .matches("env TRUDGER_NOTIFY_EVENT=log")
                .count(),
            0,
            "task_boundaries scope should not emit log notifications, got:\n{hook_contents}"
        );

        let entries: Vec<&str> = hook_contents
            .split("hook args_count=0 args=\n")
            .filter(|entry| !entry.trim().is_empty())
            .collect();
        let task_start_entry = entries
            .iter()
            .find(|entry| entry.contains("env TRUDGER_NOTIFY_EVENT=task_start"))
            .expect("task_start entry");
        assert_eq!(
            hook_env_value(task_start_entry, "TRUDGER_NOTIFY_DURATION_MS").as_deref(),
            Some("0")
        );
        assert_eq!(
            hook_env_is_set(task_start_entry, "TRUDGER_NOTIFY_EXIT_CODE"),
            Some(false)
        );

        crate::unit_tests::reset_test_env();
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

    #[test]
    fn task_status_errors_on_unknown_status() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.task_status = "printf 'stalled\\n'".to_string();

        let err =
            run_task_status(&mut state, &task("tr-1")).expect_err("expected unknown status error");
        assert!(err.contains("unknown_task_status:tr-1:stalled"));
    }

    #[cfg(unix)]
    #[test]
    fn selection_loop_exits_when_no_ready_tasks_found() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let mut state = base_state(&temp);
        state.config.commands.task_status = "printf 'blocked\\n'".to_string();
        state.config.commands.next_task = Some({
            let queue = temp.path().join("next-task-queue.txt");
            std::fs::write(&queue, "tr-1\ntr-2\n").expect("write queue");
            let queue_path = queue.display();
            // Pop the first line from a queue file and print it.
            format!(
                "queue='{queue_path}'; if [ ! -f \"$queue\" ]; then exit 1; fi; IFS= read -r line < \"$queue\" || exit 1; tail -n +2 \"$queue\" > \"$queue.tmp\" && mv \"$queue.tmp\" \"$queue\"; printf '%s\\n' \"$line\""
            )
        });

        std::env::set_var("TRUDGER_SKIP_NOT_READY_LIMIT", "2");
        let quit = run_loop(&mut state).expect_err("expected idle exit");
        assert_eq!(quit.code, 0);
        assert_eq!(quit.reason, "no_ready_task");

        crate::unit_tests::reset_test_env();
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
        let err = run_agent_solve(&state).expect_err("spawn error");
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

    #[test]
    fn run_agent_commands_use_shared_invocation_for_solve_and_review() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let codex_log = temp.path().join("codex.log");
        let fixtures_bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        std::env::set_var("CODEX_MOCK_LOG", &codex_log);

        let mut state = base_state(&temp);
        state.config.agent_command = "codex --yolo exec --default \"$@\"".to_string();
        state.config.agent_review_command = "codex --yolo exec --review \"$@\"".to_string();
        state.current_task_id = Some(task("tr-1"));

        set_agent_invocation_context(
            "shared-profile".to_string(),
            "shared-id".to_string(),
            "shared-id".to_string(),
        );

        run_agent_solve(&state).expect("agent solve should succeed");
        run_agent_review(&state).expect("agent review should succeed");

        let contents = std::fs::read_to_string(&codex_log).expect("read codex log");
        let command_lines: Vec<&str> = contents
            .lines()
            .filter(|line| line.starts_with("codex "))
            .collect();
        assert_eq!(
            command_lines.len(),
            2,
            "expected one solve and one review call"
        );
        assert_eq!(command_lines[0].trim(), "codex --yolo exec --default");
        assert_eq!(command_lines[1].trim(), "codex --yolo exec --review");
        assert!(
            command_lines.iter().all(|line| !line.contains("tr-1")),
            "expected no positional task args appended to agent invocations, got:\n{contents}"
        );

        let invocation_lines: Vec<&str> = contents
            .lines()
            .filter(|line| line.starts_with("env TRUDGER_INVOCATION_ID="))
            .collect();
        assert_eq!(
            invocation_lines.len(),
            2,
            "expected both invocations to run"
        );
        assert!(
            invocation_lines
                .iter()
                .all(|line| *line == "env TRUDGER_INVOCATION_ID=shared-id"),
            "expected shared invocation id on both invocations, got:\n{contents}"
        );

        reset_agent_invocation_context();
        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn run_agent_commands_use_split_invocations_for_solve_and_review() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let codex_log = temp.path().join("codex.log");
        let fixtures_bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        std::env::set_var("CODEX_MOCK_LOG", &codex_log);

        let mut state = base_state(&temp);
        state.config.agent_command = "codex --yolo exec --default \"$@\"".to_string();
        state.config.agent_review_command = "codex --yolo exec --review \"$@\"".to_string();
        state.current_task_id = Some(task("tr-1"));

        set_agent_invocation_context(
            "split-profile".to_string(),
            "solve-id".to_string(),
            "review-id".to_string(),
        );

        run_agent_solve(&state).expect("agent solve should succeed");
        run_agent_review(&state).expect("agent review should succeed");

        let contents = std::fs::read_to_string(&codex_log).expect("read codex log");
        let command_lines: Vec<&str> = contents
            .lines()
            .filter(|line| line.starts_with("codex "))
            .collect();
        assert_eq!(
            command_lines.len(),
            2,
            "expected one solve and one review call"
        );
        assert_eq!(command_lines[0].trim(), "codex --yolo exec --default");
        assert_eq!(command_lines[1].trim(), "codex --yolo exec --review");

        let invocation_lines: Vec<&str> = contents
            .lines()
            .filter(|line| line.starts_with("env TRUDGER_INVOCATION_ID="))
            .collect();
        assert_eq!(
            invocation_lines.len(),
            2,
            "expected both invocations to run"
        );
        assert_eq!(invocation_lines[0], "env TRUDGER_INVOCATION_ID=solve-id");
        assert_eq!(invocation_lines[1], "env TRUDGER_INVOCATION_ID=review-id");

        assert!(!command_lines[1].contains("tr-1"));

        reset_agent_invocation_context();
        crate::unit_tests::reset_test_env();
    }

    #[test]
    fn run_agent_commands_emit_profile_invocation_contract_and_remove_legacy_env() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let temp = TempDir::new().expect("temp dir");
        let codex_log = temp.path().join("codex.log");
        let fixtures_bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        std::env::set_var("CODEX_MOCK_LOG", &codex_log);
        std::env::set_var("TRUDGER_PROMPT", "legacy-prompt");
        std::env::set_var("TRUDGER_REVIEW_PROMPT", "legacy-review-prompt");

        let mut state = base_state(&temp);
        state.config.agent_command = "codex".to_string();
        state.config.agent_review_command = "codex".to_string();
        state.current_task_id = Some(task("tr-1"));

        set_agent_invocation_context(
            "manual-profile".to_string(),
            "solve-id".to_string(),
            "review-id".to_string(),
        );

        state.prompt_trudge = "solve prompt".to_string();
        state.prompt_review = "review prompt".to_string();

        run_agent_solve(&state).expect("agent solve should succeed");
        run_agent_review(&state).expect("agent review should succeed");

        let contents = std::fs::read_to_string(&codex_log).expect("read codex log");
        let invocation_lines: Vec<&str> = contents
            .lines()
            .filter(|line| line.starts_with("env TRUDGER_INVOCATION_ID="))
            .collect();

        assert_eq!(invocation_lines.len(), 2, "expected both invocations to run");
        assert_eq!(invocation_lines[0], "env TRUDGER_INVOCATION_ID=solve-id");
        assert_eq!(invocation_lines[1], "env TRUDGER_INVOCATION_ID=review-id");

        assert!(
            contents.contains("envset TRUDGER_AGENT_PROMPT=1"),
            "agent invocations should set TRUDGER_AGENT_PROMPT"
        );
        assert!(
            contents.contains("env TRUDGER_AGENT_PROMPT=solve prompt"),
            "solve invocation should forward the solve prompt"
        );
        assert!(
            contents.contains("env TRUDGER_AGENT_PROMPT=review prompt"),
            "review invocation should forward the review prompt"
        );
        assert!(
            contents.contains("envset TRUDGER_AGENT_PHASE=1"),
            "agent invocations should set TRUDGER_AGENT_PHASE"
        );
        assert!(
            contents.contains("env TRUDGER_AGENT_PHASE=trudge"),
            "solve invocation should set TRUDGER_AGENT_PHASE=trudge"
        );
        assert!(
            contents.contains("env TRUDGER_AGENT_PHASE=trudge_review"),
            "review invocation should set TRUDGER_AGENT_PHASE=trudge_review"
        );
        assert!(
            contents.contains("envset TRUDGER_PROFILE=1"),
            "agent invocations should set TRUDGER_PROFILE"
        );
        assert!(
            contents.contains("env TRUDGER_PROFILE=manual-profile"),
            "solve/review invocations should inherit active profile"
        );
        assert!(
            contents.contains("envset TRUDGER_PROMPT=0"),
            "legacy TRUDGER_PROMPT should be removed from agent env"
        );
        assert!(
            contents.contains("envset TRUDGER_REVIEW_PROMPT=0"),
            "legacy TRUDGER_REVIEW_PROMPT should be removed from agent env"
        );

        reset_agent_invocation_context();
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
        state.config.commands.task_update_status = format!(
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
