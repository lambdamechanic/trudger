use serde::{Deserialize, Deserializer};
use serde_yaml::{Mapping, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::task_types::ReviewLoopLimit;

fn deserialize_log_path<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
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
    pub task_update_status: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Hooks {
    pub on_completed: String,
    pub on_requires_human: String,
    #[serde(default)]
    pub on_doctor_setup: Option<String>,
    #[serde(default)]
    pub on_notification: Option<String>,
    #[serde(default)]
    pub on_notification_scope: Option<NotificationScope>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationScope {
    AllLogs,
    TaskBoundaries,
    RunBoundaries,
}

impl Hooks {
    pub fn effective_notification_scope(&self) -> Option<NotificationScope> {
        let has_notification_hook = self
            .on_notification
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        if !has_notification_hook {
            return None;
        }
        Some(
            self.on_notification_scope
                .unwrap_or(NotificationScope::TaskBoundaries),
        )
    }
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    #[allow(dead_code)]
    pub warnings: Vec<String>,
    pub active_profile: String,
    pub solve_invocation_id: String,
    pub review_invocation_id: String,
}

#[derive(Debug, Deserialize, Clone)]
struct ParsedProfile {
    trudge: String,
    trudge_review: String,
}

#[derive(Debug, Deserialize, Clone)]
struct ParsedInvocation {
    command: String,
}

#[derive(Debug, Deserialize, Clone)]
struct ParsedConfig {
    default_profile: String,
    profiles: HashMap<String, ParsedProfile>,
    invocations: HashMap<String, ParsedInvocation>,
    commands: Commands,
    hooks: Hooks,
    review_loop_limit: ReviewLoopLimit,
    #[serde(default, deserialize_with = "deserialize_log_path")]
    log_path: Option<PathBuf>,
}

#[allow(dead_code)]
pub fn load_config(path: &Path) -> Result<LoadedConfig, String> {
    load_config_with_profile(path, None)
}

pub(crate) fn load_config_with_profile(
    path: &Path,
    profile: Option<&str>,
) -> Result<LoadedConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read config {}: {}", path.display(), err))?;
    load_config_from_str_with_profile(&path.display().to_string(), &content, profile)
}

pub(crate) fn load_config_from_str(label: &str, content: &str) -> Result<LoadedConfig, String> {
    load_config_from_str_with_profile(label, content, None)
}

pub(crate) fn load_config_from_str_with_profile(
    label: &str,
    content: &str,
    profile: Option<&str>,
) -> Result<LoadedConfig, String> {
    let value: Value = serde_yaml::from_str(content)
        .map_err(|err| format!("Failed to parse config {}: {}", label, err))?;
    let mapping = match value {
        Value::Mapping(mapping) => mapping,
        _ => return Err(format!("Config {} must be a YAML mapping", label)),
    };

    let mut warnings = unknown_config_keys(&mapping);
    emit_unknown_key_warnings(&warnings);
    if let Some(warning) = notification_scope_without_hook_warning(&mapping) {
        eprintln!("Warning: {}", warning);
        warnings.push(warning);
    }
    validate_required_fields(&mapping)?;

    // `serde_yaml` doesn't reliably include the failing key path for custom
    // deserialization errors (like `ReviewLoopLimit`). Track the path explicitly
    // so errors remain actionable.
    let deserializer = serde_yaml::Deserializer::from_str(content);
    let config: ParsedConfig = serde_path_to_error::deserialize(deserializer)
        .map_err(|err| format!("Failed to parse config {}: {}", label, err))?;

    let resolved_commands = resolve_profile_commands(&config, profile)?;
    let config = Config {
        agent_command: resolved_commands.solve_command,
        agent_review_command: resolved_commands.review_command,
        commands: config.commands,
        hooks: config.hooks,
        review_loop_limit: config.review_loop_limit,
        log_path: config.log_path,
    };

    Ok(LoadedConfig {
        config,
        warnings,
        active_profile: resolved_commands.profile,
        solve_invocation_id: resolved_commands.solve_invocation_id,
        review_invocation_id: resolved_commands.review_invocation_id,
    })
}

struct ResolvedAgentCommands {
    profile: String,
    solve_invocation_id: String,
    review_invocation_id: String,
    solve_command: String,
    review_command: String,
}

fn resolve_profile_commands(
    config: &ParsedConfig,
    profile_override: Option<&str>,
) -> Result<ResolvedAgentCommands, String> {
    let profile_name = profile_override.unwrap_or(config.default_profile.as_str());
    let profile = config.profiles.get(profile_name).ok_or_else(|| {
        if profile_override.is_some() {
            format!("Unknown profile: {}", profile_name)
        } else {
            format!(
                "default_profile references missing profile: {}",
                config.default_profile
            )
        }
    })?;

    let agent_command = config
        .invocations
        .get(&profile.trudge)
        .ok_or_else(|| {
            format!(
                "profiles.{}.trudge references missing invocation: {}",
                profile_name, profile.trudge
            )
        })?
        .command
        .clone();

    let agent_review_command = config
        .invocations
        .get(&profile.trudge_review)
        .ok_or_else(|| {
            format!(
                "profiles.{}.trudge_review references missing invocation: {}",
                profile_name, profile.trudge_review
            )
        })?
        .command
        .clone();

    Ok(ResolvedAgentCommands {
        profile: profile_name.to_string(),
        solve_invocation_id: profile.trudge.clone(),
        review_invocation_id: profile.trudge_review.clone(),
        solve_command: agent_command,
        review_command: agent_review_command,
    })
}

fn emit_unknown_key_warnings(keys: &[String]) {
    for key in keys {
        eprintln!("Warning: Unknown config key: {}", key);
    }
}

fn unknown_top_level_keys(mapping: &Mapping) -> Vec<String> {
    let allowed = [
        "default_profile",
        "profiles",
        "invocations",
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
            "task_update_status",
        ],
    ));
    keys.extend(unknown_nested_keys(
        mapping,
        "hooks",
        &[
            "on_completed",
            "on_requires_human",
            "on_doctor_setup",
            "on_notification",
            "on_notification_scope",
        ],
    ));
    keys.extend(unknown_nested_profile_or_invocation_keys(
        mapping,
        "profiles",
        &["trudge", "trudge_review"],
    ));
    keys.extend(unknown_nested_profile_or_invocation_keys(
        mapping,
        "invocations",
        &["command"],
    ));
    keys
}

