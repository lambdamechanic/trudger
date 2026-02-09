use serde::{Deserialize, Deserializer};
use serde_yaml::{Mapping, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::task_types::ReviewLoopLimit;

fn deserialize_log_path<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(value)))
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub agent_command: String,
    pub agent_review_command: String,
    pub commands: Commands,
    pub hooks: Hooks,
    pub review_loop_limit: ReviewLoopLimit,
    #[serde(default, deserialize_with = "deserialize_log_path")]
    pub log_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Commands {
    pub next_task: Option<String>,
    pub task_show: String,
    pub task_status: String,
    pub task_update_in_progress: String,
    pub reset_task: String,
}

#[derive(Debug, Deserialize, Clone)]
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
    load_config_from_str(&path.display().to_string(), &content)
}

pub(crate) fn load_config_from_str(label: &str, content: &str) -> Result<LoadedConfig, String> {
    let value: Value = serde_yaml::from_str(content)
        .map_err(|err| format!("Failed to parse config {}: {}", label, err))?;
    let mapping = match value {
        Value::Mapping(mapping) => mapping,
        _ => return Err(format!("Config {} must be a YAML mapping", label)),
    };

    let warnings = unknown_config_keys(&mapping);
    emit_unknown_key_warnings(&warnings);
    validate_required_fields(&mapping)?;

    // `serde_yaml` doesn't reliably include the failing key path for custom
    // deserialization errors (like `ReviewLoopLimit`). Track the path explicitly
    // so errors remain actionable.
    let deserializer = serde_yaml::Deserializer::from_str(content);
    let config: Config = serde_path_to_error::deserialize(deserializer)
        .map_err(|err| format!("Failed to parse config {}: {}", label, err))?;

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

fn unknown_config_keys(mapping: &Mapping) -> Vec<String> {
    let mut keys = unknown_top_level_keys(mapping);
    keys.extend(unknown_nested_keys(
        mapping,
        "commands",
        &[
            "next_task",
            "task_show",
            "task_status",
            "task_update_in_progress",
            "reset_task",
        ],
    ));
    keys.extend(unknown_nested_keys(
        mapping,
        "hooks",
        &["on_completed", "on_requires_human", "on_doctor_setup"],
    ));
    keys
}

fn unknown_nested_keys(mapping: &Mapping, mapping_key: &str, allowed: &[&str]) -> Vec<String> {
    let key = Value::String(mapping_key.to_string());
    let Some(Value::Mapping(nested)) = mapping.get(&key) else {
        return Vec::new();
    };

    nested
        .keys()
        .filter_map(|key| key.as_str().map(|value| value.to_string()))
        .filter(|key| !allowed.contains(&key.as_str()))
        .map(|key| format!("{}.{}", mapping_key, key))
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
    require_non_empty_string(hooks, "on_requires_human", "hooks.on_requires_human")?;
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
    use tempfile::TempDir;

    fn write_temp_config(contents: &str) -> NamedTempFile {
        let file = NamedTempFile::new().expect("create temp file");
        fs::write(file.path(), contents).expect("write temp config");
        file
    }

    #[test]
    fn config_must_be_yaml_mapping() {
        let file = write_temp_config("[]");
        let err = load_config(file.path()).expect_err("expected mapping error");
        assert!(err.contains("must be a YAML mapping"));
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
        assert!(err.contains("agent_command"));

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
        assert!(err.contains("agent_review_command"));
    }

    #[test]
    fn missing_required_mapping_errors() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing commands mapping");
        assert!(err.contains("commands"));

        let config = r#"
agent_command: "agent"
agent_review_command: "review"
review_loop_limit: 3
commands:
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing hooks mapping");
        assert!(err.contains("hooks"));
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
        assert!(err.contains("agent_command"));
    }

    #[test]
    fn empty_and_wrong_type_required_value_errors() {
        let config = r#"
agent_command: ""
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
        let err = load_config(file.path()).expect_err("expected empty agent_command");
        assert!(err.contains("agent_command"));

        let config = r#"
agent_command: ["agent"]
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
        let err = load_config(file.path()).expect_err("expected agent_command type error");
        assert!(err.contains("agent_command"));
        assert!(err.contains("string"));
    }

    #[test]
    fn missing_review_loop_limit_errors() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing review_loop_limit");
        assert!(err.contains("review_loop_limit"));
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
        assert!(err.contains("review_loop_limit"));
    }

    #[test]
    fn zero_review_loop_limit_is_rejected_with_actionable_error() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 0
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected zero review_loop_limit");
        assert!(err.contains("review_loop_limit"));
        assert!(err.contains("positive integer") | err.contains("got 0"));
        assert!(err.contains(&file.path().display().to_string()));
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
        assert!(err.contains("commands"));
    }

    #[test]
    fn commands_mapping_must_be_mapping() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands: "not-a-mapping"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected commands mapping type error");
        assert!(err.contains("commands"));
        assert!(err.contains("mapping"));
    }

    #[test]
    fn missing_task_update_in_progress_errors() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  reset_task: "reset"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing task_update_in_progress");
        assert!(err.contains("commands.task_update_in_progress"));
    }

    #[test]
    fn invalid_yaml_includes_path() {
        let file = write_temp_config("agent_command: [");
        let err = load_config(file.path()).expect_err("expected parse error");
        let path = file.path().display().to_string();
        assert!(err.contains(&path));
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
    fn unknown_nested_keys_reported() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
  extra_command_key: "mystery"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
  extra_hook_key: "mystery"
"#;
        let file = write_temp_config(config);
        let loaded = load_config(file.path()).expect("config should load");
        assert_eq!(
            loaded.warnings,
            vec![
                "commands.extra_command_key".to_string(),
                "hooks.extra_hook_key".to_string()
            ]
        );
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
        assert!(err.contains("codex_command"));
        assert!(err.contains("agent_command"));
        assert!(err.contains("Migration"));
    }

    #[test]
    fn optional_doctor_setup_value_errors() {
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
  on_doctor_setup: null
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected null on_doctor_setup error");
        assert!(err.contains("hooks.on_doctor_setup"));

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
  on_doctor_setup: ""
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected empty on_doctor_setup error");
        assert!(err.contains("hooks.on_doctor_setup"));

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
  on_doctor_setup: 123
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected non-string on_doctor_setup error");
        assert!(err.contains("hooks.on_doctor_setup"));
        assert!(err.contains("string"));
    }

    #[test]
    fn optional_log_path_value_errors() {
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
log_path: 123
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected non-string log_path error");
        assert!(err.contains("log_path"));
        assert!(err.contains("string"));
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
        assert_eq!(loaded.config.log_path, None);
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
        assert_eq!(loaded.config.log_path, None);
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
        assert!(err.contains("log_path"));
    }

    #[test]
    fn read_config_errors_include_path() {
        let temp = TempDir::new().expect("temp dir");
        let dir_path = temp.path().join("config-dir");
        fs::create_dir_all(&dir_path).expect("create config dir");
        let err = load_config(&dir_path).expect_err("expected read error");
        assert!(err.contains("Failed to read config"));
        assert!(err.contains(&dir_path.display().to_string()));
    }

    #[test]
    fn deserialize_errors_include_path_and_details() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: "not-a-number"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected deserialize error");
        assert!(err.contains("Failed to parse config"));
        assert!(err.contains(&file.path().display().to_string()));
        let has_invalid = err.contains("invalid");
        let has_expected = err.contains("expected");
        let has_not_a_number = err.contains("not-a-number");
        assert!(has_invalid | has_expected | has_not_a_number);
    }

    #[test]
    fn missing_command_field_errors_are_specific() {
        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing task_show");
        assert!(err.contains("commands.task_show"));

        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_update_in_progress: "update"
  reset_task: "reset"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing task_status");
        assert!(err.contains("commands.task_status"));

        let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing reset_task");
        assert!(err.contains("commands.reset_task"));
    }

    #[test]
    fn missing_hook_field_errors_are_specific() {
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
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing on_completed");
        assert!(err.contains("hooks.on_completed"));

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
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing on_requires_human");
        assert!(err.contains("hooks.on_requires_human"));
    }
}
