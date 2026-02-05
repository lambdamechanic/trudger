use chrono::Utc;
use regex::Regex;
use shell_escape::unix::escape;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod config;

use crate::config::{load_config, Config};

const PROMPT_TRUDGE: &str = ".codex/prompts/trudge.md";
const PROMPT_REVIEW: &str = ".codex/prompts/trudge_review.md";
const DEFAULT_CONFIG_REL: &str = ".config/trudger.yml";

fn usage() {
    println!(
        "Usage: ./trudger [options] [task_id ...]\n\n\
Loop over ready br tasks and run Codex solve+review prompts.\n\
If task IDs are provided, they run first (in order) before br ready tasks.\n\
Configuration is loaded from ~/.config/trudger.yml by default.\n\n\
Options:\n\
  -c, --config PATH   Load configuration from PATH instead of ~/.config/trudger.yml.\n\
  -h, --help          Show this help text."
    );
}

fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "Missing HOME environment variable".to_string())
}

fn require_file(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("Missing {}: {}", label, path.display()));
    }
    Ok(())
}

fn bootstrap_config_error(default_path: &Path) -> String {
    format!(
        "Missing config file: {}\n\n\
Sample configurations:\n\n\
1) Trudgeable with hooks\n\
   - Selects the next ready br task labeled \"trudgeable\".\n\
   - On completion, removes the \"trudgeable\" label.\n\
   - On requires-human, removes \"trudgeable\" and adds \"human-required\".\n\
   mkdir -p ~/.config && curl -fsSL https://raw.githubusercontent.com/lambdamechanic/trudger/main/sample_configuration/trudgeable-with-hooks.yml -o ~/.config/trudger.yml\n\n\
2) Robot triage\n\
   - Selects tasks via `bv --robot-next`.\n\
   - No label changes (hooks are no-ops).\n\
   mkdir -p ~/.config && curl -fsSL https://raw.githubusercontent.com/lambdamechanic/trudger/main/sample_configuration/robot-triage.yml -o ~/.config/trudger.yml",
        default_path.display()
    )
}

#[derive(Debug)]
struct Logger {
    path: Option<PathBuf>,
}

impl Logger {
    fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    fn log_transition(&self, message: &str) {
        let Some(path) = &self.path else {
            return;
        };
        let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let sanitized = sanitize_log_value(message);
        let line = format!("{} {}\n", ts, sanitized);
        let mut file = match fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(file) => file,
            Err(_) => return,
        };
        let _ = file.write_all(line.as_bytes());
    }
}

