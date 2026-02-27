use chrono::Utc;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::notification_payload::NotificationPayload;
use crate::shell::{
    run_shell_command_status, truncate_utf8_to_bytes, CommandEnv, TRUDGER_ENV_VALUE_MAX_BYTES,
};

struct NotificationInFlightGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> NotificationInFlightGuard<'a> {
    fn try_acquire(flag: &'a AtomicBool) -> Option<Self> {
        if flag
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(Self { flag })
        } else {
            None
        }
    }
}

impl Drop for NotificationInFlightGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub(crate) struct Logger {
    path: Option<PathBuf>,
    disabled: AtomicBool,
    all_logs_notification_command: Option<String>,
    notification_config_path: String,
    notification_invocation_folder: String,
    notification_in_flight: AtomicBool,
    notification_run_started_at: Option<Instant>,

    // Best-effort task context for all_logs notifications.
    notification_task_id: Option<String>,
    notification_task_show: Option<String>,
    notification_task_status: Option<String>,
    notification_task_description: String,
}

impl Logger {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            disabled: AtomicBool::new(false),
            all_logs_notification_command: None,
            notification_config_path: String::new(),
            notification_invocation_folder: String::new(),
            notification_in_flight: AtomicBool::new(false),
            notification_run_started_at: None,
            notification_task_id: None,
            notification_task_show: None,
            notification_task_status: None,
            notification_task_description: String::new(),
        }
    }

    pub(crate) fn configure_all_logs_notification(
        &mut self,
        hook_command: Option<&str>,
        config_path: &Path,
        invocation_folder: String,
    ) {
        self.all_logs_notification_command = hook_command
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        self.notification_config_path = config_path.display().to_string();
        self.notification_invocation_folder = invocation_folder;
        self.notification_run_started_at = None;
    }

    pub(crate) fn mark_all_logs_run_started_at(&mut self, run_started_at: Instant) {
        self.notification_run_started_at = Some(run_started_at);
    }

    pub(crate) fn set_all_logs_task_id(&mut self, task_id: Option<&str>) {
        match task_id {
            Some(value) => {
                if self.notification_task_id.as_deref() == Some(value) {
                    return;
                }
                self.notification_task_id = Some(value.to_string());
                self.notification_task_show = None;
                self.notification_task_status = None;
                self.notification_task_description.clear();
            }
            None => {
                self.notification_task_id = None;
                self.notification_task_show = None;
                self.notification_task_status = None;
                self.notification_task_description.clear();
            }
        }
    }

    pub(crate) fn set_all_logs_task_show(&mut self, task_show: Option<String>) {
        if self.notification_task_id.is_none() {
            return;
        }
        self.notification_task_show = task_show;
    }

    pub(crate) fn set_all_logs_task_status(&mut self, task_status: Option<&str>) {
        if self.notification_task_id.is_none() {
            return;
        }
        self.notification_task_status = task_status.map(|value| value.to_string());
    }

    pub(crate) fn set_all_logs_task_description(&mut self, task_description: String) {
        if self.notification_task_id.is_none() {
            return;
        }
        self.notification_task_description = task_description;
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

        let Some(_guard) = NotificationInFlightGuard::try_acquire(&self.notification_in_flight)
        else {
            return;
        };

        let duration_ms = self
            .notification_run_started_at
            .map(|started_at| started_at.elapsed().as_millis())
            .unwrap_or(0);
        let folder = self.notification_invocation_folder.clone();
        let redacted_message = redact_transition_message_for_notification(message);

        let task_id = self.notification_task_id.clone();
        let task_show = self.notification_task_show.clone();
        let task_status = self.notification_task_status.clone();
        let notify_task_id = task_id.clone().unwrap_or_default();
        let notify_task_description = task_id
            .as_ref()
            .map(|_| self.notification_task_description.clone())
            .unwrap_or_default();

        let mut env = CommandEnv {
            cwd: None,
            config_path: self.notification_config_path.clone(),
            scratch_dir: None,
            task_id,
            task_show,
            task_status,
            target_status: None,
            agent_prompt: None,
            agent_phase: None,
            agent_profile: None,
            agent_invocation_id: None,
            completed: None,
            needs_human: None,
            notify_event: Some("log".to_string()),
            notify_duration_ms: Some(duration_ms.to_string()),
            notify_folder: Some(folder),
            notify_exit_code: None,
            notify_task_id: Some(notify_task_id.clone()),
            notify_task_description: Some(notify_task_description.clone()),
            notify_message: Some(redacted_message.clone()),
            notify_payload_path: None,
        };

        let payload = NotificationPayload {
            event: "log".to_string(),
            duration_ms,
            folder: truncate_utf8_to_bytes(
                env.notify_folder.as_deref().unwrap_or_default(),
                TRUDGER_ENV_VALUE_MAX_BYTES,
            )
            .to_string(),
            exit_code: None,
            task_id: truncate_utf8_to_bytes(&notify_task_id, TRUDGER_ENV_VALUE_MAX_BYTES)
                .to_string(),
            task_description: truncate_utf8_to_bytes(
                &notify_task_description,
                TRUDGER_ENV_VALUE_MAX_BYTES,
            )
            .to_string(),
            message: Some(
                truncate_utf8_to_bytes(&redacted_message, TRUDGER_ENV_VALUE_MAX_BYTES).to_string(),
            ),
        };
        let payload_file = match payload.write_to_temp_file() {
            Ok(file) => file,
            Err(err) => {
                let escaped = sanitize_log_value(&err);
                self.write_transition(&format!(
                    "notification_hook_failed event=log task=none err={}",
                    escaped
                ));
                let mut stderr = std::io::stderr().lock();
                let _ = writeln!(
                    stderr,
                    "Warning: failed to prepare notification payload: {}.",
                    err
                );
                return;
            }
        };
        env.notify_payload_path = Some(payload_file.path().display().to_string());

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
