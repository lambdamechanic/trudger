use chrono::Utc;
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use std::path::{Path, PathBuf};

use crate::config::load_config_from_str;
use crate::run_loop::validate_config;
use crate::wizard_templates::{
    load_embedded_wizard_templates, AgentTemplate, TrackingTemplate, WizardTemplates,
};

mod fs;
mod io;

use io::{TerminalWizardIo, WizardIo};

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
    let mut io = TerminalWizardIo::new();
    run_wizard_with_io(
        config_path,
        &templates,
        WizardMergeMode::Interactive,
        &mut io,
    )
}

fn run_wizard_with_io(
    config_path: &Path,
    templates: &WizardTemplates,
    merge_mode: WizardMergeMode,
    io: &mut dyn WizardIo,
) -> Result<WizardResult, String> {
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
        io,
        "Select agent template",
        templates
            .agents
            .iter()
            .map(|t| format!("{}: {} ({})", t.id, t.label, t.description))
            .collect(),
        default_agent_id.as_deref(),
    )?;
    let tracking_id = prompt_template_choice(
        io,
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
        templates,
        existing,
        &agent_id,
        &tracking_id,
        merge_mode,
        io,
    )
}

fn run_wizard_selected_with_existing(
    config_path: &Path,
    templates: &WizardTemplates,
    existing: ExistingConfig,
    agent_id: &str,
    tracking_id: &str,
    merge_mode: WizardMergeMode,
    io: &mut dyn WizardIo,
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
            let mut decider = |prompt: &MergePrompt| prompt_merge_decision(io, prompt);
            merge_known_template_keys(existing_mapping, &mut candidate_value, &mut decider)?;
        }
    }

    let mut yaml = serde_yaml::to_string(&candidate_value)
        .map_err(|err| format!("Failed to render generated config as YAML: {}", err))?;

    let mut unknown_keys_warning: Option<String> = None;
    if let Some(existing_mapping) = existing.mapping.as_ref() {
        if let Some((unknown_block, unknown_paths)) =
            render_unknown_keys_commented_block(existing_mapping)?
        {
            yaml.push_str(&unknown_block);
            unknown_keys_warning = Some(format!(
                "Warning: Unknown/custom config keys were commented out and appended to the generated config: {}",
                unknown_paths.join(", ")
            ));
        }
    }

    let (backup_path, write_warnings) = validate_then_write_config(config_path, &yaml)?;

    let mut all_warnings = existing.warnings;
    if let Some(warning) = unknown_keys_warning {
        all_warnings.push(warning);
    }
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
            fs::create_dir_all(parent)?;
        }
    }

    let backup_path = if fs::exists(config_path) {
        let backup_path = next_backup_path(config_path);
        fs::copy(config_path, &backup_path)?;
        Some(backup_path)
    } else {
        None
    };

    fs::write(config_path, content)?;

    Ok((backup_path, Vec::new()))
}

