use shell_escape::unix::escape;
use std::borrow::Cow;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::logger::{sanitize_log_value, Logger};

// Guardrail against `execve`/`spawn` failures (E2BIG) from oversized env values.
// Keep this conservative; prompt/show payloads are expected to be small.
pub(crate) const TRUDGER_ENV_VALUE_MAX_BYTES: usize = 64 * 1024;
// Total budget for all `TRUDGER_*` environment variables we set for a subprocess.
// This intentionally ignores the inherited environment size; the goal is to cap our contribution.
const TRUDGER_ENV_TOTAL_MAX_BYTES: usize = 128 * 1024;

pub(crate) fn truncate_utf8_to_bytes(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }

    let mut cut = max_bytes.min(value.len());
    while cut > 0 && !value.is_char_boundary(cut) {
        cut -= 1;
    }

    &value[..cut]
}

pub(crate) fn render_args(args: &[String]) -> String {
    if args.is_empty() {
        return String::new();
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
    pub(crate) target_status: Option<String>,
    pub(crate) agent_prompt: Option<String>,
    pub(crate) agent_phase: Option<String>,
    pub(crate) agent_profile: Option<String>,
    pub(crate) agent_invocation_id: Option<String>,
    pub(crate) completed: Option<String>,
    pub(crate) needs_human: Option<String>,
    pub(crate) notify_event: Option<String>,
    pub(crate) notify_duration_ms: Option<String>,
    pub(crate) notify_folder: Option<String>,
    pub(crate) notify_exit_code: Option<String>,
    pub(crate) notify_task_id: Option<String>,
    pub(crate) notify_task_description: Option<String>,
    pub(crate) notify_message: Option<String>,
    pub(crate) notify_payload_path: Option<String>,
}

impl CommandEnv {
    pub(crate) fn apply(
        &self,
        cmd: &mut Command,
        logger: &Logger,
        log_label: &str,
        task_token: &str,
    ) {
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }

        // Remove legacy prompt keys to prevent inherited values from leaking into children.
        cmd.env_remove("TRUDGER_PROMPT");
        cmd.env_remove("TRUDGER_REVIEW_PROMPT");

        let mut task_show_max = TRUDGER_ENV_VALUE_MAX_BYTES;
        let mut agent_prompt_max = TRUDGER_ENV_VALUE_MAX_BYTES;

        let total = Self::estimate_trudger_payload_bytes(
            task_show_max,
            agent_prompt_max,
            &self.config_path,
            self.scratch_dir.as_deref(),
            self.task_id.as_deref(),
            self.task_show.as_deref(),
            self.task_status.as_deref(),
            self.target_status.as_deref(),
            self.agent_prompt.as_deref(),
            self.agent_phase.as_deref(),
            self.agent_profile.as_deref(),
            self.agent_invocation_id.as_deref(),
            self.completed.as_deref(),
            self.needs_human.as_deref(),
            self.notify_event.as_deref(),
            self.notify_duration_ms.as_deref(),
            self.notify_folder.as_deref(),
            self.notify_exit_code.as_deref(),
            self.notify_task_id.as_deref(),
            self.notify_task_description.as_deref(),
            self.notify_message.as_deref(),
            self.notify_payload_path.as_deref(),
        );

        if total > TRUDGER_ENV_TOTAL_MAX_BYTES {
            // Reduce the largest/least-critical payloads first. The goal is to avoid spawn failures
            // while keeping the rest of the contract intact (vars stay set, but may be empty).
            let over = total - TRUDGER_ENV_TOTAL_MAX_BYTES;
            let over = Self::reduce_overage(&mut task_show_max, self.task_show.as_deref(), over);
            let _ = Self::reduce_overage(&mut agent_prompt_max, self.agent_prompt.as_deref(), over);

            let new_total = Self::estimate_trudger_payload_bytes(
                task_show_max,
                agent_prompt_max,
                &self.config_path,
                self.scratch_dir.as_deref(),
                self.task_id.as_deref(),
                self.task_show.as_deref(),
                self.task_status.as_deref(),
                self.target_status.as_deref(),
                self.agent_prompt.as_deref(),
                self.agent_phase.as_deref(),
                self.agent_profile.as_deref(),
                self.agent_invocation_id.as_deref(),
                self.completed.as_deref(),
                self.needs_human.as_deref(),
                self.notify_event.as_deref(),
                self.notify_duration_ms.as_deref(),
                self.notify_folder.as_deref(),
                self.notify_exit_code.as_deref(),
                self.notify_task_id.as_deref(),
                self.notify_task_description.as_deref(),
                self.notify_message.as_deref(),
                self.notify_payload_path.as_deref(),
            );

            if new_total < total {
                // Avoid `eprintln!` so tests can reliably capture stderr via fd redirection.
                let mut stderr = std::io::stderr().lock();
                let _ = writeln!(
                    stderr,
                    "Warning: TRUDGER_* env payload is {} bytes; truncating to {} bytes for command execution.",
                    total, new_total
                );
                logger.log_transition(&format!(
                    "env_truncate_total label={} task={} original_bytes={} truncated_bytes={}",
                    log_label, task_token, total, new_total
                ));
            }
        }

        Self::apply_value_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_CONFIG_PATH",
            &self.config_path,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_DOCTOR_SCRATCH_DIR",
            self.scratch_dir.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_TASK_ID",
            self.task_id.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_TASK_SHOW",
            self.task_show.as_deref(),
            task_show_max,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_TASK_STATUS",
            self.task_status.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_TARGET_STATUS",
            self.target_status.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_AGENT_PROMPT",
            self.agent_prompt.as_deref(),
            agent_prompt_max,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_AGENT_PHASE",
            self.agent_phase.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_PROFILE",
            self.agent_profile.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_INVOCATION_ID",
            self.agent_invocation_id.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_COMPLETED",
            self.completed.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NEEDS_HUMAN",
            self.needs_human.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NOTIFY_EVENT",
            self.notify_event.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NOTIFY_DURATION_MS",
            self.notify_duration_ms.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NOTIFY_FOLDER",
            self.notify_folder.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NOTIFY_EXIT_CODE",
            self.notify_exit_code.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NOTIFY_TASK_ID",
            self.notify_task_id.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NOTIFY_TASK_DESCRIPTION",
            self.notify_task_description.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NOTIFY_MESSAGE",
            self.notify_message.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        Self::apply_optional_with_max(
            cmd,
            logger,
            log_label,
            task_token,
            "TRUDGER_NOTIFY_PAYLOAD_PATH",
            self.notify_payload_path.as_deref(),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
    }

    fn maybe_truncate_utf8(value: &str, max_bytes: usize) -> (Cow<'_, str>, usize, usize) {
        let bytes = value.len();
        if bytes <= max_bytes {
            return (Cow::Borrowed(value), bytes, bytes);
        }

        let mut cut = max_bytes.min(bytes);
        while cut > 0 && !value.is_char_boundary(cut) {
            cut -= 1;
        }

        let truncated = &value[..cut];
        (Cow::Borrowed(truncated), bytes, truncated.len())
    }

    fn env_entry_payload_bytes(key: &str, value: Option<&str>, max_bytes: usize) -> usize {
        let Some(value) = value else {
            return 0;
        };

        let truncated_len = Self::maybe_truncate_utf8(value, max_bytes).2;
        // Approximate execve accounting for "KEY=VALUE\0".
        key.len() + 1 + truncated_len + 1
    }

    #[allow(clippy::too_many_arguments)]
    fn estimate_trudger_payload_bytes(
        task_show_max: usize,
        agent_prompt_max: usize,
        config_path: &str,
        scratch_dir: Option<&str>,
        task_id: Option<&str>,
        task_show: Option<&str>,
        task_status: Option<&str>,
        target_status: Option<&str>,
        agent_prompt: Option<&str>,
        agent_phase: Option<&str>,
        agent_profile: Option<&str>,
        agent_invocation_id: Option<&str>,
        completed: Option<&str>,
        needs_human: Option<&str>,
        notify_event: Option<&str>,
        notify_duration_ms: Option<&str>,
        notify_folder: Option<&str>,
        notify_exit_code: Option<&str>,
        notify_task_id: Option<&str>,
        notify_task_description: Option<&str>,
        notify_message: Option<&str>,
        notify_payload_path: Option<&str>,
    ) -> usize {
        let mut total = 0usize;
        total += Self::env_entry_payload_bytes(
            "TRUDGER_CONFIG_PATH",
            Some(config_path),
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_DOCTOR_SCRATCH_DIR",
            scratch_dir,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total +=
            Self::env_entry_payload_bytes("TRUDGER_TASK_ID", task_id, TRUDGER_ENV_VALUE_MAX_BYTES);
        total += Self::env_entry_payload_bytes("TRUDGER_TASK_SHOW", task_show, task_show_max);
        total += Self::env_entry_payload_bytes(
            "TRUDGER_TASK_STATUS",
            task_status,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_TARGET_STATUS",
            target_status,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total +=
            Self::env_entry_payload_bytes("TRUDGER_AGENT_PROMPT", agent_prompt, agent_prompt_max);
        total += Self::env_entry_payload_bytes(
            "TRUDGER_AGENT_PHASE",
            agent_phase,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_PROFILE",
            agent_profile,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_INVOCATION_ID",
            agent_invocation_id,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_COMPLETED",
            completed,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NEEDS_HUMAN",
            needs_human,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NOTIFY_EVENT",
            notify_event,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NOTIFY_DURATION_MS",
            notify_duration_ms,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NOTIFY_FOLDER",
            notify_folder,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NOTIFY_EXIT_CODE",
            notify_exit_code,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NOTIFY_TASK_ID",
            notify_task_id,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NOTIFY_TASK_DESCRIPTION",
            notify_task_description,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NOTIFY_MESSAGE",
            notify_message,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total += Self::env_entry_payload_bytes(
            "TRUDGER_NOTIFY_PAYLOAD_PATH",
            notify_payload_path,
            TRUDGER_ENV_VALUE_MAX_BYTES,
        );
        total
    }

    fn reduce_overage(max_bytes: &mut usize, value: Option<&str>, over: usize) -> usize {
        if over == 0 {
            return 0;
        }
        let Some(value) = value else {
            return over;
        };

        let current = Self::maybe_truncate_utf8(value, *max_bytes).2;
        if current == 0 {
            return over;
        }

        let target = current.saturating_sub(over);
        *max_bytes = (*max_bytes).min(target);
        let new_len = Self::maybe_truncate_utf8(value, *max_bytes).2;
        let reduced = current.saturating_sub(new_len);
        over.saturating_sub(reduced)
    }

    fn apply_value_with_max(
        cmd: &mut Command,
        logger: &Logger,
        log_label: &str,
        task_token: &str,
        key: &str,
        value: &str,
        max_bytes: usize,
    ) {
        let (rendered, original_bytes, truncated_bytes) =
            Self::maybe_truncate_utf8(value, max_bytes);
        if original_bytes != truncated_bytes {
            // Avoid `eprintln!` so tests can reliably capture stderr via fd redirection.
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(
                stderr,
                "Warning: {} is {} bytes; truncating to {} bytes for command execution.",
                key, original_bytes, truncated_bytes
            );
            logger.log_transition(&format!(
                "env_truncate label={} task={} key={} original_bytes={} truncated_bytes={}",
                log_label, task_token, key, original_bytes, truncated_bytes
            ));
        }
        cmd.env(key, rendered.as_ref());
    }

    fn apply_optional_with_max(
        cmd: &mut Command,
        logger: &Logger,
        log_label: &str,
        task_token: &str,
        key: &str,
        value: Option<&str>,
        max_bytes: usize,
    ) {
        match value {
            Some(value) => Self::apply_value_with_max(
                cmd, logger, log_label, task_token, key, value, max_bytes,
            ),
            None => {
                cmd.env_remove(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{run_shell_command_capture, CommandEnv, Logger};
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::raw::c_int;
    use std::process::Command;
    use std::fs;
    use tempfile::TempDir;

    #[cfg(unix)]
    fn capture_stderr<F: FnOnce()>(f: F) -> String {
        extern "C" {
            fn pipe(fds: *mut c_int) -> c_int;
            fn dup(fd: c_int) -> c_int;
            fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
            fn close(fd: c_int) -> c_int;
        }

        unsafe {
            let mut fds = [0 as c_int; 2];
            if pipe(fds.as_mut_ptr()) != 0 {
                panic!("pipe failed");
            }

            let read_fd = fds[0];
            let write_fd = fds[1];
            let stderr_fd = std::io::stderr().as_raw_fd();
            let saved_stderr = dup(stderr_fd);

            if saved_stderr < 0 {
                let _ = close(read_fd);
                let _ = close(write_fd);
                panic!("dup stderr failed");
            }

            if dup2(write_fd, stderr_fd) < 0 {
                let _ = close(saved_stderr);
                let _ = close(read_fd);
                let _ = close(write_fd);
                panic!("dup2 stderr failed");
            }

            let _ = close(write_fd);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

            if dup2(saved_stderr, stderr_fd) < 0 {
                let _ = close(saved_stderr);
                let _ = close(read_fd);
                panic!("restore stderr failed");
            }
            let _ = close(saved_stderr);

            let mut output = Vec::new();
            let mut reader = std::fs::File::from_raw_fd(read_fd);
            reader.read_to_end(&mut output).expect("read stderr");
            let output = String::from_utf8_lossy(&output).into_owned();

            if let Err(payload) = result {
                std::panic::resume_unwind(payload);
            }

            output
        }
    }

    #[test]
    fn reduce_overage_returns_over_when_value_is_none() {
        let mut max_bytes = 10;
        let remaining = CommandEnv::reduce_overage(&mut max_bytes, None, 5);
        assert_eq!(remaining, 5);
        assert_eq!(max_bytes, 10);
    }

    #[test]
    fn reduce_overage_returns_over_when_current_is_zero() {
        let mut max_bytes = 0;
        let remaining = CommandEnv::reduce_overage(&mut max_bytes, Some("abc"), 5);
        assert_eq!(remaining, 5);
        assert_eq!(max_bytes, 0);
    }

    #[test]
    fn apply_can_hit_unreduced_total_overage_path() {
        // Force `total > TRUDGER_ENV_TOTAL_MAX_BYTES` via vars that are NOT reduced by `reduce_overage`,
        // so `new_total == total` and we exercise the `if new_total < total { .. }` false region.
        let huge = "x".repeat(super::TRUDGER_ENV_VALUE_MAX_BYTES + 1024);
        let env = CommandEnv {
            cwd: None,
            config_path: "config".to_string(),
            scratch_dir: None,
            task_id: None,
            task_show: None,
            task_status: Some(huge.clone()),
            target_status: None,
            agent_prompt: None,
            agent_phase: None,
            agent_profile: None,
            agent_invocation_id: None,
            completed: Some(huge),
            needs_human: None,
            notify_event: None,
            notify_duration_ms: None,
            notify_folder: None,
            notify_exit_code: None,
            notify_task_id: None,
            notify_task_description: None,
            notify_message: None,
            notify_payload_path: None,
        };

        let mut cmd = Command::new("true");
        let logger = Logger::new(None);
        env.apply(&mut cmd, &logger, "test", "task");
    }

    #[cfg(unix)]
    #[test]
    fn command_env_truncates_oversized_prompt_value_and_warns() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        crate::unit_tests::reset_test_env();

        let max = 64 * 1024;
        let ch = '\u{20AC}';
        let per_char = ch.len_utf8();
        assert_eq!(per_char, 3, "expected 3-byte UTF-8 character");

        let repeat = max / per_char + 1;
        let large: String = std::iter::repeat_n(ch, repeat).collect();
        let expected_len = (max / per_char).to_string();

        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("trudger.log");
        let logger = Logger::new(Some(log_path.clone()));
        let env = CommandEnv {
            cwd: None,
            config_path: "config".to_string(),
            scratch_dir: None,
            task_id: None,
            task_show: None,
            task_status: None,
            target_status: None,
            agent_prompt: Some(large),
            agent_phase: None,
            completed: None,
            needs_human: None,
            notify_event: None,
            notify_duration_ms: None,
            notify_folder: None,
            notify_exit_code: None,
            notify_task_id: None,
            notify_task_description: None,
            notify_message: None,
            notify_payload_path: None,
            agent_profile: None,
            agent_invocation_id: None,
        };

        let stderr = capture_stderr(|| {
            let result = run_shell_command_capture(
                "printf '%s' \"${#TRUDGER_AGENT_PROMPT}\"",
                "label",
                "task-id",
                &[],
                &env,
                &logger,
            )
            .expect("capture should succeed");
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout, expected_len);
        });

        assert!(
            stderr.contains("Warning: TRUDGER_AGENT_PROMPT"),
            "expected truncation warning, got: {stderr:?}"
        );

        let contents = fs::read_to_string(&log_path).expect("read log file");
        assert!(
            contents.contains("env_truncate label=label task=task-id key=TRUDGER_AGENT_PROMPT"),
            "expected env_truncate transition log, got: {contents:?}"
        );

        crate::unit_tests::reset_test_env();
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

    env.apply(&mut cmd, logger, log_label, task_token);

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
