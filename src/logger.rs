use chrono::Utc;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) struct Logger {
    path: Option<PathBuf>,
}

impl Logger {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    pub(crate) fn log_transition(&self, message: &str) {
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

pub(crate) fn sanitize_log_value(value: &str) -> String {
    value
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
