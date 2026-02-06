use chrono::Utc;
use clap::{Parser, Subcommand};
use regex::Regex;
use shell_escape::unix::escape;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod config;

use crate::config::{load_config, Config};

const PROMPT_TRUDGE: &str = ".codex/prompts/trudge.md";
const PROMPT_REVIEW: &str = ".codex/prompts/trudge_review.md";
const DEFAULT_CONFIG_REL: &str = ".config/trudger.yml";

#[derive(Debug, Parser)]
#[command(name = "trudger", disable_help_subcommand = true)]
struct Cli {
    #[arg(short = 'c', long = "config", global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[arg(
        short = 't',
        long = "task",
        global = true,
        action = clap::ArgAction::Append,
        value_name = "TASK_ID"
    )]
    task: Vec<String>,

    #[arg(value_name = "ARG", hide = true)]
    positional: Vec<String>,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Doctor,
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
    let mut out = String::new();
    let mut in_frontmatter = false;
    let mut first_line = true;

    for line in content.lines() {
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
    let output = Command::new("bash")
        .arg("-lc")
        .arg("printf \"%q \" \"$@\"")
        .arg("--")
        .args(args)
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout).to_string();
        }
    }

    let mut rendered = String::new();
    for arg in args {
        rendered.push_str(escape(arg.into()).as_ref());
        rendered.push(' ');
    }
    rendered
}

#[derive(Debug, Clone)]
struct CommandEnv {
    cwd: Option<PathBuf>,
    config_path: String,
    scratch_dir: Option<String>,
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
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        cmd.env("TRUDGER_CONFIG_PATH", &self.config_path);
        Self::apply_optional(cmd, "TRUDGER_DOCTOR_SCRATCH_DIR", &self.scratch_dir);
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

    fn update_name(
        &self,
        phase: &str,
        task_id: &str,
        completed: &[String],
        needs_human: &[String],
    ) {
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
            Command::new("hostname").output().ok().and_then(|output| {
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
        .unwrap_or_default();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Run,
    Doctor,
}

fn parse_manual_tasks(raw_values: &[String]) -> Result<Vec<String>, String> {
    let mut tasks = Vec::new();
    for raw in raw_values {
        for (index, segment) in raw.split(',').enumerate() {
            let trimmed = segment.trim();
            if trimmed.is_empty() {
                return Err(format!(
                    "Invalid -t/--task value: empty segment in {:?} at index {}.",
                    raw, index
                ));
            }
            tasks.push(trimmed.to_string());
        }
    }
    Ok(tasks)
}

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

fn run_doctor_mode(config: &Config, config_path: &Path, logger: &Logger) -> Result<(), Quit> {
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

fn run_with_cli(cli: Cli) -> Result<(), Quit> {
    let mode = match cli.command {
        Some(CliCommand::Doctor) => AppMode::Doctor,
        None => AppMode::Run,
    };

    let manual_tasks = match parse_manual_tasks(&cli.task) {
        Ok(tasks) => tasks,
        Err(message) => {
            eprintln!("{}", message);
            return Err(Quit {
                code: 1,
                reason: message,
            });
        }
    };
    if mode == AppMode::Doctor && !manual_tasks.is_empty() {
        let message = "-t/--task is not supported in doctor mode.".to_string();
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: message,
        });
    }
    if mode == AppMode::Run && !cli.positional.is_empty() {
        let message = format!(
            "Positional arguments are not supported.\nMigration: pass manual task ids via -t/--task (for example: trudger -t {}).",
            cli.positional.join(" -t ")
        );
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: "positional_args_not_supported".to_string(),
        });
    }

    let config_path = cli.config;
    let config_path_source_flag = config_path.is_some();
    let home = home_dir().map_err(|message| Quit {
        code: 1,
        reason: message,
    })?;

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

    let log_path = loaded.config.log_path.trim();
    let logger = Logger::new(if log_path.is_empty() {
        None
    } else {
        Some(PathBuf::from(log_path))
    });

    if mode == AppMode::Doctor {
        return run_doctor_mode(&loaded.config, &config_path, &logger);
    }

    if let Err(message) = validate_config(&loaded.config, &manual_tasks) {
        eprintln!("{}", message);
        return Err(quit(&logger, &message, 1));
    }