#[allow(dead_code)]
fn unknown_nested_profile_or_invocation_keys_legacy(
    mapping: &Mapping,
    mapping_key: &str,
    allowed: &[&str],
) -> Vec<String> {
    let key = Value::String(mapping_key.to_string());
    let Some(Value::Mapping(nested_entries)) = mapping.get(&key) else {
        return Vec::new();
    };

    nested_entries
        .iter()
        .filter_map(|(nested_key, nested_value)| {
            let nested_key = nested_key.as_str()?;
            let Value::Mapping(nested_mapping) = nested_value else {
                return Some(vec![format!(
                    "{}.{}
			 must be a mapping",
                    mapping_key, nested_key
                )]);
            };

            Some(
                nested_mapping
                    .keys()
                    .filter_map(|key| key.as_str().map(|value| value.to_string()))
                    .filter(|key| !allowed.contains(&key.as_str()))
                    .map(|key| format!("{}.{}.{}", mapping_key, nested_key, key))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

fn unknown_nested_profile_or_invocation_keys(
    mapping: &Mapping,
    mapping_key: &str,
    allowed: &[&str],
) -> Vec<String> {
    let key = Value::String(mapping_key.to_string());
    let Some(Value::Mapping(nested_entries)) = mapping.get(&key) else {
        return Vec::new();
    };

    nested_entries
        .iter()
        .filter_map(|(nested_key, nested_value)| {
            let nested_key = nested_key.as_str()?;
            let Value::Mapping(nested_mapping) = nested_value else {
                return Some(vec![format!(
                    "{}.{} must be a mapping",
                    mapping_key, nested_key
                )]);
            };

            Some(
                nested_mapping
                    .keys()
                    .filter_map(|key| key.as_str().map(|value| value.to_string()))
                    .filter(|key| !allowed.contains(&key.as_str()))
                    .map(|key| format!("{}.{}.{}", mapping_key, nested_key, key))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
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
    let default_profile = require_non_empty_string(mapping, "default_profile", "default_profile")?;
    let profiles = require_mapping(mapping, "profiles", "profiles")?;
    if profiles.is_empty() {
        return Err("profiles must not be empty".to_string());
    }
    let invocations = require_mapping(mapping, "invocations", "invocations")?;
    if invocations.is_empty() {
        return Err("invocations must not be empty".to_string());
    }

    for (invocation_id, invocation) in invocations {
        let invocation_id = invocation_id
            .as_str()
            .ok_or_else(|| "invocations keys must be strings".to_string())?;
        let invocation = match invocation {
            Value::Mapping(invocation) => invocation,
            _ => return Err(format!("invocations.{} must be a mapping", invocation_id)),
        };

        let _ = require_non_empty_string(
            invocation,
            "command",
            &format!("invocations.{}.command", invocation_id),
        )?;
    }

    for (profile_id, profile) in profiles {
        let profile_id = profile_id
            .as_str()
            .ok_or_else(|| "profiles keys must be strings".to_string())?;
        let profile = match profile {
            Value::Mapping(profile) => profile,
            _ => return Err(format!("profiles.{} must be a mapping", profile_id)),
        };

        let trudge = require_non_empty_string(
            profile,
            "trudge",
            &format!("profiles.{}.trudge", profile_id),
        )?;
        let trudge_review = require_non_empty_string(
            profile,
            "trudge_review",
            &format!("profiles.{}.trudge_review", profile_id),
        )?;

        if !invocations.contains_key(Value::String(trudge.clone())) {
            return Err(format!(
                "profiles.{}.trudge references missing invocation: {}",
                profile_id, trudge
            ));
        }
        if !invocations.contains_key(Value::String(trudge_review.clone())) {
            return Err(format!(
                "profiles.{}.trudge_review references missing invocation: {}",
                profile_id, trudge_review
            ));
        }
    }

    if !profiles.contains_key(Value::String(default_profile.clone())) {
        return Err(format!(
            "default_profile references missing profile: {}",
            default_profile
        ));
    }

    require_non_null(mapping, "review_loop_limit", "review_loop_limit")?;
    validate_optional_string(mapping, "log_path", "log_path")?;

    let commands = require_mapping(mapping, "commands", "commands")?;
    let _ = require_non_empty_string(commands, "task_show", "commands.task_show")?;
    let _ = require_non_empty_string(commands, "task_status", "commands.task_status")?;
    let _ = require_non_empty_string(
        commands,
        "task_update_status",
        "commands.task_update_status",
    )?;

    let hooks = require_mapping(mapping, "hooks", "hooks")?;
    let _ = require_non_empty_string(hooks, "on_completed", "hooks.on_completed")?;
    let _ = require_non_empty_string(hooks, "on_requires_human", "hooks.on_requires_human")?;
    validate_optional_non_empty_string(hooks, "on_doctor_setup", "hooks.on_doctor_setup")?;
    validate_optional_non_empty_string(hooks, "on_notification", "hooks.on_notification")?;
    validate_optional_notification_scope(
        hooks,
        "on_notification_scope",
        "hooks.on_notification_scope",
    )?;

    Ok(())
}

fn notification_scope_without_hook_warning(mapping: &Mapping) -> Option<String> {
    let hooks_key = Value::String("hooks".to_string());
    let Some(Value::Mapping(hooks)) = mapping.get(&hooks_key) else {
        return None;
    };

    let hook_key = Value::String("on_notification".to_string());
    let scope_key = Value::String("on_notification_scope".to_string());
    if hooks.contains_key(&scope_key) && !hooks.contains_key(&hook_key) {
        return Some(
            "hooks.on_notification_scope is ignored because hooks.on_notification is not configured."
                .to_string(),
        );
    }

    None
}

fn reject_deprecated_keys(mapping: &Mapping) -> Result<(), String> {
    let legacy_keys = ["agent_command", "agent_review_command", "codex_command"];
    for key in legacy_keys {
        let key = Value::String(key.to_string());
        if mapping.contains_key(&key) {
            return Err(format!(
                "Migration: {} is no longer supported; use default_profile, profiles, and invocations.",
                key.as_str().unwrap()
            ));
        }
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

fn require_non_empty_string(
    mapping: &Mapping,
    key_name: &str,
    label: &str,
) -> Result<String, String> {
    let key = Value::String(key_name.to_string());
    match mapping.get(&key) {
        None => Err(format!("Missing required config value: {}", label)),
        Some(Value::Null) => Err(format!("{} must not be null", label)),
        Some(Value::String(value)) => {
            if value.trim().is_empty() {
                Err(format!("{} must not be empty", label))
            } else {
                Ok(value.clone())
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

fn validate_optional_notification_scope(
    mapping: &Mapping,
    key_name: &str,
    label: &str,
) -> Result<(), String> {
    let key = Value::String(key_name.to_string());
    let allowed = ["all_logs", "task_boundaries", "run_boundaries"];
    match mapping.get(&key) {
        None => Ok(()),
        Some(Value::Null) => Err(format!("{} must not be null", label)),
        Some(Value::String(value)) => {
            if allowed.contains(&value.as_str()) {
                Ok(())
            } else {
                Err(format!(
                    "{} must be one of all_logs|task_boundaries|run_boundaries",
                    label
                ))
            }
        }
        Some(_) => Err(format!(
            "{} must be one of all_logs|task_boundaries|run_boundaries",
            label
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::IntoDeserializer;
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
    fn deserialize_log_path_rejects_non_string_values() {
        let value: serde_yaml::Value = serde_yaml::from_str("123").expect("value");
        let err = deserialize_log_path(value.into_deserializer()).expect_err("expected error");
        assert!(err.to_string().contains("string"));
    }

    #[test]
    fn deserialize_log_path_error_path_is_covered_for_config_deserializer() {
        // Exercise the `String::deserialize(..)?` error path for the specific deserializer
        // instantiation used by `load_config_from_str` (via `serde_path_to_error`).
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
log_path: [123]
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;

        let deserializer = serde_yaml::Deserializer::from_str(config);
        let err = serde_path_to_error::deserialize::<_, ParsedConfig>(deserializer)
            .expect_err("expected log_path deserialization error");
        assert!(err.to_string().contains("log_path"));
    }

    #[test]
    fn missing_required_profile_schema_keys() {
        let config = r#"
default_profile: codex
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing required profile");
        assert!(err.contains("profiles"));

        let config = r#"
default_profile: codex
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing profiles");
        assert!(err.contains("profiles"));
    }

    #[test]
    fn legacy_agent_keys_are_rejected_with_migration_error() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
agent_command: "agent"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected legacy agent key rejection");
        assert!(err.contains("agent_command"));
        assert!(err.contains("Migration"));
    }

    #[test]
    fn missing_default_profile_reference_is_rejected() {
        let config = r#"
default_profile: missing
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing default profile reference");
        assert!(err.contains("default_profile references missing profile"));
    }

    #[test]
    fn load_config_uses_default_profile_when_no_override_is_provided() {
        let config = r#"
default_profile: fast
profiles:
  fast:
    trudge: fast-agent
    trudge_review: fast-review
  review:
    trudge: review-agent
    trudge_review: review-review
invocations:
  fast-agent:
    command: "agent-fast"
  fast-review:
    command: "review-fast"
  review-agent:
    command: "agent-review"
  review-review:
    command: "review-review"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_status: "task-update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let loaded = load_config(file.path()).expect("load config");
        assert_eq!(loaded.config.agent_command, "agent-fast");
        assert_eq!(loaded.config.agent_review_command, "review-fast");
    }

    #[test]
    fn load_config_with_profile_override_selects_profile() {
        let config = r#"
default_profile: fast
profiles:
  fast:
    trudge: fast-agent
    trudge_review: fast-review
  review:
    trudge: review-agent
    trudge_review: review-review
invocations:
  fast-agent:
    command: "agent-fast"
  fast-review:
    command: "review-fast"
  review-agent:
    command: "agent-review"
  review-review:
    command: "review-review"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_status: "task-update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let loaded = load_config_with_profile(file.path(), Some("review")).expect("load profile");
        assert_eq!(loaded.config.agent_command, "agent-review");
        assert_eq!(loaded.config.agent_review_command, "review-review");
    }

    #[test]
    fn load_config_with_unknown_profile_is_rejected() {
        let config = r#"
default_profile: fast
profiles:
  fast:
    trudge: fast-agent
    trudge_review: fast-review
invocations:
  fast-agent:
    command: "agent-fast"
  fast-review:
    command: "review-fast"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_status: "task-update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config_with_profile(file.path(), Some("missing")).expect_err("load profile");
        assert!(err.contains("Unknown profile: missing"));
    }

    #[test]
    fn profile_missing_invocation_reference_is_rejected() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review-missing
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing invocation reference");
        assert!(err.contains("profiles.codex.trudge_review references missing invocation"));
    }

    #[test]
    fn missing_required_mapping_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing commands mapping");
        assert!(err.contains("commands"));

        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
review_loop_limit: 3
commands:
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing hooks mapping");
        assert!(err.contains("hooks"));
    }

    #[test]
    fn null_required_value_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: null
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected null invocation command");
        assert!(err.contains("invocations.codex.command"));
    }

    #[test]
    fn empty_and_wrong_type_required_value_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: ""
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected empty invocation command");
        assert!(err.contains("invocations.codex.command"));

        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: ["agent"]
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected invocation command type error");
        assert!(err.contains("invocations.codex.command"));
        assert!(err.contains("string"));
    }

    #[test]
    fn missing_review_loop_limit_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
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
    fn missing_task_update_status_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing task_update_status");
        assert!(err.contains("commands.task_update_status"));
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
    extra_profile_key: "mystery"
invocations:
  codex:
    command: "agent"
    extra_invocation_key: "mystery"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
                "hooks.extra_hook_key".to_string(),
                "profiles.codex.extra_profile_key".to_string(),
                "invocations.codex.extra_invocation_key".to_string()
            ]
        );
    }

    #[test]
    fn codex_command_is_rejected() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
codex_command: "codex"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
log_path: "./log"
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected codex_command error");
        assert!(err.contains("codex_command"));
        assert!(err.contains("default_profile"));
        assert!(err.contains("profiles"));
        assert!(err.contains("invocations"));
        assert!(err.contains("Migration"));
    }

    #[test]
    fn optional_doctor_setup_value_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
    fn optional_notification_value_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
  on_notification: null
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected null on_notification error");
        assert!(err.contains("hooks.on_notification"));

        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
  on_notification: ""
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected empty on_notification error");
        assert!(err.contains("hooks.on_notification"));
    }

    #[test]
    fn invalid_notification_scope_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
  on_notification: "notify"
  on_notification_scope: "bad_scope"
"#;
        let file = write_temp_config(config);
        let err =
            load_config(file.path()).expect_err("expected invalid on_notification_scope error");
        assert!(err.contains("hooks.on_notification_scope"));
        assert!(err.contains("all_logs|task_boundaries|run_boundaries"));
    }

    #[test]
    fn notification_scope_defaults_and_scope_without_hook_warns() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
  on_notification: "notify"
"#;
        let file = write_temp_config(config);
        let loaded = load_config(file.path()).expect("config should load");
        assert_eq!(
            loaded.config.hooks.effective_notification_scope(),
            Some(NotificationScope::TaskBoundaries)
        );

        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
  on_notification_scope: "run_boundaries"
"#;
        let file = write_temp_config(config);
        let loaded = load_config(file.path()).expect("config should load");
        let warning = loaded
            .warnings
            .iter()
            .find(|warning| warning.contains("hooks.on_notification_scope"))
            .expect("scope-without-hook warning");
        assert!(warning.contains("ignored"));
        assert_eq!(loaded.config.hooks.effective_notification_scope(), None);
    }

    #[test]
    fn optional_log_path_value_errors() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
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
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing task_show");
        assert!(err.contains("commands.task_show"));

        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing task_status");
        assert!(err.contains("commands.task_status"));

        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
review_loop_limit: 3
hooks:
  on_completed: "done"
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing task_update_status");
        assert!(err.contains("commands.task_update_status"));
    }

    #[test]
    fn missing_hook_field_errors_are_specific() {
        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_requires_human: "human"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing on_completed");
        assert!(err.contains("hooks.on_completed"));

        let config = r#"
default_profile: codex
profiles:
  codex:
    trudge: codex
    trudge_review: codex-review
invocations:
  codex:
    command: "agent"
  codex-review:
    command: "review"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_status: "update"
review_loop_limit: 3
hooks:
  on_completed: "done"
"#;
        let file = write_temp_config(config);
        let err = load_config(file.path()).expect_err("expected missing on_requires_human");
        assert!(err.contains("hooks.on_requires_human"));
    }
}
