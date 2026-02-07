use chrono::Utc;
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::load_config_from_str;
use crate::run_loop::validate_config;
use crate::wizard_templates::{load_embedded_wizard_templates, AgentTemplate, TrackingTemplate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WizardResult {
    pub(crate) config_path: PathBuf,
    pub(crate) backup_path: Option<PathBuf>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WizardConfigOut {
    agent_command: String,
    agent_review_command: String,
    commands: WizardCommandsOut,
    hooks: WizardHooksOut,
    review_loop_limit: u64,
    log_path: String,
}

#[derive(Debug, Clone, Serialize)]
struct WizardCommandsOut {
    next_task: String,
    task_show: String,
    task_status: String,
    task_update_in_progress: String,
    reset_task: String,
}

#[derive(Debug, Clone, Serialize)]
struct WizardHooksOut {
    on_completed: String,
    on_requires_human: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    on_doctor_setup: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ExistingDefaults {
    review_loop_limit: Option<u64>,
    log_path: Option<String>,
}

pub(crate) fn run_wizard_interactive(config_path: &Path) -> Result<WizardResult, String> {
    let templates = load_embedded_wizard_templates()?;

    let agent_id = prompt_template_choice(
        "Select agent template",
        templates
            .agents
            .iter()
            .map(|t| format!("{}: {} ({})", t.id, t.label, t.description))
            .collect(),
    )?;
    let tracking_id = prompt_template_choice(
        "Select tracking template",
        templates
            .tracking
            .iter()
            .map(|t| format!("{}: {} ({})", t.id, t.label, t.description))
            .collect(),
    )?;

    run_wizard_selected(config_path, &agent_id, &tracking_id)
}

pub(crate) fn run_wizard_selected(
    config_path: &Path,
    agent_id: &str,
    tracking_id: &str,
) -> Result<WizardResult, String> {
    let templates = load_embedded_wizard_templates()?;

    let agent = find_agent_template(&templates.agents, agent_id)?;
    let tracking = find_tracking_template(&templates.tracking, tracking_id)?;

    let (existing_defaults, warnings) = read_existing_defaults(config_path)?;

    let review_loop_limit = existing_defaults
        .review_loop_limit
        .filter(|value| *value > 0)
        .unwrap_or(templates.defaults.review_loop_limit);
    let log_path = existing_defaults
        .log_path
        .unwrap_or_else(|| templates.defaults.log_path.clone());

    let candidate = WizardConfigOut {
        agent_command: agent.agent_command.clone(),
        agent_review_command: agent.agent_review_command.clone(),
        commands: WizardCommandsOut {
            next_task: tracking.commands.next_task.clone(),
            task_show: tracking.commands.task_show.clone(),
            task_status: tracking.commands.task_status.clone(),
            task_update_in_progress: tracking.commands.task_update_in_progress.clone(),
            reset_task: tracking.commands.reset_task.clone(),
        },
        hooks: WizardHooksOut {
            on_completed: tracking.hooks.on_completed.clone(),
            on_requires_human: tracking.hooks.on_requires_human.clone(),
            on_doctor_setup: tracking.hooks.on_doctor_setup.clone(),
        },
        review_loop_limit,
        log_path,
    };

    let yaml = serde_yaml::to_string(&candidate)
        .map_err(|err| format!("Failed to render generated config as YAML: {}", err))?;

    let (backup_path, write_warnings) = validate_then_write_config(config_path, &yaml)?;

    let mut all_warnings = warnings;
    all_warnings.extend(write_warnings);

    Ok(WizardResult {
        config_path: config_path.to_path_buf(),
        backup_path,
        warnings: all_warnings,
    })
}

fn validate_generated_config(yaml: &str) -> Result<(), String> {
    let loaded = load_config_from_str("<generated>", yaml)?;
    validate_config(&loaded.config, &[])?;
    Ok(())
}

fn validate_then_write_config(
    config_path: &Path,
    content: &str,
) -> Result<(Option<PathBuf>, Vec<String>), String> {
    validate_generated_config(content)?;
    write_config_with_backup(config_path, content)
}

fn write_config_with_backup(
    config_path: &Path,
    content: &str,
) -> Result<(Option<PathBuf>, Vec<String>), String> {
    // Create parent directory only after validation succeeded to avoid side effects on failure.
    if let Some(parent) = config_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "Failed to create config parent directory {}: {}",
                    parent.display(),
                    err
                )
            })?;
        }
    }

    let backup_path = if config_path.exists() {
        let backup_path = next_backup_path(config_path);
        fs::copy(config_path, &backup_path)
            .map_err(|err| format!("Failed to create backup {}: {}", backup_path.display(), err))?;
        Some(backup_path)
    } else {
        None
    };

    fs::write(config_path, content)
        .map_err(|err| format!("Failed to write config {}: {}", config_path.display(), err))?;

    Ok((backup_path, Vec::new()))
}

