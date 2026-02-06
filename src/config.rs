use serde::Deserialize;
use serde_yaml::{Mapping, Value};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub agent_command: String,
    pub agent_review_command: String,
    pub commands: Commands,
    pub hooks: Hooks,
    pub review_loop_limit: u64,
    #[serde(default)]
    pub log_path: String,
}

#[derive(Debug, Deserialize)]
pub struct Commands {
    pub next_task: Option<String>,
    pub task_show: String,
    pub task_status: String,
    pub task_update_in_progress: String,
    pub reset_task: String,
}

#[derive(Debug, Deserialize)]
pub struct Hooks {
    pub on_completed: String,
    pub on_requires_human: String,
    #[serde(default)]
    pub on_doctor_setup: Option<String>,
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    #[allow(dead_code)]
    pub warnings: Vec<String>,
}

pub fn load_config(path: &Path) -> Result<LoadedConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read config {}: {}", path.display(), err))?;
    let value: Value = serde_yaml::from_str(&content)
        .map_err(|err| format!("Failed to parse config {}: {}", path.display(), err))?;
    let mapping = match value {
        Value::Mapping(mapping) => mapping,
        _ => {
            return Err(format!(
                "Config {} must be a YAML mapping",
                path.display()
            ))
        }
    };

    let warnings = unknown_top_level_keys(&mapping);
    emit_unknown_key_warnings(&warnings);
    validate_required_fields(&mapping)?;

    let config: Config = serde_yaml::from_value(Value::Mapping(mapping))
        .map_err(|err| format!("Failed to parse config {}: {}", path.display(), err))?;

    Ok(LoadedConfig { config, warnings })
}

fn emit_unknown_key_warnings(keys: &[String]) {
    for key in keys {
        eprintln!("Warning: Unknown config key: {}", key);
    }
}

fn unknown_top_level_keys(mapping: &Mapping) -> Vec<String> {
    let allowed = [
        "agent_command",
        "agent_review_command",
        "commands",
        "hooks",
        "review_loop_limit",
        "log_path",
    ];

    mapping
        .keys()
        .filter_map(|key| key.as_str().map(|value| value.to_string()))
        .filter(|key| !allowed.contains(&key.as_str()))
        .collect()
}

fn validate_required_fields(mapping: &Mapping) -> Result<(), String> {
    reject_deprecated_keys(mapping)?;
    require_non_empty_string(mapping, "agent_command", "agent_command")?;
    require_non_empty_string(mapping, "agent_review_command", "agent_review_command")?;
    require_non_null(mapping, "review_loop_limit", "review_loop_limit")?;
    validate_optional_string(mapping, "log_path", "log_path")?;

    let commands = require_mapping(mapping, "commands", "commands")?;
    require_non_empty_string(commands, "task_show", "commands.task_show")?;
    require_non_empty_string(commands, "task_status", "commands.task_status")?;
    require_non_empty_string(
        commands,
        "task_update_in_progress",
        "commands.task_update_in_progress",
    )?;
    require_non_empty_string(commands, "reset_task", "commands.reset_task")?;

    let hooks = require_mapping(mapping, "hooks", "hooks")?;
    require_non_empty_string(hooks, "on_completed", "hooks.on_completed")?;
    require_non_empty_string(
        hooks,
        "on_requires_human",
        "hooks.on_requires_human",
    )?;
    validate_optional_non_empty_string(hooks, "on_doctor_setup", "hooks.on_doctor_setup")?;

    Ok(())
}

fn reject_deprecated_keys(mapping: &Mapping) -> Result<(), String> {
    let key = Value::String("codex_command".to_string());
    if mapping.contains_key(&key) {
        return Err("Migration: codex_command is no longer supported; use agent_command and agent_review_command.".to_string());
    }
    Ok(())
}

fn require_mapping<'a>(
    mapping: &'a Mapping,
    key_name: &str,
    label: &str,
) -> Result<&'a Mapping, String> {
    let key = Value::String(key_name.to_string());
    match mapping.get(&key) {
        None => Err(format!("Missing required config value: {}", label)),
        Some(Value::Null) => Err(format!("{} must not be null", label)),
        Some(Value::Mapping(value)) => Ok(value),
        Some(_) => Err(format!("{} must be a mapping", label)),
    }
}