fn sanitize_log_value(value: &str) -> String {
    value
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn render_prompt(path: &Path) -> Result<String, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read prompt {}: {}", path.display(), err))?;
    let mut lines = content.lines();
    let mut out = String::new();
    let mut in_frontmatter = false;
    let mut first_line = true;

    while let Some(line) = lines.next() {
        if first_line && line == "---" {
            in_frontmatter = true;
            first_line = false;
            continue;
        }
        first_line = false;
        if in_frontmatter && line == "---" {
            in_frontmatter = false;
            continue;
        }
        if !in_frontmatter {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.ends_with('\n') {
        out.pop();
    }
    Ok(out)
}

fn render_args(args: &[String]) -> String {
    if args.is_empty() {
        return String::new();
    }
    let mut rendered = String::new();
    for arg in args {
        rendered.push_str(&escape(arg.into()).to_string());
        rendered.push(' ');
    }
    rendered
}

#[derive(Debug, Clone)]
struct CommandEnv {
    config_path: String,
    task_id: Option<String>,
    task_show: Option<String>,
    task_status: Option<String>,
    prompt: Option<String>,
    review_prompt: Option<String>,
    completed: Option<String>,
    needs_human: Option<String>,
}

impl CommandEnv {
    fn apply(&self, cmd: &mut Command) {
        cmd.env("TRUDGER_CONFIG_PATH", &self.config_path);
        Self::apply_optional(cmd, "TRUDGER_TASK_ID", &self.task_id);
        Self::apply_optional(cmd, "TRUDGER_TASK_SHOW", &self.task_show);
        Self::apply_optional(cmd, "TRUDGER_TASK_STATUS", &self.task_status);
        Self::apply_optional(cmd, "TRUDGER_PROMPT", &self.prompt);
        Self::apply_optional(cmd, "TRUDGER_REVIEW_PROMPT", &self.review_prompt);
        Self::apply_optional(cmd, "TRUDGER_COMPLETED", &self.completed);
        Self::apply_optional(cmd, "TRUDGER_NEEDS_HUMAN", &self.needs_human);
    }

    fn apply_optional(cmd: &mut Command, key: &str, value: &Option<String>) {
        match value {
            Some(value) => {
                cmd.env(key, value);
            }
            None => {
                cmd.env_remove(key);
            }
        }
    }
}

#[derive(Debug)]
struct CommandResult {
    stdout: String,
    exit_code: i32,
}

fn run_shell_command_capture(
    command: &str,
    log_label: &str,
    task_token: &str,
    args: &[String],
    env: &CommandEnv,
    logger: &Logger,
) -> Result<CommandResult, String> {
    if command.is_empty() {
        return Ok(CommandResult {
            stdout: String::new(),
            exit_code: 0,
        });
    }

    let args_render = render_args(args);
    logger.log_transition(&format!(
        "cmd start label={} task={} mode=bash_lc command={} args={}",
        log_label,
        task_token,
        sanitize_log_value(command),
        sanitize_log_value(&args_render)
    ));

    let mut cmd = Command::new("bash");
    cmd.arg("-lc").arg(command);
    if !args.is_empty() {
        cmd.arg("--");
        cmd.args(args);
    }
    env.apply(&mut cmd);
    let output = cmd
        .output()
        .map_err(|err| format!("Failed to run command '{}': {}", command, err))?;

    let exit_code = output.status.code().unwrap_or(1);
    logger.log_transition(&format!(
        "cmd exit label={} task={} exit={}",
        log_label, task_token, exit_code
    ));

    Ok(CommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        exit_code,
    })
}

fn run_shell_command_status(
    command: &str,
    log_label: &str,
    task_token: &str,
    args: &[String],
    env: &CommandEnv,
    logger: &Logger,
) -> Result<i32, String> {
    if command.is_empty() {
        return Ok(0);
    }

    let args_render = render_args(args);
    logger.log_transition(&format!(
        "cmd start label={} task={} mode=bash_lc command={} args={}",
        log_label,
        task_token,
        sanitize_log_value(command),
        sanitize_log_value(&args_render)
    ));

    let mut cmd = Command::new("bash");
    cmd.arg("-lc").arg(command);
    if !args.is_empty() {
        cmd.arg("--");
        cmd.args(args);
    }
    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());
    env.apply(&mut cmd);
    let status = cmd
        .status()
        .map_err(|err| format!("Failed to run command '{}': {}", command, err))?;

    let exit_code = status.code().unwrap_or(1);
    logger.log_transition(&format!(
        "cmd exit label={} task={} exit={}",
        log_label, task_token, exit_code
    ));

    Ok(exit_code)
}

fn command_exists(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|path| {
        let full = path.join(name);
        full.is_file() || full.is_symlink()
    })
}

#[derive(Debug, Clone)]
struct TmuxState {
    enabled: bool,
    base_name: String,
    original_title: String,
}

impl TmuxState {
    fn new() -> Self {
        let enabled = env::var("TMUX").is_ok() && command_exists("tmux");
        if !enabled {
            return Self {
                enabled: false,
                base_name: String::new(),
                original_title: String::new(),
            };
        }

        let session_name = env::var("TRUDGER_TMUX_SESSION_NAME")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| tmux_display("#S"));
        if let Some(value) = &session_name {
            env::set_var("TRUDGER_TMUX_SESSION_NAME", value);
        }

