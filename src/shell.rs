use shell_escape::unix::escape;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::logger::{sanitize_log_value, Logger};

pub(crate) fn render_args(args: &[String]) -> String {
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
pub(crate) struct CommandEnv {
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) config_path: String,
    pub(crate) scratch_dir: Option<String>,
    pub(crate) task_id: Option<String>,
    pub(crate) task_show: Option<String>,
    pub(crate) task_status: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) review_prompt: Option<String>,
    pub(crate) completed: Option<String>,
    pub(crate) needs_human: Option<String>,
}

impl CommandEnv {
    pub(crate) fn apply(&self, cmd: &mut Command) {
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
pub(crate) struct CommandResult {
    pub(crate) stdout: String,
    pub(crate) exit_code: i32,
}

#[derive(Clone, Copy, Debug)]
enum ShellCommandStdioMode {
    Capture,
    Inherit,
}

fn run_shell_command_bash_lc(
    command: &str,
    log_label: &str,
    task_token: &str,
    args: &[String],
    env: &CommandEnv,
    logger: &Logger,
    stdio_mode: ShellCommandStdioMode,
) -> Result<(i32, Option<String>), String> {
    if command.is_empty() {
        return Ok((0, None));
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

    match stdio_mode {
        ShellCommandStdioMode::Capture => {}
        ShellCommandStdioMode::Inherit => {
            cmd.stdin(std::process::Stdio::inherit());
            cmd.stdout(std::process::Stdio::inherit());
            cmd.stderr(std::process::Stdio::inherit());
        }
    }

    env.apply(&mut cmd);

    let (exit_code, stdout) = match stdio_mode {
        ShellCommandStdioMode::Capture => {
            let output = cmd
                .output()
                .map_err(|err| format!("Failed to run command '{}': {}", command, err))?;

            let exit_code = output.status.code().unwrap_or(1);
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            (exit_code, Some(stdout))
        }
        ShellCommandStdioMode::Inherit => {
            let status = cmd
                .status()
                .map_err(|err| format!("Failed to run command '{}': {}", command, err))?;

            let exit_code = status.code().unwrap_or(1);
            (exit_code, None)
        }
    };

    logger.log_transition(&format!(
        "cmd exit label={} task={} exit={}",
        log_label, task_token, exit_code
    ));

    Ok((exit_code, stdout))
}

pub(crate) fn run_shell_command_capture(
    command: &str,
    log_label: &str,
    task_token: &str,
    args: &[String],
    env: &CommandEnv,
    logger: &Logger,
) -> Result<CommandResult, String> {
    let (exit_code, stdout) = run_shell_command_bash_lc(
        command,
        log_label,
        task_token,
        args,
        env,
        logger,
        ShellCommandStdioMode::Capture,
    )?;

    Ok(CommandResult {
        stdout: stdout.unwrap_or_default(),
        exit_code,
    })
}

pub(crate) fn run_shell_command_status(
    command: &str,
    log_label: &str,
    task_token: &str,
    args: &[String],
    env: &CommandEnv,
    logger: &Logger,
) -> Result<i32, String> {
    let (exit_code, _stdout) = run_shell_command_bash_lc(
        command,
        log_label,
        task_token,
        args,
        env,
        logger,
        ShellCommandStdioMode::Inherit,
    )?;

    Ok(exit_code)
}

pub(crate) fn command_exists(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|path| {
        let full = path.join(name);
        is_executable_file(&full)
    })
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    // `metadata` follows symlinks; dangling symlinks fail and are treated as missing.
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    if !metadata.is_file() {
        return false;
    }

    (metadata.permissions().mode() & 0o111) != 0
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}