fn require_non_null(mapping: &Mapping, key_name: &str, label: &str) -> Result<(), String> {
    let key = Value::String(key_name.to_string());
    match mapping.get(&key) {
        None => Err(format!("Missing required config value: {}", label)),
        Some(Value::Null) => Err(format!("{} must not be null", label)),
        Some(_) => Ok(()),
    }
}

fn require_non_empty_string(mapping: &Mapping, key_name: &str, label: &str) -> Result<(), String> {
    let key = Value::String(key_name.to_string());
    match mapping.get(&key) {
        None => Err(format!("Missing required config value: {}", label)),
        Some(Value::Null) => Err(format!("{} must not be null", label)),
        Some(Value::String(value)) => {
            if value.trim().is_empty() {
                Err(format!("{} must not be empty", label))
            } else {
                Ok(())
            }
        }
        Some(_) => Err(format!("{} must be a string", label)),
    }
}

fn validate_optional_non_empty_string(
    mapping: &Mapping,
    key_name: &str,
    label: &str,
) -> Result<(), String> {
    let key = Value::String(key_name.to_string());
    match mapping.get(&key) {
        None => Ok(()),
        Some(Value::Null) => Err(format!("{} must not be null", label)),
        Some(Value::String(value)) => {
            if value.trim().is_empty() {
                Err(format!("{} must not be empty", label))
            } else {
                Ok(())
            }
        }
        Some(_) => Err(format!("{} must be a string", label)),
    }
}

fn validate_optional_string(mapping: &Mapping, key_name: &str, label: &str) -> Result<(), String> {
    let key = Value::String(key_name.to_string());
    match mapping.get(&key) {
        None => Ok(()),
        Some(Value::Null) => Err(format!("{} must not be null", label)),
        Some(Value::String(_)) => Ok(()),
        Some(_) => Err(format!("{} must be a string", label)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    fn write_temp_config(contents: &str) -> NamedTempFile {
        let file = NamedTempFile::new().expect("create temp file");
        fs::write(file.path(), contents).expect("write temp config");
        file
    }

    #[test]
    fn missing_agent_commands_error() {
        let config = r#"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing agent_command");
        assert!(
            err.contains("agent_command"),
            "error should name agent_command, got: {err}"
        );

        let config = r#"
agent_command: "agent"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing agent_review_command");
        assert!(
            err.contains("agent_review_command"),
            "error should name agent_review_command, got: {err}"
        );
    }

    #[test]
    fn null_required_value_errors() {
        let config = r#"
agent_command: null
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected null agent_command");
        assert!(
            err.contains("agent_command"),
            "error should name agent_command, got: {err}"
        );
    }

    #[test]
    fn null_review_loop_limit_errors() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: null
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected null review_loop_limit");
        assert!(
            err.contains("review_loop_limit"),
            "error should name review_loop_limit, got: {err}"
        );
    }

    #[test]
    fn null_commands_mapping_errors() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands: null
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected null commands");
        assert!(
            err.contains("commands"),
            "error should name commands, got: {err}"
        );
    }

    #[test]
    fn invalid_yaml_includes_path() {
        let file = write_temp_config("agent_command: [");
        let err = load_config(file.path()).expect_err("expected parse error");
        let path = file.path().display().to_string();
        assert!(
            err.contains(&path),
            "error should include path {path}, got: {err}"
        );
    }

    #[test]
    fn unknown_keys_reported() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
extra_key: true
"#;
        let file = write_temp_config(config);
        let loaded = load_config(file.path()).expect("config should load");
        assert_eq!(loaded.warnings, vec!["extra_key".to_string()]);
    }

    #[test]
    fn codex_command_is_rejected() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
codex_command: "codex"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected codex_command error");
        assert!(
            err.contains("codex_command"),
            "error should mention codex_command, got: {err}"
        );
        assert!(
            err.contains("agent_command"),
            "error should mention agent_command, got: {err}"
        );
        assert!(
            err.contains("Migration"),
            "error should include migration guidance, got: {err}"
        );
    }

    #[test]
    fn missing_log_path_is_allowed() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let loaded = load_config(file.path()).expect("config should load");
        assert_eq!(loaded.config.log_path, "");
    }

    #[test]
    fn empty_log_path_is_allowed() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
log_path: ""
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let loaded = load_config(file.path()).expect("config should load");
        assert_eq!(loaded.config.log_path, "");
    }

    #[test]
    fn null_log_path_errors() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
log_path: null
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected null log_path");
        assert!(
            err.contains("log_path"),
            "error should name log_path, got: {err}"
        );
    }
}