fn read_existing_defaults(config_path: &Path) -> Result<(ExistingDefaults, Vec<String>), String> {
    if !config_path.is_file() {
        return Ok((ExistingDefaults::default(), Vec::new()));
    }

    let content = fs::read_to_string(config_path).map_err(|err| {
        format!(
            "Failed to read existing config {}: {}",
            config_path.display(),
            err
        )
    })?;

    let value: Value = match serde_yaml::from_str(&content) {
        Ok(value) => value,
        Err(err) => {
            return Ok((
                ExistingDefaults::default(),
                vec![format!(
                    "Warning: Existing config {} could not be parsed as YAML and will be backed up and overwritten: {}",
                    config_path.display(),
                    err
                )],
            ));
        }
    };

    let mapping = match value {
        Value::Mapping(mapping) => mapping,
        _ => {
            return Ok((
                ExistingDefaults::default(),
                vec![format!(
                    "Warning: Existing config {} is not a YAML mapping and will be backed up and overwritten.",
                    config_path.display()
                )],
            ));
        }
    };

    Ok((extract_existing_defaults(&mapping), Vec::new()))
}

fn extract_existing_defaults(mapping: &Mapping) -> ExistingDefaults {
    let review_loop_limit = mapping
        .get(Value::String("review_loop_limit".to_string()))
        .and_then(|value| value.as_u64());
    let log_path = mapping
        .get(Value::String("log_path".to_string()))
        .and_then(|value| value.as_str().map(|s| s.to_string()));

    ExistingDefaults {
        review_loop_limit,
        log_path,
    }
}

fn prompt_template_choice(title: &str, options: Vec<String>) -> Result<String, String> {
    if options.is_empty() {
        return Err(format!("No choices available for {}.", title));
    }

    loop {
        println!("{}\n", title);
        for (index, option) in options.iter().enumerate() {
            println!("  {}) {}", index + 1, option);
        }
        print!("\nEnter number or id: ");
        io::stdout()
            .flush()
            .map_err(|err| format!("Failed to flush stdout: {}", err))?;

        let mut input = String::new();
        let bytes = io::stdin()
            .read_line(&mut input)
            .map_err(|err| format!("Failed to read selection: {}", err))?;
        if bytes == 0 {
            return Err("Wizard aborted (stdin closed).".to_string());
        }
        let trimmed = input.trim();
        if trimmed.is_empty() {
            eprintln!("Selection must not be empty.\n");
            continue;
        }
        if let Ok(choice) = trimmed.parse::<usize>() {
            if choice >= 1 && choice <= options.len() {
                let option = &options[choice - 1];
                let id = option.split(':').next().unwrap_or(option).trim();
                return Ok(id.to_string());
            }
            eprintln!("Selection out of range.\n");
            continue;
        }

        // Assume user entered the ID directly.
        return Ok(trimmed.to_string());
    }
}

fn find_agent_template<'a>(
    agents: &'a [AgentTemplate],
    id: &str,
) -> Result<&'a AgentTemplate, String> {
    agents
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("Unknown agent template id: {}", id))
}

fn find_tracking_template<'a>(
    tracking: &'a [TrackingTemplate],
    id: &str,
) -> Result<&'a TrackingTemplate, String> {
    tracking
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("Unknown tracking template id: {}", id))
}

