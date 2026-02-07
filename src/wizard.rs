use chrono::Utc;
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::load_config_from_str;
use crate::run_loop::validate_config;
use crate::wizard_templates::{
    load_embedded_wizard_templates, AgentTemplate, TrackingTemplate, WizardTemplates,
};

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

#[derive(Debug, Clone)]
struct ExistingConfig {
    mapping: Option<Mapping>,
    defaults: ExistingDefaults,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct ExistingDefaults {
    review_loop_limit: Option<u64>,
    log_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeDecision {
    KeepCurrent,
    ReplaceWithProposed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WizardMergeMode {
    Interactive,
    #[cfg(test)]
    Overwrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MergePrompt {
    key: String,
    current: Option<Value>,
    proposed: Option<Value>,
}

pub(crate) fn run_wizard_interactive(config_path: &Path) -> Result<WizardResult, String> {
    let templates = load_embedded_wizard_templates()?;

    let existing = read_existing_config(config_path)?;
    let default_agent_id = existing
        .mapping
        .as_ref()
        .map(|mapping| best_matching_agent_template_id(&templates.agents, mapping));
    let default_tracking_id = existing
        .mapping
        .as_ref()
        .map(|mapping| best_matching_tracking_template_id(&templates.tracking, mapping));

    let agent_id = prompt_template_choice(
        "Select agent template",
        templates
            .agents
            .iter()
            .map(|t| format!("{}: {} ({})", t.id, t.label, t.description))
            .collect(),
        default_agent_id.as_deref(),
    )?;
    let tracking_id = prompt_template_choice(
        "Select tracking template",
        templates
            .tracking
            .iter()
            .map(|t| format!("{}: {} ({})", t.id, t.label, t.description))
            .collect(),
        default_tracking_id.as_deref(),
    )?;

    run_wizard_selected_with_existing(
        config_path,
        &templates,
        existing,
        &agent_id,
        &tracking_id,
        WizardMergeMode::Interactive,
    )
}

#[cfg(test)]
pub(crate) fn run_wizard_selected(
    config_path: &Path,
    agent_id: &str,
    tracking_id: &str,
) -> Result<WizardResult, String> {
    let templates = load_embedded_wizard_templates()?;
    let existing = read_existing_config(config_path)?;
    run_wizard_selected_with_existing(
        config_path,
        &templates,
        existing,
        agent_id,
        tracking_id,
        WizardMergeMode::Overwrite,
    )
}

fn run_wizard_selected_with_existing(
    config_path: &Path,
    templates: &WizardTemplates,
    existing: ExistingConfig,
    agent_id: &str,
    tracking_id: &str,
    merge_mode: WizardMergeMode,
) -> Result<WizardResult, String> {
    let agent = find_agent_template(&templates.agents, agent_id)?;
    let tracking = find_tracking_template(&templates.tracking, tracking_id)?;

    let review_loop_limit = existing
        .defaults
        .review_loop_limit
        .filter(|value| *value > 0)
        .unwrap_or(templates.defaults.review_loop_limit);
    let log_path = existing
        .defaults
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

    let mut candidate_value = serde_yaml::to_value(&candidate)
        .map_err(|err| format!("Failed to render generated config as YAML: {}", err))?;

    if merge_mode == WizardMergeMode::Interactive {
        if let Some(existing_mapping) = existing.mapping.as_ref() {
            let mut decider = prompt_merge_decision;
            merge_known_template_keys(existing_mapping, &mut candidate_value, &mut decider)?;
        }
    }

    let yaml = serde_yaml::to_string(&candidate_value)
        .map_err(|err| format!("Failed to render generated config as YAML: {}", err))?;

    let (backup_path, write_warnings) = validate_then_write_config(config_path, &yaml)?;

    let mut all_warnings = existing.warnings;
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

fn read_existing_config(config_path: &Path) -> Result<ExistingConfig, String> {
    if !config_path.is_file() {
        return Ok(ExistingConfig {
            mapping: None,
            defaults: ExistingDefaults::default(),
            warnings: Vec::new(),
        });
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
            return Ok(ExistingConfig {
                mapping: None,
                defaults: ExistingDefaults::default(),
                warnings: vec![format!(
                    "Warning: Existing config {} could not be parsed as YAML and will be backed up and overwritten: {}",
                    config_path.display(),
                    err
                )],
            });
        }
    };

    let mapping = match value {
        Value::Mapping(mapping) => mapping,
        _ => {
            return Ok(ExistingConfig {
                mapping: None,
                defaults: ExistingDefaults::default(),
                warnings: vec![format!(
                    "Warning: Existing config {} is not a YAML mapping and will be backed up and overwritten.",
                    config_path.display()
                )],
            });
        }
    };

    Ok(ExistingConfig {
        defaults: extract_existing_defaults(&mapping),
        mapping: Some(mapping),
        warnings: Vec::new(),
    })
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

fn get_mapping_value_at_path<'a>(mapping: &'a Mapping, path: &[&str]) -> Option<&'a Value> {
    if path.is_empty() {
        return None;
    }

    let mut current = mapping.get(&Value::String(path[0].to_string()))?;
    for segment in &path[1..] {
        let nested = current.as_mapping()?;
        current = nested.get(&Value::String((*segment).to_string()))?;
    }
    Some(current)
}

fn get_value_at_path<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    if path.is_empty() {
        return None;
    }

    let mut current = root;
    for segment in path {
        let nested = current.as_mapping()?;
        current = nested.get(&Value::String((*segment).to_string()))?;
    }
    Some(current)
}

fn remove_value_at_path(root: &mut Value, path: &[&str]) -> bool {
    if path.is_empty() {
        return false;
    }

    let mut current = root;
    for (index, segment) in path.iter().enumerate() {
        let is_last = index == path.len() - 1;
        let nested = match current.as_mapping_mut() {
            Some(mapping) => mapping,
            None => return false,
        };

        let key = Value::String((*segment).to_string());
        if is_last {
            return nested.remove(&key).is_some();
        }

        current = match nested.get_mut(&key) {
            Some(value) => value,
            None => return false,
        };
    }

    false
}

fn set_value_at_path(root: &mut Value, path: &[&str], value: Value) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }

