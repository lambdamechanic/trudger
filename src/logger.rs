use chrono::Utc;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub(crate) struct Logger {
    path: Option<PathBuf>,
    disabled: AtomicBool,
}

impl Logger {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            disabled: AtomicBool::new(false),
        }
    }

    pub(crate) fn log_transition(&self, message: &str) {
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

pub(crate) fn sanitize_log_value(value: &str) -> String {
    value
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
