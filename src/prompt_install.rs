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
    use chrono::TimeZone;

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
        assert_eq!(err.op(), "read");
        assert_eq!(err.path(), prompt.as_path());
        let message = err.to_string();
        assert!(message.contains("read"));
        assert!(message.contains(&prompt.display().to_string()));
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
        assert!(next.ends_with(format!(
            "{}.bak-{}-2",
            prompt.file_name().unwrap().to_string_lossy(),
            timestamp
        )));
    }

    #[test]
    fn overwrite_prompt_with_backup_at_creates_timestamped_sibling_backup_before_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        std::fs::write(&prompt, "old").expect("write prompt");

        let now = Utc.with_ymd_and_hms(2026, 2, 9, 12, 44, 25).unwrap();
        let backup =
            overwrite_prompt_with_backup_at(&prompt, "new", now).expect("overwrite with backup");
        let backup = backup.expect("expected backup");
        assert_eq!(
            backup.file_name().unwrap().to_string_lossy(),
            "trudge.md.bak-20260209T124425Z"
        );

        assert_eq!(
            std::fs::read_to_string(&backup).expect("read backup"),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(&prompt).expect("read prompt"),
            "new"
        );
    }

    #[test]
    fn overwrite_prompt_with_backup_at_adds_collision_suffix_when_backup_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        std::fs::write(&prompt, "old").expect("write prompt");

        let now = Utc.with_ymd_and_hms(2026, 2, 9, 12, 44, 25).unwrap();
        let occupied = dir.path().join("trudge.md.bak-20260209T124425Z");
        std::fs::write(&occupied, "occupied").expect("write occupied backup");

        let backup =
            overwrite_prompt_with_backup_at(&prompt, "new", now).expect("overwrite with backup");
        let backup = backup.expect("expected backup");
        assert_eq!(
            backup.file_name().unwrap().to_string_lossy(),
            "trudge.md.bak-20260209T124425Z-2"
        );
    }

    #[test]
    fn codex_prompts_dir_returns_expected_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expected = dir.path().join(".codex").join("prompts");
        assert_eq!(codex_prompts_dir(dir.path()), expected);
    }

    #[test]
    fn ensure_prompts_dir_creates_and_returns_prompts_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompts = ensure_prompts_dir(dir.path()).expect("ensure");
        assert_eq!(prompts, codex_prompts_dir(dir.path()));
        assert!(prompts.is_dir());
    }

    #[test]
    fn ensure_prompts_dir_errors_when_prompts_path_is_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("create .codex");
        let prompts_path = codex_dir.join("prompts");
        std::fs::write(&prompts_path, "not-a-dir").expect("write prompts as file");

        let err = ensure_prompts_dir(dir.path()).expect_err("expected error");
        assert_eq!(err.op(), "mkdir");
        assert_eq!(err.path(), prompts_path.as_path());
    }

    #[test]
    fn detect_prompt_state_errors_when_prompt_is_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        std::fs::create_dir_all(&prompt).expect("create prompt dir");

        let err = detect_prompt_state(&prompt, "x").expect_err("expected error");
        assert_eq!(err.op(), "read");
        assert_eq!(err.path(), prompt.as_path());
    }

    #[test]
    fn write_prompt_if_missing_returns_false_when_file_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        std::fs::write(&prompt, "existing").expect("write prompt");

        let wrote = write_prompt_if_missing(&prompt, "default").expect("write");
        assert!(!wrote);
        assert_eq!(
            std::fs::read_to_string(&prompt).expect("read prompt"),
            "existing"
        );
    }

    #[test]
    fn write_prompt_if_missing_creates_parent_and_writes_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("nested").join("trudge.md");

        let wrote = write_prompt_if_missing(&prompt, "default").expect("write");
        assert!(wrote);
        assert_eq!(
            std::fs::read_to_string(&prompt).expect("read prompt"),
            "default"
        );
    }

    #[test]
    fn write_prompt_if_missing_errors_when_parent_is_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let parent = dir.path().join("not-a-dir");
        std::fs::write(&parent, "x").expect("write parent as file");
        let prompt = parent.join("trudge.md");

        let err = write_prompt_if_missing(&prompt, "default").expect_err("expected error");
        assert_eq!(err.op(), "mkdir");
        assert_eq!(err.path(), parent.as_path());
    }

    #[test]
    fn write_prompt_if_missing_skips_parent_create_when_parent_is_none() {
        let err = write_prompt_if_missing(Path::new(""), "default").expect_err("expected error");
        assert_eq!(err.op(), "write");
        assert_eq!(err.path(), Path::new(""));
    }

    #[cfg(unix)]
    #[test]
    fn write_prompt_if_missing_errors_when_parent_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let parent = dir.path().join("readonly");
        std::fs::create_dir_all(&parent).expect("create parent dir");
        let mut permissions = std::fs::metadata(&parent).expect("metadata").permissions();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&parent, permissions).expect("chmod readonly");

        let prompt = parent.join("trudge.md");
        let err = write_prompt_if_missing(&prompt, "default").expect_err("expected error");
        assert_eq!(err.op(), "write");
        assert_eq!(err.path(), prompt.as_path());

        let mut permissions = std::fs::metadata(&parent).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&parent, permissions).expect("restore permissions");
    }

    #[test]
    fn overwrite_prompt_with_backup_returns_none_when_not_confirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        let backup = overwrite_prompt_with_backup(&prompt, "new", false).expect("overwrite");
        assert!(backup.is_none());
    }

    #[test]
    fn overwrite_prompt_with_backup_at_writes_without_backup_when_prompt_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");

        let now = Utc.with_ymd_and_hms(2026, 2, 9, 12, 44, 25).unwrap();
        let backup = overwrite_prompt_with_backup_at(&prompt, "new", now).expect("overwrite");
        assert!(backup.is_none());
        assert_eq!(
            std::fs::read_to_string(&prompt).expect("read prompt"),
            "new"
        );
    }

    #[test]
    fn overwrite_prompt_with_backup_at_errors_when_parent_is_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let parent = dir.path().join("not-a-dir");
        std::fs::write(&parent, "x").expect("write parent as file");
        let prompt = parent.join("trudge.md");
        let now = Utc.with_ymd_and_hms(2026, 2, 9, 12, 44, 25).unwrap();

        let err = overwrite_prompt_with_backup_at(&prompt, "new", now).expect_err("expected error");
        assert_eq!(err.op(), "mkdir");
        assert_eq!(err.path(), parent.as_path());
    }

    #[test]
    fn overwrite_prompt_with_backup_at_errors_when_prompt_is_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        std::fs::create_dir_all(&prompt).expect("create prompt dir");
        let now = Utc.with_ymd_and_hms(2026, 2, 9, 12, 44, 25).unwrap();

        let err = overwrite_prompt_with_backup_at(&prompt, "new", now).expect_err("expected error");
        assert_eq!(err.op(), "backup");
        assert_eq!(err.path(), prompt.as_path());
    }

    #[cfg(unix)]
    #[test]
    fn overwrite_prompt_with_backup_at_errors_when_prompt_has_no_filename() {
        let now = Utc.with_ymd_and_hms(2026, 2, 9, 12, 44, 25).unwrap();
        let err = overwrite_prompt_with_backup_at(Path::new("/"), "new", now).expect_err("error");
        assert_eq!(err.op(), "backup");
        assert_eq!(err.path(), Path::new("/"));
    }

    #[test]
    fn next_prompt_backup_path_with_timestamp_errors_when_path_has_no_filename() {
        let err = next_prompt_backup_path_with_timestamp(Path::new(""), "20260209T124425Z")
            .expect_err("error");
        assert_eq!(err.op(), "backup");
        assert_eq!(err.path(), Path::new(""));
    }

    #[test]
    fn next_prompt_backup_path_with_timestamp_skips_existing_suffixes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        std::fs::write(&prompt, "x").expect("write prompt");

        let timestamp = "20260209T124425Z";
        let base = dir.path().join("trudge.md.bak-20260209T124425Z");
        let second = dir.path().join("trudge.md.bak-20260209T124425Z-2");
        std::fs::write(&base, "backup").expect("write base");
        std::fs::write(&second, "backup").expect("write second");

        let next = next_prompt_backup_path_with_timestamp(&prompt, timestamp).expect("next");
        assert!(next.ends_with("trudge.md.bak-20260209T124425Z-3"));
    }

    #[test]
    fn next_prompt_backup_path_with_timestamp_returns_last_candidate_when_exhausted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = dir.path().join("trudge.md");
        std::fs::write(&prompt, "x").expect("write prompt");

        let timestamp = "20260209T124425Z";
        std::fs::write(dir.path().join("trudge.md.bak-20260209T124425Z"), "backup")
            .expect("write base backup");
        for index in 2u32..=1000 {
            std::fs::write(
                dir.path()
                    .join(format!("trudge.md.bak-20260209T124425Z-{}", index)),
                "backup",
            )
            .expect("write backup");
        }

        let next = next_prompt_backup_path_with_timestamp(&prompt, timestamp).expect("next");
        assert!(next.ends_with("trudge.md.bak-20260209T124425Z-1000"));
    }
}
