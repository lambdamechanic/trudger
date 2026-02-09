#![allow(dead_code)]
// Helpers for wizard-managed prompt installation/update.
//
// This module is intentionally UI-free: the wizard flow decides when to ask for
// confirmation and passes the result here as a boolean.

use chrono::{DateTime, Utc};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptState {
    Missing,
    MatchesDefault,
    Differs,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptInstallError {
    op: &'static str,
    path: PathBuf,
    details: String,
}

impl PromptInstallError {
    fn new(op: &'static str, path: &Path, details: impl Into<String>) -> Self {
        Self {
            op,
            path: path.to_path_buf(),
            details: details.into(),
        }
    }

    pub(crate) fn op(&self) -> &'static str {
        self.op
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for PromptInstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Prompt install error ({}): {}: {}",
            self.op,
            self.path.display(),
            self.details
        )
    }
}

impl std::error::Error for PromptInstallError {}

pub(crate) fn codex_prompts_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(".codex").join("prompts")
}

pub(crate) fn ensure_prompts_dir(home_dir: &Path) -> Result<PathBuf, PromptInstallError> {
    let dir = codex_prompts_dir(home_dir);
    std::fs::create_dir_all(&dir).map_err(|err| {
        PromptInstallError::new(
            "mkdir",
            &dir,
            format!("failed to create directory: {}", err),
        )
    })?;
    Ok(dir)
}

pub(crate) fn detect_prompt_state(
    prompt_path: &Path,
    default_contents: &str,
) -> Result<PromptState, PromptInstallError> {
    if !prompt_path.exists() {
        return Ok(PromptState::Missing);
    }

    let bytes = std::fs::read(prompt_path).map_err(|err| {
        PromptInstallError::new("read", prompt_path, format!("failed to read: {}", err))
    })?;
    let on_disk = String::from_utf8(bytes).map_err(|err| {
        PromptInstallError::new(
            "read",
            prompt_path,
            format!("failed to decode as UTF-8: {}", err),
        )
    })?;

    let disk_norm = normalize_prompt_text(&on_disk);
    let default_norm = normalize_prompt_text(default_contents);
    if disk_norm == default_norm {
        Ok(PromptState::MatchesDefault)
    } else {
        Ok(PromptState::Differs)
    }
}

pub(crate) fn write_prompt_if_missing(
    prompt_path: &Path,
    default_contents: &str,
) -> Result<bool, PromptInstallError> {
    if prompt_path.exists() {
        return Ok(false);
    }

    if let Some(parent) = prompt_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            PromptInstallError::new(
                "mkdir",
                parent,
                format!("failed to create directory: {}", err),
            )
        })?;
    }

    std::fs::write(prompt_path, default_contents).map_err(|err| {
        PromptInstallError::new("write", prompt_path, format!("failed to write: {}", err))
    })?;
    Ok(true)
}

pub(crate) fn overwrite_prompt_with_backup(
    prompt_path: &Path,
    default_contents: &str,
    overwrite_confirmed: bool,
) -> Result<Option<PathBuf>, PromptInstallError> {
    if !overwrite_confirmed {
        return Ok(None);
    }
    overwrite_prompt_with_backup_at(prompt_path, default_contents, Utc::now())
}

fn overwrite_prompt_with_backup_at(
    prompt_path: &Path,
    default_contents: &str,
    now: DateTime<Utc>,
) -> Result<Option<PathBuf>, PromptInstallError> {
    if let Some(parent) = prompt_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            PromptInstallError::new(
                "mkdir",
                parent,
                format!("failed to create directory: {}", err),
            )
        })?;
    }

    // Only back up when there is something to back up.
    let backup_path = if prompt_path.exists() {
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let backup_path = next_prompt_backup_path_with_timestamp(prompt_path, &timestamp)?;
        std::fs::copy(prompt_path, &backup_path).map_err(|err| {
            PromptInstallError::new(
                "backup",
                prompt_path,
                format!(
                    "failed to copy {} -> {}: {}",
                    prompt_path.display(),
                    backup_path.display(),
                    err
                ),
            )
        })?;
        Some(backup_path)
    } else {
        None
    };

    std::fs::write(prompt_path, default_contents).map_err(|err| {
        PromptInstallError::new("write", prompt_path, format!("failed to write: {}", err))
    })?;

    Ok(backup_path)
}

pub(crate) fn next_prompt_backup_path_with_timestamp(
    prompt_path: &Path,
    timestamp: &str,
) -> Result<PathBuf, PromptInstallError> {
    let file_name = prompt_path
        .file_name()
        .ok_or_else(|| PromptInstallError::new("backup", prompt_path, "path has no filename"))?
        .to_string_lossy();

    let mut backup = prompt_path.with_file_name(format!("{}.bak-{}", file_name, timestamp));
    if !backup.exists() {
        return Ok(backup);
    }

    for index in 2u32..=1000 {
        backup = prompt_path.with_file_name(format!("{}.bak-{}-{}", file_name, timestamp, index));
        if !backup.exists() {
            return Ok(backup);
        }
    }

    Ok(backup)
}

fn normalize_prompt_text(text: &str) -> String {
    // Normalize line endings: CRLF/CR -> LF.
    let mut normalized = text.replace("\r\n", "\n").replace('\r', "\n");

    // Ignore a single trailing newline at EOF.
    if normalized.ends_with('\n') {
        normalized.pop();
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prompt_state_reports_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("missing.md");
        let state = detect_prompt_state(&prompt, "x").expect("state");
        assert_eq!(state, PromptState::Missing);
    }

    #[test]
    fn detect_prompt_state_normalizes_line_endings_and_ignores_single_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");

        let default = "a\nb"; // no trailing newline
        std::fs::write(&prompt, "a\r\nb\r\n").expect("write prompt");

        let state = detect_prompt_state(&prompt, default).expect("state");
        assert_eq!(state, PromptState::MatchesDefault);

        std::fs::write(&prompt, "a\nb\n\n").expect("write prompt");
        let state = detect_prompt_state(&prompt, default).expect("state");
        assert_eq!(state, PromptState::Differs);
    }

    #[test]
    fn detect_prompt_state_errors_on_utf8_decode_failure_and_mentions_op_and_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");

        std::fs::write(&prompt, [0xff, 0xfe, 0xfd]).expect("write invalid utf8");
        let err = detect_prompt_state(&prompt, "x").expect_err("expected error");
        let message = err.to_string();
        assert!(
            message.contains("read"),
            "expected error to mention op=read, got: {}",
            message
        );
        assert!(
            message.contains(&prompt.display().to_string()),
            "expected error to mention failing path, got: {}",
            message
        );
    }

    #[test]
    fn next_prompt_backup_path_with_timestamp_skips_existing_backups() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        std::fs::write(&prompt, "x").expect("write prompt");

        let timestamp = "20260209T124425Z";
        let first = dir.path().join(format!(
            "{}.bak-{}",
            prompt.file_name().unwrap().to_string_lossy(),
            timestamp
        ));
        std::fs::write(&first, "backup").expect("write first backup");

        let next = next_prompt_backup_path_with_timestamp(&prompt, timestamp).expect("next");
        assert!(
            next.ends_with(format!(
                "{}.bak-{}-2",
                prompt.file_name().unwrap().to_string_lossy(),
                timestamp
            )),
            "expected -2 suffix, got: {}",
            next.display()
        );
    }
}
