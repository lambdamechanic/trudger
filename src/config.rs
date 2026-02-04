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
    pub log_path: String,
}

#[derive(Debug, Deserialize)]
pub struct Commands {
    pub next_task: String,
    pub task_show: String,
    pub task_status: String,
    pub task_update_in_progress: String,
}

#[derive(Debug, Deserialize)]
pub struct Hooks {
    pub on_completed: String,
    pub on_requires_human: String,
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
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
        eprintln!("Warning: unknown config key: {}", key);
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
    require_non_empty_string(mapping, "agent_command", "agent_command")?;
    require_non_empty_string(mapping, "agent_review_command", "agent_review_command")?;
    require_non_empty_string(mapping, "log_path", "log_path")?;
    require_non_null(mapping, "review_loop_limit", "review_loop_limit")?;

    let commands = require_mapping(mapping, "commands", "commands")?;
    require_non_empty_string(commands, "next_task", "commands.next_task")?;
    require_non_empty_string(commands, "task_show", "commands.task_show")?;
    require_non_empty_string(commands, "task_status", "commands.task_status")?;
    require_non_empty_string(
        commands,
        "task_update_in_progress",
        "commands.task_update_in_progress",
    )?;

    let hooks = require_mapping(mapping, "hooks", "hooks")?;
    require_non_empty_string(hooks, "on_completed", "hooks.on_completed")?;
    require_non_empty_string(
        hooks,
        "on_requires_human",
        "hooks.on_requires_human",
    )?;

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
}