    let mut current = root;
    for (index, segment) in path.iter().enumerate() {
        let is_last = index == path.len() - 1;
        let nested = current.as_mapping_mut().ok_or_else(|| {
            format!(
                "Internal wizard error: expected YAML mapping at {}",
                path[..index].join(".")
            )
        })?;

        let key = Value::String((*segment).to_string());
        if is_last {
            nested.insert(key, value);
            return Ok(());
        }

        if !nested.contains_key(&key) {
            nested.insert(key.clone(), Value::Mapping(Mapping::new()));
        }
        let next = nested.get_mut(&key).expect("key exists");
        if !matches!(next, Value::Mapping(_)) {
            *next = Value::Mapping(Mapping::new());
        }
        current = next;
    }

    Ok(())
}

fn get_string_value_at_path<'a>(mapping: &'a Mapping, path: &[&str]) -> Option<&'a str> {
    get_mapping_value_at_path(mapping, path).and_then(|value| value.as_str())
}

fn best_matching_agent_template_id(templates: &[AgentTemplate], existing: &Mapping) -> String {
    let existing_agent = get_string_value_at_path(existing, &["agent_command"]);
    let existing_review = get_string_value_at_path(existing, &["agent_review_command"]);

    let mut best_id = templates
        .first()
        .map(|template| template.id.clone())
        .unwrap_or_default();
    let mut best_score: i32 = -1;

    for template in templates {
        let mut score = 0;
        if existing_agent.is_some_and(|value| value == template.agent_command) {
            score += 1;
        }
        if existing_review.is_some_and(|value| value == template.agent_review_command) {
            score += 1;
        }

        if score > best_score {
            best_score = score;
            best_id = template.id.clone();
        }
    }

    best_id
}

fn best_matching_tracking_template_id(templates: &[TrackingTemplate], existing: &Mapping) -> String {
    let existing_next_task = get_string_value_at_path(existing, &["commands", "next_task"]);
    let existing_task_show = get_string_value_at_path(existing, &["commands", "task_show"]);
    let existing_task_status = get_string_value_at_path(existing, &["commands", "task_status"]);
    let existing_task_update_in_progress =
        get_string_value_at_path(existing, &["commands", "task_update_in_progress"]);
    let existing_reset_task = get_string_value_at_path(existing, &["commands", "reset_task"]);
    let existing_on_completed = get_string_value_at_path(existing, &["hooks", "on_completed"]);
    let existing_on_requires_human =
        get_string_value_at_path(existing, &["hooks", "on_requires_human"]);
    let existing_on_doctor_setup = get_string_value_at_path(existing, &["hooks", "on_doctor_setup"]);

    let mut best_id = templates
        .first()
        .map(|template| template.id.clone())
        .unwrap_or_default();
    let mut best_score: i32 = -1;

    for template in templates {
        let mut score = 0;
        if existing_next_task.is_some_and(|value| value == template.commands.next_task) {
            score += 1;
        }
        if existing_task_show.is_some_and(|value| value == template.commands.task_show) {
            score += 1;
        }
        if existing_task_status.is_some_and(|value| value == template.commands.task_status) {
            score += 1;
        }
        if existing_task_update_in_progress
            .is_some_and(|value| value == template.commands.task_update_in_progress)
        {
            score += 1;
        }
        if existing_reset_task.is_some_and(|value| value == template.commands.reset_task) {
            score += 1;
        }
        if existing_on_completed.is_some_and(|value| value == template.hooks.on_completed) {
            score += 1;
        }
        if existing_on_requires_human.is_some_and(|value| value == template.hooks.on_requires_human)
        {
            score += 1;
        }
        if existing_on_doctor_setup.is_some_and(|value| {
            template
                .hooks
                .on_doctor_setup
                .as_ref()
                .is_some_and(|candidate| value == candidate)
        }) {
            score += 1;
        }

        if score > best_score {
            best_score = score;
            best_id = template.id.clone();
        }
    }

    best_id
}