        let original_title = env::var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| tmux_display("#{pane_title}"))
            .unwrap_or_default();
        if !original_title.is_empty() {
            env::set_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE", &original_title);
        }

        let mut base_name = original_title.clone();
        if !base_name.is_empty() {
            let re = Regex::new(r" (COMPLETED|NEEDS_HUMAN) \[[^\]]*\]").unwrap();
            base_name = re.replace_all(&base_name, "").to_string();
            let re = Regex::new(r" (SOLVING|REVIEWING) .*$").unwrap();
            base_name = re.replace_all(&base_name, "").to_string();
            let re = Regex::new(r" HALTED ON ERROR .*$").unwrap();
            base_name = re.replace_all(&base_name, "").to_string();
        }

        if base_name.trim().is_empty() {
            base_name = default_tmux_base_name();
        }

        let state = Self {
            enabled: true,
            base_name,
            original_title,
        };
        state.select_pane(&state.base_name);
        state
    }

    fn select_pane(&self, name: &str) {
        if !self.enabled {
            return;
        }
        let _ = Command::new("tmux")
            .arg("select-pane")
            .arg("-T")
            .arg(name)
            .status();
    }

    fn update_name(&self, phase: &str, task_id: &str, completed: &[String], needs_human: &[String]) {
        if !self.enabled {
            return;
        }
        let name = build_tmux_name(&self.base_name, phase, task_id, completed, needs_human);
        self.select_pane(&name);
    }

    fn restore(&self) {
        if !self.enabled {
            return;
        }
        if !self.original_title.is_empty() {
            self.select_pane(&self.original_title);
        }
    }
}

fn tmux_display(format: &str) -> Option<String> {
    let output = Command::new("tmux")
        .arg("display-message")
        .arg("-p")
        .arg(format)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn default_tmux_base_name() -> String {
    let host = Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .and_then(|output| {
                    if output.status.success() {
                        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                    } else {
                        None
                    }
                })
        })
        .unwrap_or_else(|| "host".to_string());

    let folder = env::current_dir()
        .ok()
        .and_then(|path| path.file_name().map(|v| v.to_string_lossy().to_string()))
        .unwrap_or_else(|| "".to_string());
    let command = env::args().next().unwrap_or_else(|| "trudger".to_string());
    let command = Path::new(&command)
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or(command);
    format!("({}) {}: {}", host, folder, command)
}

fn format_task_list(label: &str, tasks: &[String]) -> String {
    if tasks.is_empty() {
        return String::new();
    }
    format!("{} [{}]", label, tasks.join(", "))
}

fn build_tmux_name(
    base_name: &str,
    phase: &str,
    task_id: &str,
    completed: &[String],
    needs_human: &[String],
) -> String {
    let mut base = base_name.to_string();
    if let Some((prefix, command)) = base_name.rsplit_once(": ") {
        if command == "fg" || command == "codex" {
            base = prefix.to_string();
        }
    }

    let activity = match phase {
        "SOLVING" => format!("SOLVING {}", task_id),
        "REVIEWING" => format!("REVIEWING {}", task_id),
        "ERROR" => format!("HALTED ON ERROR {}", task_id),
        _ => String::new(),
    };

    let mut parts = Vec::new();
    parts.push(base);
    let completed_segment = format_task_list("COMPLETED", completed);
    let needs_human_segment = format_task_list("NEEDS_HUMAN", needs_human);
    if !completed_segment.is_empty() {
        parts.push(completed_segment);
    }
    if !needs_human_segment.is_empty() {
        parts.push(needs_human_segment);
    }
    if !activity.is_empty() {
        parts.push(activity);
    }

    parts.join(" ")
}

#[derive(Debug)]
struct RuntimeState {
    config: Config,
    config_path: PathBuf,
    prompt_trudge: String,
    prompt_review: String,
    logger: Logger,
    tmux: TmuxState,
    interrupt_flag: Arc<AtomicBool>,
    manual_tasks: Vec<String>,
    completed_tasks: Vec<String>,
    needs_human_tasks: Vec<String>,
    current_task_id: Option<String>,
    current_task_show: Option<String>,
    current_task_status: Option<String>,
}