fn read_existing_config(config_path: &Path) -> Result<ExistingConfig, String> {
    if !fs::is_file(config_path) {
        return Ok(ExistingConfig {
            mapping: None,
            defaults: ExistingDefaults::default(),
            warnings: Vec::new(),
        });
    }

    let content = fs::read_to_string(config_path)?;

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

fn render_unknown_keys_commented_block(
    existing: &Mapping,
) -> Result<Option<(String, Vec<String>)>, String> {
    let (unknown_mapping, unknown_paths) = extract_unknown_key_values(existing);
    if unknown_mapping.is_empty() {
        return Ok(None);
    }

    let rendered = serde_yaml::to_string(&Value::Mapping(unknown_mapping))
        .map_err(|err| format!("Failed to render unknown keys as YAML: {}", err))?;
    let rendered = strip_yaml_document_prefix(&rendered);

    let mut block = String::new();
    block.push('\n');
    block.push_str(
        "# -----------------------------------------------------------------------------\n",
    );
    block.push_str(
        "# WARNING: Unknown/custom keys from your previous config were preserved below.\n",
    );
    block.push_str("# They are commented out so the generated config remains valid; you may\n");
    block.push_str("# restore them manually if needed.\n");
    block.push_str(
        "# -----------------------------------------------------------------------------\n",
    );
    block.push_str(&comment_out_yaml_lines(&rendered));

    Ok(Some((block, unknown_paths)))
}

fn strip_yaml_document_prefix(rendered: &str) -> String {
    let Some(stripped) = rendered.strip_prefix("---\n") else {
        return rendered.to_string();
    };
    stripped.to_string()
}

fn comment_out_yaml_lines(rendered: &str) -> String {
    let mut out = String::new();
    for line in rendered.lines() {
        if line.trim().is_empty() {
            out.push_str("#\n");
            continue;
        }
        out.push_str("# ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn extract_unknown_key_values(existing: &Mapping) -> (Mapping, Vec<String>) {
    const ALLOWED_TOP_LEVEL: &[&str] = &[
        "agent_command",
        "agent_review_command",
        "commands",
        "hooks",
        "review_loop_limit",
        "log_path",
    ];
    const ALLOWED_COMMANDS: &[&str] = &[
        "next_task",
        "task_show",
        "task_status",
        "task_update_in_progress",
        "reset_task",
    ];
    const ALLOWED_HOOKS: &[&str] = &["on_completed", "on_requires_human", "on_doctor_setup"];

    let mut out = Mapping::new();
    let mut unknown_paths: Vec<String> = Vec::new();

    for (key, value) in existing {
        let Some(key_str) = key.as_str() else {
            continue;
        };
        if !ALLOWED_TOP_LEVEL.contains(&key_str) {
            out.insert(Value::String(key_str.to_string()), value.clone());
            unknown_paths.push(key_str.to_string());
        }
    }

    if let Some((commands, paths)) =
        extract_unknown_nested_mapping(existing, "commands", ALLOWED_COMMANDS)
    {
        out.insert(
            Value::String("commands".to_string()),
            Value::Mapping(commands),
        );
        unknown_paths.extend(paths);
    }

    if let Some((hooks, paths)) = extract_unknown_nested_mapping(existing, "hooks", ALLOWED_HOOKS) {
        out.insert(Value::String("hooks".to_string()), Value::Mapping(hooks));
        unknown_paths.extend(paths);
    }

    (out, unknown_paths)
}

fn extract_unknown_nested_mapping(
    existing: &Mapping,
    mapping_key: &str,
    allowed: &[&str],
) -> Option<(Mapping, Vec<String>)> {
    let Some(Value::Mapping(nested)) = existing.get(mapping_key) else {
        return None;
    };

    let mut out = Mapping::new();
    let mut unknown_paths: Vec<String> = Vec::new();

    for (key, value) in nested {
        let Some(key_str) = key.as_str() else {
            continue;
        };
        if !allowed.contains(&key_str) {
            out.insert(Value::String(key_str.to_string()), value.clone());
            unknown_paths.push(format!("{}.{}", mapping_key, key_str));
        }
    }

    if out.is_empty() {
        return None;
    }

    Some((out, unknown_paths))
}

fn get_mapping_value_at_path<'a>(mapping: &'a Mapping, path: &[&str]) -> Option<&'a Value> {
    if path.is_empty() {
        return None;
    }

    let mut current = mapping.get(path[0])?;
    for segment in &path[1..] {
        let nested = current.as_mapping()?;
        current = nested.get(*segment)?;
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
        current = nested.get(*segment)?;
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

        if is_last {
            return nested.remove(*segment).is_some();
        }

        current = match nested.get_mut(*segment) {
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

        let key = *segment;
        if is_last {
            nested.insert(Value::String(key.to_string()), value);
            return Ok(());
        }

        if !nested.contains_key(key) {
            nested.insert(
                Value::String(key.to_string()),
                Value::Mapping(Mapping::new()),
            );
        }
        let next = nested.get_mut(key).expect("key exists");
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

fn best_matching_tracking_template_id(
    templates: &[TrackingTemplate],
    existing: &Mapping,
) -> String {
    let existing_next_task = get_string_value_at_path(existing, &["commands", "next_task"]);
    let existing_task_show = get_string_value_at_path(existing, &["commands", "task_show"]);
    let existing_task_status = get_string_value_at_path(existing, &["commands", "task_status"]);
    let existing_task_update_in_progress =
        get_string_value_at_path(existing, &["commands", "task_update_in_progress"]);
    let existing_reset_task = get_string_value_at_path(existing, &["commands", "reset_task"]);
    let existing_on_completed = get_string_value_at_path(existing, &["hooks", "on_completed"]);
    let existing_on_requires_human =
        get_string_value_at_path(existing, &["hooks", "on_requires_human"]);
    let existing_on_doctor_setup =
        get_string_value_at_path(existing, &["hooks", "on_doctor_setup"]);

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

fn prompt_merge_decision(
    io: &mut dyn WizardIo,
    prompt: &MergePrompt,
) -> Result<MergeDecision, String> {
    io.write_out(&format!("\nKey: {}\n", prompt.key))?;
    io.write_out(&format!(
        "Current: {}\n",
        format_yaml_value(&prompt.current)
    ))?;
    io.write_out(&format!(
        "Proposed: {}\n",
        format_yaml_value(&prompt.proposed)
    ))?;

    loop {
        io.write_out("Keep current or replace with proposed? [K/r] (default K): ")?;
        io.flush_out()?;

        let Some(input) = io.read_line()? else {
            return Err("Wizard aborted (stdin closed).".to_string());
        };

        if let Some(decision) = parse_merge_decision(&input) {
            return Ok(decision);
        }

        io.write_err("Please enter 'k' to keep current or 'r' to replace.\n\n")?;
    }
}

fn prompt_template_choice(
    io: &mut dyn WizardIo,
    title: &str,
    options: Vec<String>,
    default_id: Option<&str>,
) -> Result<String, String> {
    if options.is_empty() {
        return Err(format!("No choices available for {}.", title));
    }

    loop {
        io.write_out(&format!("{}\n\n", title))?;
        for (index, option) in options.iter().enumerate() {
            let id = option.split(':').next().unwrap_or(option).trim();
            let default_marker = if default_id.is_some_and(|value| value == id) {
                " (default)"
            } else {
                ""
            };
            io.write_out(&format!("  {}) {}{}\n", index + 1, option, default_marker))?;
        }
        if default_id.is_some() {
            io.write_out("\nEnter number or id (blank for default): ")?;
        } else {
            io.write_out("\nEnter number or id: ")?;
        }
        io.flush_out()?;

        let Some(input) = io.read_line()? else {
            return Err("Wizard aborted (stdin closed).".to_string());
        };
        let trimmed = input.trim();
        if trimmed.is_empty() {
            if let Some(id) = default_id {
                return Ok(id.to_string());
            }
            io.write_err("Selection must not be empty.\n\n")?;
            continue;
        }
        if let Ok(choice) = trimmed.parse::<usize>() {
            if choice >= 1 && choice <= options.len() {
                let option = &options[choice - 1];
                let id = option.split(':').next().unwrap_or(option).trim();
                return Ok(id.to_string());
            }
            io.write_err("Selection out of range.\n\n")?;
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

    if !fs::exists(&backup) {
        return backup;
    }

    for index in 2..=1000 {
        backup = config_path.with_file_name(format!("{}.bak-{}-{}", file_name, timestamp, index));
        if !fs::exists(&backup) {
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

        let templates = load_embedded_wizard_templates().expect("templates");
        let mut wizard_io =
            io::TestWizardIo::new(vec!["codex\n".to_string(), "br-next-task\n".to_string()]);
        let result = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect("wizard");
        assert_eq!(result.config_path, config_path);
        assert!(nested.is_dir(), "expected parent dir created");
        assert!(config_path.is_file(), "expected config written");
        assert!(result.backup_path.is_none());

        let contents = fs::read_to_string(&config_path).expect("read config");
        let loaded = load_config_from_str("<test>", &contents).expect("load config");
        assert_eq!(
            loaded.config.review_loop_limit, templates.defaults.review_loop_limit,
            "expected review_loop_limit to use embedded default"
        );
        assert_eq!(
            loaded.config.log_path, templates.defaults.log_path,
            "expected log_path to use embedded default"
        );
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

        let templates = load_embedded_wizard_templates().expect("templates");
        let mut wizard_io =
            io::TestWizardIo::new(vec!["codex\n".to_string(), "br-next-task\n".to_string()]);
        let result = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect("wizard");

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
    fn existing_review_loop_limit_and_log_path_are_preserved() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        fs::write(
            &config_path,
            r#"
review_loop_limit: 99
log_path: "./custom.log"
"#,
        )
        .expect("write existing config");

        let templates = load_embedded_wizard_templates().expect("templates");
        let mut wizard_io =
            io::TestWizardIo::new(vec!["codex\n".to_string(), "br-next-task\n".to_string()]);
        let result = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect("wizard");
        assert!(
            result.backup_path.is_some(),
            "expected overwrite to create a backup"
        );

        let contents = fs::read_to_string(&config_path).expect("read config");
        let loaded = load_config_from_str("<test>", &contents).expect("load config");
        assert_eq!(
            loaded.config.review_loop_limit, 99,
            "expected review_loop_limit to be preserved"
        );
        assert_eq!(
            loaded.config.log_path, "./custom.log",
            "expected log_path to be preserved"
        );
    }

    #[test]
    fn invalid_yaml_existing_config_warns_and_is_backed_up() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        let original = ":\n  - invalid";
        fs::write(&config_path, original).expect("write invalid yaml");

        let templates = load_embedded_wizard_templates().expect("templates");
        let mut wizard_io =
            io::TestWizardIo::new(vec!["codex\n".to_string(), "br-next-task\n".to_string()]);
        let result = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect("wizard");
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
    fn unknown_keys_are_preserved_as_commented_yaml_and_warned() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        let original = r#"
agent_command: "agent"
agent_review_command: "review"
review_loop_limit: 3
log_path: "./log"
custom_top: 123
commands:
  next_task: "next"
  task_show: "show"
  task_status: "status"
  task_update_in_progress: "update"
  reset_task: "reset"
  extra_cmd: "foo"
hooks:
  on_completed: "done"
  on_requires_human: "human"
  on_doctor_setup: "setup"
  extra_hook: "bar"
"#;
        fs::write(&config_path, original).expect("write existing config");

        let templates = load_embedded_wizard_templates().expect("templates");
        let mut wizard_io =
            io::TestWizardIo::new(vec!["codex\n".to_string(), "br-next-task\n".to_string()]);
        let result = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect("wizard");
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("Unknown/custom config keys were commented out")),
            "expected unknown-keys warning, got: {:?}",
            result.warnings
        );

        let new_contents = fs::read_to_string(&config_path).expect("read new config");
        assert!(
            new_contents.contains(
                "WARNING: Unknown/custom keys from your previous config were preserved below."
            ),
            "expected warning header in output"
        );
        assert!(
            new_contents.contains("# custom_top: 123"),
            "expected custom_top commented"
        );
        assert!(
            !new_contents.contains("\ncustom_top:"),
            "expected custom_top not present as real YAML key"
        );
        assert!(
            new_contents.contains("# commands:"),
            "expected commands block commented"
        );
        assert!(
            new_contents.contains("extra_cmd") && new_contents.contains("#   extra_cmd:"),
            "expected extra_cmd commented"
        );
        assert!(
            new_contents.contains("# hooks:"),
            "expected hooks block commented"
        );
        assert!(
            new_contents.contains("extra_hook") && new_contents.contains("#   extra_hook:"),
            "expected extra_hook commented"
        );

        // Ensure commented keys do not affect parsing.
        let loaded = load_config_from_str("<test>", &new_contents).expect("load config");
        assert!(
            loaded.warnings.is_empty(),
            "expected no unknown-key warnings from commented block, got: {:?}",
            loaded.warnings
        );
    }

    #[test]
    fn merge_prompts_only_for_known_keys_that_differ() {
        let templates = load_embedded_wizard_templates().expect("templates");
        let agent = find_agent_template(&templates.agents, "codex").expect("agent");
        let tracking =
            find_tracking_template(&templates.tracking, "br-next-task").expect("tracking");

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
        let existing_mapping = existing_value.as_mapping().expect("mapping").clone();

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
        let tracking =
            find_tracking_template(&templates.tracking, "br-next-task").expect("tracking");

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
        let existing_mapping = existing_value.as_mapping().expect("mapping").clone();

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

    #[test]
    fn template_selection_defaults_are_applied_on_blank_input() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");

        let templates = load_embedded_wizard_templates().expect("templates");
        let agent = find_agent_template(&templates.agents, "claude").expect("agent");
        let tracking = find_tracking_template(&templates.tracking, "bd-labels").expect("tracking");

        let mut existing = Mapping::new();
        existing.insert(
            Value::String("agent_command".to_string()),
            Value::String(agent.agent_command.clone()),
        );
        existing.insert(
            Value::String("agent_review_command".to_string()),
            Value::String(agent.agent_review_command.clone()),
        );

        let mut commands = Mapping::new();
        commands.insert(
            Value::String("next_task".to_string()),
            Value::String(tracking.commands.next_task.clone()),
        );
        commands.insert(
            Value::String("task_show".to_string()),
            Value::String(tracking.commands.task_show.clone()),
        );
        commands.insert(
            Value::String("task_status".to_string()),
            Value::String(tracking.commands.task_status.clone()),
        );
        commands.insert(
            Value::String("task_update_in_progress".to_string()),
            Value::String(tracking.commands.task_update_in_progress.clone()),
        );
        commands.insert(
            Value::String("reset_task".to_string()),
            Value::String(tracking.commands.reset_task.clone()),
        );
        existing.insert(
            Value::String("commands".to_string()),
            Value::Mapping(commands),
        );

        let mut hooks = Mapping::new();
        hooks.insert(
            Value::String("on_completed".to_string()),
            Value::String(tracking.hooks.on_completed.clone()),
        );
        hooks.insert(
            Value::String("on_requires_human".to_string()),
            Value::String(tracking.hooks.on_requires_human.clone()),
        );
        if let Some(setup) = &tracking.hooks.on_doctor_setup {
            hooks.insert(
                Value::String("on_doctor_setup".to_string()),
                Value::String(setup.clone()),
            );
        }
        existing.insert(Value::String("hooks".to_string()), Value::Mapping(hooks));

        let existing_yaml =
            serde_yaml::to_string(&Value::Mapping(existing)).expect("existing yaml");
        fs::write(&config_path, existing_yaml).expect("write existing config");

        let mut wizard_io = io::TestWizardIo::new(vec!["\n".to_string(), "\n".to_string()]);
        let result = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect("wizard");
        assert_eq!(result.config_path, config_path);

        let new_contents = fs::read_to_string(&config_path).expect("read config");
        let loaded = load_config_from_str("<test>", &new_contents).expect("load config");
        assert_eq!(
            loaded.config.agent_command, agent.agent_command,
            "expected default agent selection to be applied"
        );
        assert_eq!(
            loaded.config.commands.next_task,
            Some(tracking.commands.next_task.clone()),
            "expected default tracking selection to be applied"
        );
    }

    #[test]
    fn merge_decisions_are_driven_via_io_interpreter() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");

        let templates = load_embedded_wizard_templates().expect("templates");
        let agent = find_agent_template(&templates.agents, "codex").expect("agent");
        let tracking =
            find_tracking_template(&templates.tracking, "br-next-task").expect("tracking");

        // Existing config differs on one known key; we will choose to keep it.
        let mut existing = Mapping::new();
        existing.insert(
            Value::String("agent_command".to_string()),
            Value::String(agent.agent_command.clone()),
        );
        existing.insert(
            Value::String("agent_review_command".to_string()),
            Value::String(agent.agent_review_command.clone()),
        );

        let mut commands = Mapping::new();
        commands.insert(
            Value::String("next_task".to_string()),
            Value::String(tracking.commands.next_task.clone()),
        );
        commands.insert(
            Value::String("task_show".to_string()),
            Value::String(tracking.commands.task_show.clone()),
        );
        commands.insert(
            Value::String("task_status".to_string()),
            Value::String(tracking.commands.task_status.clone()),
        );
        commands.insert(
            Value::String("task_update_in_progress".to_string()),
            Value::String(tracking.commands.task_update_in_progress.clone()),
        );
        commands.insert(
            Value::String("reset_task".to_string()),
            Value::String("keep-me".to_string()),
        );
        existing.insert(
            Value::String("commands".to_string()),
            Value::Mapping(commands),
        );

        let mut hooks = Mapping::new();
        hooks.insert(
            Value::String("on_completed".to_string()),
            Value::String(tracking.hooks.on_completed.clone()),
        );
        hooks.insert(
            Value::String("on_requires_human".to_string()),
            Value::String(tracking.hooks.on_requires_human.clone()),
        );
        if let Some(setup) = &tracking.hooks.on_doctor_setup {
            hooks.insert(
                Value::String("on_doctor_setup".to_string()),
                Value::String(setup.clone()),
            );
        }
        existing.insert(Value::String("hooks".to_string()), Value::Mapping(hooks));

        let existing_yaml =
            serde_yaml::to_string(&Value::Mapping(existing)).expect("existing yaml");
        fs::write(&config_path, existing_yaml).expect("write existing config");

        let mut wizard_io = io::TestWizardIo::new(vec![
            "codex\n".to_string(),
            "br-next-task\n".to_string(),
            "k\n".to_string(),
        ]);
        let _ = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Interactive,
            &mut wizard_io,
        )
        .expect("wizard");

        let new_contents = fs::read_to_string(&config_path).expect("read new config");
        let loaded = load_config_from_str("<test>", &new_contents).expect("load config");
        assert_eq!(
            loaded.config.commands.reset_task, "keep-me",
            "expected interactive merge to keep current value"
        );
    }

    #[test]
    fn wizard_validation_failure_prevents_write_and_backup() {
        use crate::wizard_templates::{
            AgentTemplate, DefaultsTemplate, TrackingCommands, TrackingHooks, TrackingTemplate,
            WizardTemplates,
        };

        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        fs::write(&config_path, "old").expect("write existing config");

        let templates = WizardTemplates {
            agents: vec![AgentTemplate {
                id: "bad".to_string(),
                label: "Bad".to_string(),
                description: "bad".to_string(),
                agent_command: "".to_string(),
                agent_review_command: "review".to_string(),
            }],
            tracking: vec![TrackingTemplate {
                id: "trk".to_string(),
                label: "trk".to_string(),
                description: "trk".to_string(),
                commands: TrackingCommands {
                    next_task: "next".to_string(),
                    task_show: "show".to_string(),
                    task_status: "status".to_string(),
                    task_update_in_progress: "update".to_string(),
                    reset_task: "reset".to_string(),
                },
                hooks: TrackingHooks {
                    on_completed: "done".to_string(),
                    on_requires_human: "human".to_string(),
                    on_doctor_setup: None,
                },
            }],
            defaults: DefaultsTemplate {
                review_loop_limit: 1,
                log_path: "./log".to_string(),
            },
        };

        let mut wizard_io = io::TestWizardIo::new(vec!["bad\n".to_string(), "trk\n".to_string()]);
        let err = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect_err("expected error");
        assert!(err.contains("agent_command"), "err: {err}");

        let contents = fs::read_to_string(&config_path).expect("read config");
        assert_eq!(contents, "old");
        assert!(list_backups(temp.path(), "trudger.yml").is_empty());
    }
}