fn merge_known_template_keys(
    existing: &Mapping,
    candidate: &mut Value,
    prompt: &mut dyn FnMut(&MergePrompt) -> Result<MergeDecision, String>,
) -> Result<Vec<MergePrompt>, String> {
    const KNOWN_TEMPLATE_KEYS: &[&[&str]] = &[
        &["agent_command"],
        &["agent_review_command"],
        &["commands", "next_task"],
        &["commands", "task_show"],
        &["commands", "task_status"],
        &["commands", "task_update_in_progress"],
        &["commands", "reset_task"],
        &["hooks", "on_completed"],
        &["hooks", "on_requires_human"],
        &["hooks", "on_doctor_setup"],
    ];

    let mut prompts = Vec::new();
    for key_path in KNOWN_TEMPLATE_KEYS {
        let current = get_mapping_value_at_path(existing, key_path).cloned();
        let proposed = get_value_at_path(candidate, key_path).cloned();
        if current == proposed {
            continue;
        }

        let merge_prompt = MergePrompt {
            key: key_path.join("."),
            current,
            proposed,
        };

        let decision = prompt(&merge_prompt)?;
        if decision == MergeDecision::KeepCurrent {
            match merge_prompt.current.clone() {
                Some(value) => set_value_at_path(candidate, key_path, value)?,
                None => {
                    remove_value_at_path(candidate, key_path);
                }
            }
        }

        prompts.push(merge_prompt);
    }

    Ok(prompts)
}

fn format_yaml_value(value: &Option<Value>) -> String {
    match value {
        None => "<missing>".to_string(),
        Some(value) => serde_yaml::to_string(value)
            .map(|rendered| rendered.trim_end().to_string())
            .unwrap_or_else(|_| format!("{:?}", value)),
    }
}

fn parse_merge_decision(input: &str) -> Option<MergeDecision> {
    match input.trim().to_ascii_lowercase().as_str() {
        "" | "k" | "keep" | "y" | "yes" => Some(MergeDecision::KeepCurrent),
        "r" | "replace" | "n" | "no" => Some(MergeDecision::ReplaceWithProposed),
        _ => None,
    }
}

fn prompt_merge_decision(prompt: &MergePrompt) -> Result<MergeDecision, String> {
    println!("\nKey: {}", prompt.key);
    println!("Current: {}", format_yaml_value(&prompt.current));
    println!("Proposed: {}", format_yaml_value(&prompt.proposed));

    loop {
        print!("Keep current or replace with proposed? [K/r] (default K): ");
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

        if let Some(decision) = parse_merge_decision(&input) {
            return Ok(decision);
        }

        eprintln!("Please enter 'k' to keep current or 'r' to replace.\n");
    }
}