    let prompt_trudge = home.join(PROMPT_TRUDGE);
    let prompt_review = home.join(PROMPT_REVIEW);
    if let Err(message) = require_file(&prompt_trudge, "prompt file") {
        eprintln!("{}", message);
        return Err(quit(&logger, &message, 1));
    }
    if let Err(message) = require_file(&prompt_review, "prompt file") {
        eprintln!("{}", message);
        return Err(quit(&logger, &message, 1));
    }

    let prompt_trudge_content =
        render_prompt(&prompt_trudge).map_err(|message| quit(&logger, &message, 1))?;
    let prompt_review_content =
        render_prompt(&prompt_review).map_err(|message| quit(&logger, &message, 1))?;

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

fn run() -> Result<(), Quit> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            return Err(Quit {
                code: err.exit_code(),
                reason: "cli_parse".to_string(),
            });
        }
    };
    run_with_cli(cli)
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
    use std::sync::Mutex;
    use tempfile::{NamedTempFile, TempDir};

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn reset_test_env() {
        for key in [
            "NEXT_TASK_EXIT_CODE",
            "NEXT_TASK_OUTPUT_QUEUE",
            "NEXT_TASK_OUTPUT",
            "TASK_STATUS_QUEUE",
            "TASK_STATUS_OUTPUT",
            "TASK_SHOW_QUEUE",
            "TASK_SHOW_OUTPUT",
            "TRUDGER_CONFIG_PATH",
            "TRUDGER_DOCTOR_SCRATCH_DIR",
            "TRUDGER_SKIP_NOT_READY_LIMIT",
            "TRUDGER_PROMPT",
            "TRUDGER_REVIEW_PROMPT",
            "TRUDGER_TASK_ID",
            "TRUDGER_TASK_SHOW",
            "TRUDGER_TASK_STATUS",
            "CODEX_MOCK_LOG",
            "TASK_SHOW_LOG",
            "TASK_STATUS_LOG",
            "TASK_UPDATE_LOG",
            "HOOK_MOCK_LOG",
            "RESET_TASK_LOG",
            "NEXT_TASK_LOG",
            "TRUDGER_TEST_FORCE_ERR",
        ] {
            env::remove_var(key);
        }
    }

    #[test]
    fn sanitize_log_value_replaces_controls() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let value = "line\ncarriage\rtab\t";
        assert_eq!(sanitize_log_value(value), "line\\ncarriage\\rtab\\t");
    }

    #[test]
    fn log_line_matches_shell_fixture() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let logger = Logger::new(Some(log_path.clone()));

        let command = "next-task\t--with-tab";
        let args = vec![
            "tr-1".to_string(),
            "with space".to_string(),
            "tab\targ".to_string(),
            "#start".to_string(),
            "~home".to_string(),
            "foo$bar".to_string(),
            "foo\"bar".to_string(),
            "foo\\bar".to_string(),
        ];
        let args_render = render_args(&args);
        logger.log_transition(&format!(
            "cmd start label=next-task task=tr-1 mode=bash_lc command={} args={}",
            sanitize_log_value(command),
            sanitize_log_value(&args_render)
        ));

        let log_contents = fs::read_to_string(&log_path).expect("read log file");
        let line = log_contents.lines().next().expect("log line");
        let message = line.split_once(' ').map(|x| x.1).unwrap_or("");
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("logs")
            .join("command-start.txt");
        let expected = fs::read_to_string(&fixture_path).expect("read fixture");
        let expected = expected.trim_end_matches('\n').trim_end_matches('\r');
        assert_eq!(message, expected);
    }

    #[test]
    fn render_prompt_strips_frontmatter() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let mut file = NamedTempFile::new().expect("temp file");
        writeln!(file, "---\nname: test\n---\nHello\nWorld").expect("write");
        let rendered = render_prompt(file.path()).expect("render");
        assert_eq!(rendered, "Hello\nWorld");
    }

    #[test]
    fn require_file_reports_missing() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
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
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
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
        fs::write(&status_queue, "ready\nclosed\nready\nblocked\n").expect("write status queue");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
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
                on_doctor_setup: None,
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
            task_show_contents.contains("task-show args_count=0 args="),
            "task-show should receive no args"
        );

        let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
        assert!(
            hook_contents.contains("hook args_count=1 args=--done"),
            "hook should not receive positional task args"
        );
        assert!(
            hook_contents.contains("hook args_count=1 args=--human"),
            "requires-human hook should not receive positional task args"
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
    fn review_loop_limit_retries_until_closed() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let task_update_log = temp.path().join("task-update.log");
        let hook_log = temp.path().join("hook.log");

        let next_task_queue = temp.path().join("next-task-queue.txt");
        fs::write(&next_task_queue, "tr-1\n\n").expect("write next task queue");
        let status_queue = temp.path().join("status-queue.txt");
        fs::write(&status_queue, "ready\nopen\nclosed\n").expect("write status queue");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
        env::set_var("TASK_STATUS_QUEUE", &status_queue);
        env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");
        env::set_var("TASK_UPDATE_LOG", &task_update_log);
        env::set_var("HOOK_MOCK_LOG", &hook_log);

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
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: log_path.display().to_string(),
        };

        let mut state = RuntimeState {
            config,
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "Task context".to_string(),
            prompt_review: "Review context".to_string(),
            logger: Logger::new(Some(log_path)),
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

        let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
        assert!(
            hook_contents.contains("hook args_count=1 args=--done"),
            "expected completed hook"
        );
        assert!(
            !hook_contents.contains("--human"),
            "requires-human hook should not run when closed within limit, got:\n{hook_contents}"
        );

        let update_contents = fs::read_to_string(&task_update_log).expect("read task-update log");
        assert_eq!(
            update_contents.matches("args=--status in_progress").count(),
            2,
            "expected task_update_in_progress to run once per solve loop, got:\n{update_contents}"
        );
        assert!(
            !update_contents.contains("args=--status blocked"),
            "should not block task when closed within limit, got:\n{update_contents}"
        );
    }

    #[test]
    fn review_loop_limit_exhaustion_marks_blocked_and_requires_human() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let task_update_log = temp.path().join("task-update.log");
        let hook_log = temp.path().join("hook.log");

        let next_task_queue = temp.path().join("next-task-queue.txt");
        fs::write(&next_task_queue, "tr-1\n\n").expect("write next task queue");
        let status_queue = temp.path().join("status-queue.txt");
        fs::write(&status_queue, "ready\nopen\nopen\n").expect("write status queue");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
        env::set_var("TASK_STATUS_QUEUE", &status_queue);
        env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");
        env::set_var("TASK_UPDATE_LOG", &task_update_log);
        env::set_var("HOOK_MOCK_LOG", &hook_log);

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
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: log_path.display().to_string(),
        };

        let mut state = RuntimeState {
            config,
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "Task context".to_string(),
            prompt_review: "Review context".to_string(),
            logger: Logger::new(Some(log_path)),
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
        assert_eq!(state.needs_human_tasks, vec!["tr-1"]);
        assert!(state.completed_tasks.is_empty());

        let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
        assert!(
            hook_contents.contains("hook args_count=1 args=--human"),
            "expected requires-human hook after exhaustion, got:\n{hook_contents}"
        );
        assert!(
            hook_contents.contains("env TRUDGER_TASK_STATUS=blocked"),
            "expected hook to see blocked status after exhaustion, got:\n{hook_contents}"
        );
        assert!(
            !hook_contents.contains("--done"),
            "completed hook should not run after exhaustion, got:\n{hook_contents}"
        );

        let update_contents = fs::read_to_string(&task_update_log).expect("read task-update log");
        assert_eq!(
            update_contents.matches("args=--status in_progress").count(),
            2,
            "expected task_update_in_progress to run once per solve loop, got:\n{update_contents}"
        );
        assert!(
            update_contents.contains("args=--status blocked"),
            "expected task to be marked blocked after exhaustion, got:\n{update_contents}"
        );
    }

    #[test]
    fn next_task_exit_1_exits_zero_without_running_commands() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let task_show_log = temp.path().join("task-show.log");
        let codex_log = temp.path().join("codex.log");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        env::set_var("NEXT_TASK_EXIT_CODE", "1");
        env::set_var("TASK_SHOW_LOG", &task_show_log);
        env::set_var("CODEX_MOCK_LOG", &codex_log);

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
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: log_path.display().to_string(),
        };

        let mut state = RuntimeState {
            config,
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "Task context".to_string(),
            prompt_review: "Review context".to_string(),
            logger: Logger::new(Some(log_path)),
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

        let result = run_loop(&mut state).expect_err("should exit when next-task returns exit 1");
        assert_eq!(result.code, 0, "expected graceful exit");
        assert!(
            !codex_log.exists(),
            "codex should not run when next_task exits 1"
        );
        assert!(
            !task_show_log.exists(),
            "task_show should not run when next_task exits 1"
        );
    }

    #[test]
    fn hook_uses_env_task_id_in_shell() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let next_task_log = temp.path().join("next-task.log");
        let task_show_log = temp.path().join("task-show.log");
        let task_status_log = temp.path().join("task-status.log");
        let task_update_log = temp.path().join("task-update.log");
        let hook_log = temp.path().join("hook.log");
        let codex_log = temp.path().join("codex.log");

        let next_task_queue = temp.path().join("next-task-queue.txt");
        fs::write(&next_task_queue, "tr-55\n\n").expect("write next task queue");
        let status_queue = temp.path().join("status-queue.txt");
        fs::write(&status_queue, "ready\nclosed\n").expect("write status queue");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
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
                on_completed: "hook --done \"$TRUDGER_TASK_ID\"".to_string(),
                on_requires_human: "hook --human \"$TRUDGER_TASK_ID\"".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: log_path.display().to_string(),
        };

        let mut state = RuntimeState {
            config,
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "Task context".to_string(),
            prompt_review: "Review context".to_string(),
            logger: Logger::new(Some(log_path)),
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

        let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
        assert!(
            hook_contents.contains("hook args_count=2 args=--done tr-55"),
            "hook should receive task id via TRUDGER_TASK_ID"
        );
    }

    #[test]
    fn skip_not_ready_respects_limit() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let task_show_log = temp.path().join("task-show.log");
        let codex_log = temp.path().join("codex.log");

        let next_task_queue = temp.path().join("next-task-queue.txt");
        fs::write(&next_task_queue, "tr-1\ntr-2\n").expect("write next task queue");
        let status_queue = temp.path().join("status-queue.txt");
        fs::write(&status_queue, "blocked\nstalled\n").expect("write status queue");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
        env::set_var("TASK_STATUS_QUEUE", &status_queue);
        env::set_var("TASK_SHOW_LOG", &task_show_log);
        env::set_var("CODEX_MOCK_LOG", &codex_log);
        env::set_var("TRUDGER_SKIP_NOT_READY_LIMIT", "2");

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
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: log_path.display().to_string(),
        };

        let mut state = RuntimeState {
            config,
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "Task context".to_string(),
            prompt_review: "Review context".to_string(),
            logger: Logger::new(Some(log_path)),
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

        let result = run_loop(&mut state).expect_err("should exit when no ready task found");
        assert_eq!(result.code, 0, "expected idle exit code");
        assert_eq!(result.reason, "no_ready_task");
        assert!(
            !codex_log.exists(),
            "codex should not run when tasks are not ready"
        );
        assert!(
            !task_show_log.exists(),
            "task_show should not run when tasks are not ready"
        );
    }

    #[test]
    fn missing_status_after_review_errors() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let codex_log = temp.path().join("codex.log");

        let next_task_queue = temp.path().join("next-task-queue.txt");
        fs::write(&next_task_queue, "tr-1\n").expect("write next task queue");
        let status_queue = temp.path().join("status-queue.txt");
        fs::write(&status_queue, "ready\n\n").expect("write status queue");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
        env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
        env::set_var("TASK_STATUS_QUEUE", &status_queue);
        env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");
        env::set_var("CODEX_MOCK_LOG", &codex_log);

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
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: log_path.display().to_string(),
        };

        let mut state = RuntimeState {
            config,
            config_path: temp.path().join("trudger.yml"),
            prompt_trudge: "Task context".to_string(),
            prompt_review: "Review context".to_string(),
            logger: Logger::new(Some(log_path)),
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

        let result = run_loop(&mut state).expect_err("should error on missing status");
        assert_eq!(result.code, 1);
        assert!(
            result.reason.contains("task_missing_status_after_review"),
            "expected missing status reason, got: {}",
            result.reason
        );
    }

    #[test]
    fn reset_task_runs_on_exit_with_active_task() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();
        let temp = TempDir::new().expect("temp dir");
        let reset_task_log = temp.path().join("reset-task.log");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
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
                on_doctor_setup: None,
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

    #[test]
    fn parse_manual_tasks_trims_and_rejects_empty_segments() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();

        let tasks = parse_manual_tasks(&[" tr-1, tr-2 ".to_string(), "tr-3".to_string()])
            .expect("parse tasks");
        assert_eq!(tasks, vec!["tr-1", "tr-2", "tr-3"]);

        let err = parse_manual_tasks(&["tr-1,,tr-2".to_string()]).expect_err("should error");
        assert!(
            err.contains("empty segment"),
            "expected empty segment error, got: {err}"
        );
    }

    #[test]
    fn clap_parses_doctor_subcommand_and_positional_args() {
        let cli = Cli::try_parse_from(["trudger", "doctor"]).expect("parse doctor");
        assert!(
            matches!(cli.command, Some(CliCommand::Doctor)),
            "expected doctor subcommand"
        );
        assert!(
            cli.positional.is_empty(),
            "doctor should have no positionals"
        );

        let cli = Cli::try_parse_from(["trudger", "doctor", "-t", "tr-1"]).expect("parse doctor");
        assert!(
            matches!(cli.command, Some(CliCommand::Doctor)),
            "expected doctor subcommand"
        );
        assert_eq!(cli.task, vec!["tr-1"], "expected task flag capture");

        let cli = Cli::try_parse_from(["trudger", "tr-1"]).expect("parse positional");
        assert!(
            cli.command.is_none(),
            "positional should not be a subcommand"
        );
        assert_eq!(cli.positional, vec!["tr-1"]);
    }

    #[test]
    fn doctor_rejects_task_flag_with_clear_error() {
        let err = run_with_cli(Cli {
            config: None,
            task: vec!["tr-1".to_string()],
            positional: Vec::new(),
            command: Some(CliCommand::Doctor),
        })
        .expect_err("expected doctor task-flag rejection");
        assert_eq!(err.code, 1);
        assert!(
            err.reason.contains("-t/--task"),
            "expected -t/--task error, got: {}",
            err.reason
        );
    }

    #[test]
    fn positional_task_ids_are_rejected_with_migration_hint() {
        let err = run_with_cli(Cli {
            config: None,
            task: Vec::new(),
            positional: vec!["tr-1".to_string()],
            command: None,
        })
        .expect_err("expected positional task id rejection");
        assert_eq!(err.code, 1);
        assert_eq!(err.reason, "positional_args_not_supported");
    }

    #[test]
    fn doctor_does_not_require_prompts_and_cleans_scratch_dir() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();

        let old_home = env::var_os("HOME");
        let original_cwd = env::current_dir().expect("cwd");
        let temp = TempDir::new().expect("temp dir");
        let invocation = temp.path().join("invocation");
        fs::create_dir_all(&invocation).expect("create invocation dir");
        env::set_current_dir(&invocation).expect("set cwd");

        let hook_log = temp.path().join("hook.log");
        let next_task_log = temp.path().join("next-task.log");
        let task_show_log = temp.path().join("task-show.log");
        let task_status_log = temp.path().join("task-status.log");
        let task_update_log = temp.path().join("task-update.log");
        let reset_task_log = temp.path().join("reset-task.log");
        env::set_var("HOOK_MOCK_LOG", &hook_log);
        env::set_var("NEXT_TASK_LOG", &next_task_log);
        env::set_var("TASK_SHOW_LOG", &task_show_log);
        env::set_var("TASK_STATUS_LOG", &task_status_log);
        env::set_var("TASK_UPDATE_LOG", &task_update_log);
        env::set_var("RESET_TASK_LOG", &reset_task_log);

        // Ensure the setup hook sees these as unset even if present in the parent process env.
        env::set_var("TRUDGER_TASK_ID", "PARENT_TASK");
        env::set_var("TRUDGER_TASK_SHOW", "PARENT_SHOW");
        env::set_var("TRUDGER_TASK_STATUS", "PARENT_STATUS");
        env::set_var("TRUDGER_PROMPT", "PARENT_PROMPT");
        env::set_var("TRUDGER_REVIEW_PROMPT", "PARENT_REVIEW_PROMPT");

        let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("bin");
        let old_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

        let beads_dir = invocation.join(".beads");
        fs::create_dir_all(&beads_dir).expect("create .beads dir");
        fs::write(
            beads_dir.join("issues.jsonl"),
            r#"{"id":"tr-open","status":"open"}
{"id":"tr-closed","status":"closed"}
"#,
        )
        .expect("write issues.jsonl");

        env::set_var("NEXT_TASK_OUTPUT", "tr-open");
        env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");
        let status_queue = temp.path().join("status-queue.txt");
        fs::write(&status_queue, "open\nin_progress\nopen\nclosed\n").expect("write status queue");
        env::set_var("TASK_STATUS_QUEUE", &status_queue);

        let config_path = temp.path().join("trudger.yml");
        fs::write(
            &config_path,
            r#"
agent_command: "agent"
agent_review_command: "agent-review"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "task-update \"$@\""
  reset_task: "reset-task"
review_loop_limit: 2
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --human"
  on_doctor_setup: 'hook --doctor-setup; rm -rf "$TRUDGER_DOCTOR_SCRATCH_DIR/.beads"; cp -R ".beads" "$TRUDGER_DOCTOR_SCRATCH_DIR/"'
"#,
        )
        .expect("write config");

        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create home");
        env::set_var("HOME", &home);

        let cli = Cli {
            config: Some(config_path.clone()),
            task: Vec::new(),
            positional: Vec::new(),
            command: Some(CliCommand::Doctor),
        };

        run_with_cli(cli).expect("doctor should succeed without prompts");

        let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
        assert!(
            hook_contents.contains(&format!("cwd {}", invocation.display())),
            "setup hook should run from invocation cwd, got:\n{hook_contents}"
        );
        assert!(
            hook_contents.contains("envset TRUDGER_CONFIG_PATH=1"),
            "setup hook should receive TRUDGER_CONFIG_PATH"
        );
        assert!(
            hook_contents.contains("envset TRUDGER_DOCTOR_SCRATCH_DIR=1"),
            "setup hook should receive TRUDGER_DOCTOR_SCRATCH_DIR"
        );
        assert!(
            hook_contents.contains("envset TRUDGER_TASK_ID=0"),
            "setup hook should not receive TRUDGER_TASK_ID"
        );
        assert!(
            hook_contents.contains("envset TRUDGER_TASK_SHOW=0"),
            "setup hook should not receive TRUDGER_TASK_SHOW"
        );
        assert!(
            hook_contents.contains("envset TRUDGER_TASK_STATUS=0"),
            "setup hook should not receive TRUDGER_TASK_STATUS"
        );
        assert!(
            hook_contents.contains("envset TRUDGER_PROMPT=0"),
            "setup hook should not receive TRUDGER_PROMPT"
        );
        assert!(
            hook_contents.contains("envset TRUDGER_REVIEW_PROMPT=0"),
            "setup hook should not receive TRUDGER_REVIEW_PROMPT"
        );

        let scratch_dir = hook_contents
            .lines()
            .find_map(|line| line.strip_prefix("env TRUDGER_DOCTOR_SCRATCH_DIR="))
            .unwrap_or("")
            .trim()
            .to_string();
        assert!(!scratch_dir.is_empty(), "missing scratch dir in hook log");

        let next_task_contents = fs::read_to_string(&next_task_log).expect("read next-task log");
        assert!(
            next_task_contents.contains(&format!("cwd {}", scratch_dir)),
            "next-task should run from scratch dir, got:\n{next_task_contents}"
        );
        assert!(
            next_task_contents.contains("envset TRUDGER_DOCTOR_SCRATCH_DIR=1"),
            "next-task should receive TRUDGER_DOCTOR_SCRATCH_DIR"
        );
        assert!(
            next_task_contents.contains("envset TRUDGER_TASK_ID=0"),
            "next-task should not receive TRUDGER_TASK_ID, got:\n{next_task_contents}"
        );

        let task_show_contents = fs::read_to_string(&task_show_log).expect("read task-show log");
        assert!(
            task_show_contents.contains(&format!("cwd {}", scratch_dir)),
            "task-show should run from scratch dir, got:\n{task_show_contents}"
        );
        assert!(
            task_show_contents.contains("env TRUDGER_TASK_ID=tr-open"),
            "task-show should receive TRUDGER_TASK_ID=tr-open, got:\n{task_show_contents}"
        );
        assert!(
            task_show_contents.contains("envset TRUDGER_PROMPT=0"),
            "task-show should not receive TRUDGER_PROMPT, got:\n{task_show_contents}"
        );

        let task_status_contents =
            fs::read_to_string(&task_status_log).expect("read task-status log");
        assert!(
            task_status_contents.contains(&format!("cwd {}", scratch_dir)),
            "task-status should run from scratch dir, got:\n{task_status_contents}"
        );
        assert!(
            task_status_contents.contains("env TRUDGER_TASK_ID=tr-open"),
            "task-status should run for tr-open, got:\n{task_status_contents}"
        );
        assert!(
            task_status_contents.contains("env TRUDGER_TASK_ID=tr-closed"),
            "task-status should run for tr-closed (closed parsing), got:\n{task_status_contents}"
        );

        let task_update_contents =
            fs::read_to_string(&task_update_log).expect("read task-update log");
        assert!(
            task_update_contents.contains(&format!("cwd {}", scratch_dir)),
            "task-update should run from scratch dir, got:\n{task_update_contents}"
        );
        assert!(
            task_update_contents.contains("args_count=2 args=--status in_progress"),
            "task-update should set in_progress, got:\n{task_update_contents}"
        );

        let reset_task_contents = fs::read_to_string(&reset_task_log).expect("read reset-task log");
        assert!(
            reset_task_contents.contains(&format!("cwd {}", scratch_dir)),
            "reset-task should run from scratch dir, got:\n{reset_task_contents}"
        );
        assert_eq!(
            reset_task_contents
                .matches("reset-task args_count=0 args=")
                .count(),
            2,
            "expected reset-task to run twice, got:\n{reset_task_contents}"
        );

        assert!(
            hook_contents.contains(&format!("cwd {}", scratch_dir)),
            "hooks should run from scratch dir after setup, got:\n{hook_contents}"
        );
        assert!(
            hook_contents.contains("hook args_count=1 args=--done"),
            "doctor should execute hooks.on_completed, got:\n{hook_contents}"
        );
        assert!(
            hook_contents.contains("hook args_count=1 args=--human"),
            "doctor should execute hooks.on_requires_human, got:\n{hook_contents}"
        );
        assert!(
            !Path::new(&scratch_dir).exists(),
            "scratch dir should be cleaned up: {scratch_dir}"
        );

        env::set_current_dir(&original_cwd).expect("restore cwd");
        match old_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        };
    }

    #[test]
    fn doctor_cleanup_failure_is_an_error() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();

        let original_cwd = env::current_dir().expect("cwd");
        let temp = TempDir::new().expect("temp dir");
        let invocation = temp.path().join("invocation");
        fs::create_dir_all(&invocation).expect("create invocation dir");
        env::set_current_dir(&invocation).expect("set cwd");

        let scratch_path_file = temp.path().join("scratch-path.txt");
        let hook = format!(
            "printf '%s' \"$TRUDGER_DOCTOR_SCRATCH_DIR\" > \"{}\"; \
             mkdir -p \"$TRUDGER_DOCTOR_SCRATCH_DIR/locked\"; \
             printf 'hi' > \"$TRUDGER_DOCTOR_SCRATCH_DIR/locked/file\"; \
             chmod 555 \"$TRUDGER_DOCTOR_SCRATCH_DIR/locked\"",
            scratch_path_file.display()
        );

        let config = Config {
            agent_command: "agent".to_string(),
            agent_review_command: "agent-review".to_string(),
            commands: Commands {
                next_task: Some("next-task".to_string()),
                task_show: "task-show".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: Some(hook),
            },
            review_loop_limit: 2,
            log_path: temp.path().join("trudger.log").display().to_string(),
        };
        let logger = Logger::new(None);

        let result = run_doctor_mode(&config, &temp.path().join("trudger.yml"), &logger)
            .expect_err("expected cleanup failure");
        assert_eq!(result.code, 1);
        assert!(
            result.reason.contains("doctor scratch cleanup failed"),
            "expected cleanup failure reason, got: {}",
            result.reason
        );

        let scratch_dir = fs::read_to_string(&scratch_path_file)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !scratch_dir.is_empty() {
            let locked_dir = Path::new(&scratch_dir).join("locked");
            let _ = fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o755));
            let _ = fs::remove_dir_all(&scratch_dir);
        }

        env::set_current_dir(&original_cwd).expect("restore cwd");
    }

    #[test]
    fn sample_configs_load_and_validate() {
        let _guard = ENV_MUTEX.lock().unwrap();
        reset_test_env();

        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in ["trudgeable-with-hooks", "robot-triage"] {
            let path = root
                .join("sample_configuration")
                .join(format!("{}.yml", name));
            let loaded = load_config(&path).expect("load sample config");
            validate_config(&loaded.config, &[]).expect("validate sample config");
            let hook = loaded
                .config
                .hooks
                .on_doctor_setup
                .as_deref()
                .unwrap_or("")
                .trim();
            assert!(
                !hook.is_empty(),
                "sample config {} should include hooks.on_doctor_setup",
                name
            );
        }
    }
}
