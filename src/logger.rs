use chrono::Utc;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::shell::{run_shell_command_status, CommandEnv};

#[derive(Debug)]
pub(crate) struct Logger {
    path: Option<PathBuf>,
    disabled: AtomicBool,
    all_logs_notification_command: Option<String>,
    notification_config_path: String,
    notification_in_flight: AtomicBool,
    notification_run_started_at: Option<Instant>,
}

impl Logger {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            disabled: AtomicBool::new(false),
            all_logs_notification_command: None,
            notification_config_path: String::new(),
            notification_in_flight: AtomicBool::new(false),
            notification_run_started_at: None,
        }
    }

    pub(crate) fn configure_all_logs_notification(
        &mut self,
        hook_command: Option<&str>,
        config_path: &Path,
    ) {
        self.all_logs_notification_command = hook_command
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        self.notification_config_path = config_path.display().to_string();
        self.notification_run_started_at = Some(Instant::now());
    }

    pub(crate) fn log_transition(&self, message: &str) {
        self.dispatch_all_logs_notification_if_needed(message);

        self.write_transition(message);
    }

    fn write_transition(&self, message: &str) {
        let Some(path) = &self.path else {
            return;
        };
        if self.disabled.load(Ordering::Relaxed) {
            return;
        }
        let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let sanitized = sanitize_log_value(message);
        let line = format!("{} {}\n", ts, sanitized);
        let mut file = match fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(file) => file,
            Err(err) => {
                self.disable_with_warning(path, &err);
                return;
            }
        };
        if let Err(err) = file.write_all(line.as_bytes()) {
            self.disable_with_warning(path, &err);
        }
    }

    fn dispatch_all_logs_notification_if_needed(&self, message: &str) {
        let Some(command) = self.all_logs_notification_command.as_deref() else {
            return;
        };

        if self
            .notification_in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let duration_ms = self
            .notification_run_started_at
            .map(|started_at| started_at.elapsed().as_millis())
            .unwrap_or(0);
        let folder = env::current_dir()
            .ok()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let env = CommandEnv {
            cwd: None,
            config_path: self.notification_config_path.clone(),
            scratch_dir: None,
            task_id: None,
            task_show: None,
            task_status: None,
            target_status: None,
            prompt: None,
            review_prompt: None,
            completed: None,
            needs_human: None,
            notify_event: Some("log".to_string()),
            notify_duration_ms: Some(duration_ms.to_string()),
            notify_folder: Some(folder),
            notify_exit_code: Some(String::new()),
            notify_task_id: Some(String::new()),
            notify_task_description: Some(String::new()),
            notify_message: Some(redact_transition_message_for_notification(message)),
        };

        let result = run_shell_command_status(command, "on_notification", "none", &[], &env, self);
        match result {
            Ok(0) => {}
            Ok(exit_code) => {
                // Avoid recursive notification dispatch for notification-generated transitions.
                self.write_transition(&format!(
                    "notification_hook_failed event=log task=none exit_code={}",
                    exit_code
                ));
                let mut stderr = std::io::stderr().lock();
                let _ = writeln!(
                    stderr,
                    "Warning: notification hook failed with exit code {}.",
                    exit_code
                );
            }
            Err(err) => {
                let escaped = sanitize_log_value(&err);
                self.write_transition(&format!(
                    "notification_hook_failed event=log task=none err={}",
                    escaped
                ));
                let mut stderr = std::io::stderr().lock();
                let _ = writeln!(stderr, "Warning: failed to run notification hook: {}.", err);
            }
        }

        self.notification_in_flight.store(false, Ordering::SeqCst);
    }

    fn disable_with_warning(&self, path: &Path, err: &std::io::Error) {
        // Keep the program running, but surface logging failures once and stop retrying.
        if self
            .disabled
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // Avoid `eprintln!` so tests can reliably capture stderr via fd redirection.
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(
                stderr,
                "Warning: transition logging disabled log_path={} io_error={}",
                path.display(),
                err
            );
        }
    }
}

fn redact_transition_message_for_notification(message: &str) -> String {
    let mut redacted = sanitize_log_value(message);
    redacted = redact_field_between_markers(redacted, "command=", Some(" args="));
    redact_field_between_markers(redacted, "args=", None)
}

fn redact_field_between_markers(input: String, key: &str, end_marker: Option<&str>) -> String {
    let Some(key_offset) = input.find(key) else {
        return input;
    };
    let value_start = key_offset + key.len();
    let value_end = match end_marker {
        Some(marker) => input[value_start..]
            .find(marker)
            .map(|offset| value_start + offset)
            .unwrap_or(input.len()),
        None => input.len(),
    };

    let mut out = String::with_capacity(input.len());
    out.push_str(&input[..value_start]);
    out.push_str("[REDACTED]");
    out.push_str(&input[value_end..]);
    out
}

pub(crate) fn sanitize_log_value(value: &str) -> String {
    value
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