fn prompt_template_choice(
    title: &str,
    options: Vec<String>,
    default_id: Option<&str>,
) -> Result<String, String> {
    if options.is_empty() {
        return Err(format!("No choices available for {}.", title));
    }

    loop {
        println!("{}\n", title);
        for (index, option) in options.iter().enumerate() {
            let id = option.split(':').next().unwrap_or(option).trim();
            let default_marker = if default_id.is_some_and(|value| value == id) {
                " (default)"
            } else {
                ""
            };
            println!("  {}) {}{}", index + 1, option, default_marker);
        }
        if default_id.is_some() {
            print!("\nEnter number or id (blank for default): ");
        } else {
            print!("\nEnter number or id: ");
        }
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
            if let Some(id) = default_id {
                return Ok(id.to_string());
            }
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

    #[test]
    fn merge_prompts_only_for_known_keys_that_differ() {
        let templates = load_embedded_wizard_templates().expect("templates");
        let agent = find_agent_template(&templates.agents, "codex").expect("agent");
        let tracking = find_tracking_template(&templates.tracking, "br-next-task").expect("tracking");

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
            review_loop_limit: templates.defaults.review_loop_limit,
            log_path: templates.defaults.log_path.clone(),
        };

        let mut candidate_value = serde_yaml::to_value(&candidate).expect("candidate yaml");
        let mut existing_value = candidate_value.clone();
        set_value_at_path(
            &mut existing_value,
            &["commands", "reset_task"],
            Value::String("different".to_string()),
        )
        .expect("set existing value");
        let existing_mapping = existing_value
            .as_mapping()
            .expect("mapping")
            .clone();

        let mut prompted_keys = Vec::new();
        let mut decider = |prompt: &MergePrompt| {
            prompted_keys.push(prompt.key.clone());
            Ok(MergeDecision::ReplaceWithProposed)
        };

        let prompts =
            merge_known_template_keys(&existing_mapping, &mut candidate_value, &mut decider)
                .expect("merge");

        assert_eq!(prompts.len(), 1, "prompts: {prompts:?}");
        assert_eq!(prompts[0].key, "commands.reset_task");
        assert_eq!(prompted_keys, vec!["commands.reset_task"]);
    }

    #[test]
    fn merge_keep_current_overrides_candidate_for_hooks_on_doctor_setup() {
        let templates = load_embedded_wizard_templates().expect("templates");
        let agent = find_agent_template(&templates.agents, "codex").expect("agent");
        let tracking = find_tracking_template(&templates.tracking, "br-next-task").expect("tracking");

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
            review_loop_limit: templates.defaults.review_loop_limit,
            log_path: templates.defaults.log_path.clone(),
        };

        let mut candidate_value = serde_yaml::to_value(&candidate).expect("candidate yaml");
        let mut existing_value = candidate_value.clone();
        set_value_at_path(
            &mut existing_value,
            &["hooks", "on_doctor_setup"],
            Value::String("existing".to_string()),
        )
        .expect("set existing value");
        let existing_mapping = existing_value
            .as_mapping()
            .expect("mapping")
            .clone();

        let mut decider = |prompt: &MergePrompt| {
            assert_eq!(prompt.key, "hooks.on_doctor_setup");
            Ok(MergeDecision::KeepCurrent)
        };

        let prompts =
            merge_known_template_keys(&existing_mapping, &mut candidate_value, &mut decider)
                .expect("merge");

        assert_eq!(prompts.len(), 1, "prompts: {prompts:?}");
        let merged = get_value_at_path(&candidate_value, &["hooks", "on_doctor_setup"])
            .and_then(|value| value.as_str())
            .expect("merged hooks.on_doctor_setup");
        assert_eq!(merged, "existing");
    }

    #[test]
    fn merge_decision_defaults_to_keep_current() {
        assert_eq!(
            parse_merge_decision(""),
            Some(MergeDecision::KeepCurrent),
            "expected empty input to keep current"
        );
    }

    #[test]
    fn best_match_picks_agent_and_tracking_templates_from_existing_config() {
        let templates = load_embedded_wizard_templates().expect("templates");

        let claude = find_agent_template(&templates.agents, "claude").expect("claude");
        let mut existing_agent = Mapping::new();
        existing_agent.insert(
            Value::String("agent_command".to_string()),
            Value::String(claude.agent_command.clone()),
        );
        existing_agent.insert(
            Value::String("agent_review_command".to_string()),
            Value::String(claude.agent_review_command.clone()),
        );
        assert_eq!(
            best_matching_agent_template_id(&templates.agents, &existing_agent),
            "claude"
        );

        let bd = find_tracking_template(&templates.tracking, "bd-labels").expect("bd-labels");
        let mut existing_tracking = Mapping::new();
        let mut commands = Mapping::new();
        commands.insert(
            Value::String("next_task".to_string()),
            Value::String(bd.commands.next_task.clone()),
        );
        commands.insert(
            Value::String("task_show".to_string()),
            Value::String(bd.commands.task_show.clone()),
        );
        commands.insert(
            Value::String("task_status".to_string()),
            Value::String(bd.commands.task_status.clone()),
        );
        commands.insert(
            Value::String("task_update_in_progress".to_string()),
            Value::String(bd.commands.task_update_in_progress.clone()),
        );
        commands.insert(
            Value::String("reset_task".to_string()),
            Value::String(bd.commands.reset_task.clone()),
        );
        existing_tracking.insert(
            Value::String("commands".to_string()),
            Value::Mapping(commands),
        );

        let mut hooks = Mapping::new();
        hooks.insert(
            Value::String("on_completed".to_string()),
            Value::String(bd.hooks.on_completed.clone()),
        );
        hooks.insert(
            Value::String("on_requires_human".to_string()),
            Value::String(bd.hooks.on_requires_human.clone()),
        );
        if let Some(setup) = &bd.hooks.on_doctor_setup {
            hooks.insert(
                Value::String("on_doctor_setup".to_string()),
                Value::String(setup.clone()),
            );
        }
        existing_tracking.insert(Value::String("hooks".to_string()), Value::Mapping(hooks));

        assert_eq!(
            best_matching_tracking_template_id(&templates.tracking, &existing_tracking),
            "bd-labels"
        );
    }
}