fn next_backup_path(config_path: &Path) -> PathBuf {
    let file_name = config_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("trudger.yml");
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut backup = config_path.with_file_name(format!("{}.bak-{}", file_name, timestamp));

    if !backup.exists() {
        return backup;
    }

    for index in 2..=1000 {
        backup = config_path.with_file_name(format!("{}.bak-{}-{}", file_name, timestamp, index));
        if !backup.exists() {
            return backup;
        }
    }

    // If we've somehow exhausted all suffixes, just return the last candidate.
    backup
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn list_backups(dir: &Path, base: &str) -> Vec<PathBuf> {
        let prefix = format!("{}.bak-", base);
        let mut backups: Vec<PathBuf> = fs::read_dir(dir)
            .expect("read_dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with(&prefix))
                    .unwrap_or(false)
            })
            .collect();
        backups.sort();
        backups
    }

    #[test]
    fn validation_failure_prevents_write_and_backup() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        fs::write(&config_path, "old").expect("write existing config");

        let invalid = r#"
agent_command: ""
agent_review_command: "x"
review_loop_limit: 1
log_path: "./log"
commands:
  next_task: "x"
  task_show: "x"
  task_status: "x"
  task_update_in_progress: "x"
  reset_task: "x"
hooks:
  on_completed: "x"
  on_requires_human: "x"
"#;

        let err = validate_then_write_config(&config_path, invalid).expect_err("expected error");
        assert!(err.contains("agent_command"), "err: {err}");

        // Ensure we didn't touch the existing file.
        let contents = fs::read_to_string(&config_path).expect("read config");
        assert_eq!(contents, "old");
        assert!(list_backups(temp.path(), "trudger.yml").is_empty());
    }

    #[test]
    fn creates_parent_directory_and_writes_config() {
        let temp = TempDir::new().expect("temp dir");
        let nested = temp.path().join("missing-parent");
        let config_path = nested.join("trudger.yml");

        let result = run_wizard_selected(&config_path, "codex", "br-next-task").expect("wizard");
        assert_eq!(result.config_path, config_path);
        assert!(nested.is_dir(), "expected parent dir created");
        assert!(config_path.is_file(), "expected config written");
        assert!(result.backup_path.is_none());
    }

    #[test]
    fn overwrite_creates_timestamped_backup() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        let original = r#"
agent_command: "agent"
agent_review_command: "review"
review_loop_limit: 3
log_path: "./log"
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
hooks:
  on_completed: "done"
  on_requires_human: "human"
  on_doctor_setup: "setup"
"#;
        fs::write(&config_path, original).expect("write existing config");

        let result = run_wizard_selected(&config_path, "codex", "br-next-task").expect("wizard");

        let backups = list_backups(temp.path(), "trudger.yml");
        assert_eq!(backups.len(), 1, "expected one backup, got {backups:?}");
        assert_eq!(result.backup_path.as_ref(), Some(&backups[0]));
        let backup_contents = fs::read_to_string(&backups[0]).expect("read backup");
        assert_eq!(backup_contents, original);

        let new_contents = fs::read_to_string(&config_path).expect("read new config");
        assert_ne!(new_contents, original, "expected config overwritten");

        // Sanity check: output must be loadable by current config parser.
        let _ = load_config_from_str("<test>", &new_contents).expect("load config");
    }

    #[test]
    fn invalid_yaml_existing_config_warns_and_is_backed_up() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        let original = ":\n  - invalid";
        fs::write(&config_path, original).expect("write invalid yaml");

        let result = run_wizard_selected(&config_path, "codex", "br-next-task").expect("wizard");
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("could not be parsed as YAML")),
            "expected invalid-YAML warning, got: {:?}",
            result.warnings
        );

        let backups = list_backups(temp.path(), "trudger.yml");
        assert_eq!(backups.len(), 1, "expected one backup, got {backups:?}");
        let backup_contents = fs::read_to_string(&backups[0]).expect("read backup");
        assert_eq!(backup_contents, original);
        assert!(config_path.is_file(), "expected config overwritten");
    }
}