#[derive(Debug)]
struct Quit {
    code: i32,
    #[allow(dead_code)]
    reason: String,
}

impl Quit {
    fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code as u8)
    }
}

fn quit(logger: &Logger, reason: &str, code: i32) -> Quit {
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

fn validate_config(config: &Config, manual_tasks: &[String]) -> Result<(), String> {
    if config.agent_command.trim().is_empty() {
        return Err("agent_command must not be empty.".to_string());
    }
    if config.agent_review_command.trim().is_empty() {
        return Err("agent_review_command must not be empty.".to_string());
    }
    if config.log_path.trim().is_empty() {
        return Err("log_path must not be empty.".to_string());
    }
    if config.review_loop_limit < 1 {
        return Err(format!(
            "review_loop_limit must be a positive integer (got {}).",
            config.review_loop_limit
        ));
    }

    let next_task = config
        .commands
        .next_task
        .as_deref()
        .unwrap_or("")
        .trim();
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
        config_path: state.config_path.display().to_string(),
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
        return Err(format!("task_show failed with exit code {}", output.exit_code));
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
        return Err(format!("task_status failed with exit code {}", output.exit_code));
    }
    let status = output
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    state.current_task_status = if status.is_empty() { None } else { Some(status) };
    Ok(())
}

fn get_next_task_id(state: &RuntimeState) -> Result<String, Quit> {
    let output = run_config_command(
        state,
        state
            .config
            .commands
            .next_task
            .as_deref()
            .unwrap_or(""),
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

fn is_ready_status(status: &str) -> bool {
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

fn update_in_progress(state: &RuntimeState, task_id: &str) -> Result<(), String> {
    let args = vec!["--status".to_string(), "in_progress".to_string()];
    let exit = run_config_command_status(
        state,
        &state.config.commands.task_update_in_progress,
        Some(task_id),
        "task",
        &args,
    )?;
    if exit != 0 {
        return Err(format!("task_update_in_progress failed with exit code {}", exit));
    }
    Ok(())
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

fn reset_task_on_exit(state: &RuntimeState, result: &Result<(), Quit>) {
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

fn run_hook(state: &RuntimeState, hook_command: &str, task_id: &str, hook_name: &str) -> Result<(), String> {
    let exit = run_config_command_status(
        state,
        hook_command,
        Some(task_id),
        hook_name,
        &[],
    )?;
    if exit != 0 {
        return Err(format!("hook {} failed with exit code {}", hook_name, exit));
    }
    Ok(())
}

fn run_agent_solve(state: &RuntimeState) -> Result<(), String> {
    let exit = run_agent_command(
        state,
        &state.config.agent_command,
        "agent_solve",
        Some(state.prompt_trudge.clone()),
        None,
        &[],
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

fn run_loop(state: &mut RuntimeState) -> Result<(), Quit> {
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
                state.logger.log_transition("idle missing_next_task_command");
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
                state
                    .logger
                    .log_transition(&format!("skip_not_ready task={} status={}", task_id, status));
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
        let review_loops = 0;

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
            if let Err(err) = run_task_show(state, &task_id, &["--json".to_string()]) {
                state.tmux.update_name(
                    "ERROR",
                    &task_id,
                    &state.completed_tasks,
                    &state.needs_human_tasks,
                );
                state.logger.log_transition(&format!("error task={}", task_id));
                return Err(quit(&state.logger, &format!("error:{err}"), 1));
            }

            check_interrupted(state)?;
            if let Err(_err) = run_agent_solve(state) {
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
                return Err(quit(
                    &state.logger,
                    &format!("solve_failed:{}", task_id),
                    1,
                ));
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
            if let Err(err) = run_task_show(state, &task_id, &["--json".to_string()]) {
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
                if let Err(err) = run_hook(state, &state.config.hooks.on_completed, &task_id, "on_completed") {
                    return Err(quit(&state.logger, &format!("error:{err}"), 1));
                }
                break;
            }

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

fn run() -> Result<(), Quit> {
    let mut args = env::args().skip(1).peekable();
    let mut config_path: Option<PathBuf> = None;
    let mut config_path_source_flag = false;
    let mut manual_tasks: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                usage();
                return Ok(());
            }
            "-c" | "--config" => {
                let Some(value) = args.next() else {
                    eprintln!("Missing value for {}", arg);
                    usage();
                    return Err(Quit {
                        code: 1,
                        reason: "missing_option_value".to_string(),
                    });
                };
                if value.is_empty() {
                    eprintln!("Missing value for {}", arg);
                    usage();
                    return Err(Quit {
                        code: 1,
                        reason: "missing_option_value".to_string(),
                    });
                }
                config_path = Some(PathBuf::from(value));
                config_path_source_flag = true;
            }
            "--" => {
                manual_tasks.extend(args.map(|v| v));
                break;
            }
            _ if arg.starts_with("--config=") => {
                let value = arg.trim_start_matches("--config=");
                if value.is_empty() {
                    eprintln!("Missing value for --config");
                    usage();
                    return Err(Quit {
                        code: 1,
                        reason: "missing_option_value".to_string(),
                    });
                }
                config_path = Some(PathBuf::from(value));
                config_path_source_flag = true;
            }
            _ if arg.starts_with('-') => {
                eprintln!("Unknown option: {}", arg);
                usage();
                return Err(Quit {
                    code: 1,
                    reason: "unknown_option".to_string(),
                });
            }
            _ => {
                manual_tasks.push(arg);
            }
        }
    }

    let home = home_dir().map_err(|message| Quit {
        code: 1,
        reason: message,
    })?;

    let prompt_trudge = home.join(PROMPT_TRUDGE);
    let prompt_review = home.join(PROMPT_REVIEW);
    if let Err(message) = require_file(&prompt_trudge, "prompt file") {
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: message,
        });
    }
    if let Err(message) = require_file(&prompt_review, "prompt file") {
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: message,
        });
    }

    let default_config = home.join(DEFAULT_CONFIG_REL);
    let config_path = config_path.unwrap_or_else(|| default_config.clone());
    if !config_path.is_file() {
        if config_path_source_flag {
            eprintln!("Missing config file: {}", config_path.display());
        } else {
            eprintln!("{}", bootstrap_config_error(&default_config));
        }
        return Err(Quit {
            code: 1,
            reason: format!("missing_config:{}", config_path.display()),
        });
    }

    let loaded = load_config(&config_path).map_err(|message| Quit {
        code: 1,
        reason: message,
    })?;

    if let Err(message) = validate_config(&loaded.config, &manual_tasks) {
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: message,
        });
    }

    let prompt_trudge_content = render_prompt(&prompt_trudge).map_err(|message| Quit {
        code: 1,
        reason: message,
    })?;
    let prompt_review_content = render_prompt(&prompt_review).map_err(|message| Quit {
        code: 1,
        reason: message,
    })?;

    let log_path = loaded.config.log_path.trim();
    let logger = Logger::new(if log_path.is_empty() {
        None
    } else {
        Some(PathBuf::from(log_path))
    });

    let interrupt_flag = Arc::new(AtomicBool::new(false));
    if let Err(err) = ctrlc::set_handler({
        let interrupt_flag = Arc::clone(&interrupt_flag);
        move || {
            interrupt_flag.store(true, Ordering::SeqCst);
        }
    }) {
        eprintln!("Failed to set interrupt handler: {}", err);
    }

    let mut state = RuntimeState {
        config: loaded.config,
        config_path,
        prompt_trudge: prompt_trudge_content,
        prompt_review: prompt_review_content,
        logger,
        tmux: TmuxState::new(),
        interrupt_flag,
        manual_tasks,
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    if env::var("TRUDGER_TEST_FORCE_ERR").is_ok() {
        return Err(quit(&state.logger, "error", 1));
    }

    let result = run_loop(&mut state);
    reset_task_on_exit(&state, &result);
    state.tmux.restore();
    result
}

fn main() -> ExitCode {
    let result = run();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(quit) => quit.exit_code(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Commands, Hooks};
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn sanitize_log_value_replaces_controls() {
        let value = "line\ncarriage\rtab\t";
        assert_eq!(sanitize_log_value(value), "line\\ncarriage\\rtab\\t");
    }

    #[test]
    fn render_prompt_strips_frontmatter() {
        let mut file = NamedTempFile::new().expect("temp file");
        writeln!(
            file,
            "---\nname: test\n---\nHello\nWorld"
        )
        .expect("write");
        let rendered = render_prompt(file.path()).expect("render");
        assert_eq!(rendered, "Hello\nWorld");
    }

    #[test]
    fn require_file_reports_missing() {
        let temp = TempDir::new().expect("temp dir");
        let missing = temp.path().join("missing.txt");
        let err = require_file(&missing, "prompt file").expect_err("should fail");
        assert!(
            err.contains("Missing prompt file"),
            "error should mention missing prompt file, got: {err}"
        );
    }

    #[test]
    fn run_loop_executes_commands_and_hooks_with_env() {
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let next_task_log = temp.path().join("next-task.log");
        let task_show_log = temp.path().join("task-show.log");
        let task_status_log = temp.path().join("task-status.log");
        let task_update_log = temp.path().join("task-update.log");
        let hook_log = temp.path().join("hook.log");
        let codex_log = temp.path().join("codex.log");

        let next_task_queue = temp.path().join("next-task-queue.txt");
        fs::write(&next_task_queue, "tr-1\ntr-2\n\n").expect("write next task queue");
        let status_queue = temp.path().join("status-queue.txt");
        fs::write(&status_queue, "ready\nclosed\nready\nopen\n").expect("write status queue");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var(
            "PATH",
            format!("{}:{}", fixtures_bin.display(), old_path),
        );
        env::set_var("NEXT_TASK_LOG", &next_task_log);
        env::set_var("TASK_SHOW_LOG", &task_show_log);
        env::set_var("TASK_STATUS_LOG", &task_status_log);
        env::set_var("TASK_UPDATE_LOG", &task_update_log);
        env::set_var("HOOK_MOCK_LOG", &hook_log);
        env::set_var("CODEX_MOCK_LOG", &codex_log);
        env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
        env::set_var("TASK_STATUS_QUEUE", &status_queue);
        env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");

        let config = Config {
            agent_command: "codex --yolo exec --default".to_string(),
            agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
            commands: Commands {
                next_task: Some("next-task\t--with-tab".to_string()),
                task_show: "task-show \"$@\"".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update \"$@\"".to_string(),
                reset_task: "reset-task \"$@\"".to_string(),
            },
            hooks: Hooks {
                on_completed: "hook --done".to_string(),
                on_requires_human: "hook --human".to_string(),
            },
            review_loop_limit: 2,
            log_path: log_path.display().to_string(),
        };

        let mut state = RuntimeState {
            config,
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "Task context".to_string(),
            prompt_review: "Review context".to_string(),
            logger: Logger::new(Some(log_path.clone())),
            tmux: TmuxState {
                enabled: false,
                base_name: String::new(),
                original_title: String::new(),
            },
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            manual_tasks: Vec::new(),
            completed_tasks: Vec::new(),
            needs_human_tasks: Vec::new(),
            current_task_id: None,
            current_task_show: None,
            current_task_status: None,
        };

        let result = run_loop(&mut state).expect_err("should exit after queue drained");
        assert_eq!(result.code, 0, "expected graceful exit");

        let codex_contents = fs::read_to_string(&codex_log).expect("read codex log");
        assert!(
            codex_contents.contains("envset TRUDGER_PROMPT=1"),
            "codex should see TRUDGER_PROMPT set"
        );
        assert!(
            codex_contents.contains("envset TRUDGER_REVIEW_PROMPT=1"),
            "codex should see TRUDGER_REVIEW_PROMPT set"
        );
        assert!(
            codex_contents.contains("env TRUDGER_TASK_SHOW=SHOW_PAYLOAD"),
            "codex should receive task show output"
        );
        assert!(
            codex_contents.contains("resume --last"),
            "codex review should include resume --last"
        );

        let next_task_contents = fs::read_to_string(&next_task_log).expect("read next task log");
        assert!(
            next_task_contents.contains("envset TRUDGER_PROMPT=0"),
            "next-task should not get TRUDGER_PROMPT"
        );
        assert!(
            next_task_contents.contains("envset TRUDGER_TASK_ID=0"),
            "next-task should not get TRUDGER_TASK_ID"
        );

        let task_show_contents = fs::read_to_string(&task_show_log).expect("read task-show log");
        assert!(
            task_show_contents.contains("task-show args_count=1 args=--json"),
            "task-show should receive --json"
        );

        let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
        assert!(
            hook_contents.contains("hook args_count=1 args=--done"),
            "hook should not receive task id as arg"
        );
        assert!(
            hook_contents.contains("env TRUDGER_TASK_ID=tr-1"),
            "hook should see task id in env"
        );

        let log_contents = fs::read_to_string(&log_path).expect("read log file");
        let escaped = log_contents.contains("\\t");
        let raw_tab = log_contents.contains('\t');
        let snippet = log_contents
            .lines()
            .find(|line| line.contains("command=next-task"))
            .unwrap_or("");
        assert!(
            escaped,
            "log should escape tab in command (escaped={escaped}, raw_tab={raw_tab}, bytes={:?}), got:\n{log_contents}",
            snippet.as_bytes()
        );
        assert!(
            !raw_tab,
            "log should not include raw tab characters, got:\n{log_contents}"
        );
    }

    #[test]
    fn reset_task_runs_on_exit_with_active_task() {
        let temp = TempDir::new().expect("temp dir");
        let reset_task_log = temp.path().join("reset-task.log");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var(
            "PATH",
            format!("{}:{}", fixtures_bin.display(), old_path),
        );
        env::set_var("RESET_TASK_LOG", &reset_task_log);

        let config = Config {
            agent_command: "codex --yolo exec --default".to_string(),
            agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
            commands: Commands {
                next_task: Some("next-task".to_string()),
                task_show: "task-show \"$@\"".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update \"$@\"".to_string(),
                reset_task: "reset-task \"$@\"".to_string(),
            },
            hooks: Hooks {
                on_completed: "hook --done".to_string(),
                on_requires_human: "hook --human".to_string(),
            },
            review_loop_limit: 2,
            log_path: temp.path().join("trudger.log").display().to_string(),
        };

        let state = RuntimeState {
            config,
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "Task context".to_string(),
            prompt_review: "Review context".to_string(),
            logger: Logger::new(None),
            tmux: TmuxState {
                enabled: false,
                base_name: String::new(),
                original_title: String::new(),
            },
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            manual_tasks: Vec::new(),
            completed_tasks: Vec::new(),
            needs_human_tasks: Vec::new(),
            current_task_id: Some("tr-1".to_string()),
            current_task_show: None,
            current_task_status: None,
        };

        let result = Err(Quit {
            code: 1,
            reason: "error".to_string(),
        });
        reset_task_on_exit(&state, &result);

        let contents = fs::read_to_string(&reset_task_log).expect("read reset task log");
        assert!(
            contents.contains("reset-task args_count=0 args="),
            "reset-task should run without args"
        );
        assert!(
            contents.contains("env TRUDGER_TASK_ID=tr-1"),
            "reset-task should receive task id in env"
        );
    }
}
