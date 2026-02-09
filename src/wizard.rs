use chrono::Utc;
use serde_yaml::{Mapping, Value};
use std::env;
use std::path::{Path, PathBuf};

use crate::config::load_config_from_str;
use crate::prompt_defaults::default_prompts;
use crate::prompt_install::{
    detect_prompt_state, overwrite_prompt_with_backup, write_prompt_if_missing, PromptState,
};
use crate::run_loop::validate_config;
use crate::wizard_templates::{
    load_embedded_wizard_templates, AgentTemplate, TrackingTemplate, WizardTemplates,
};

mod fs;
mod interactive;
mod io;

pub(crate) use interactive::run_wizard_cli;
use io::WizardIo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WizardResult {
    pub(crate) config_path: PathBuf,
    pub(crate) backup_path: Option<PathBuf>,
    pub(crate) warnings: Vec<String>,
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

fn build_candidate_value(
    agent: &AgentTemplate,
    tracking: &TrackingTemplate,
    review_loop_limit: u64,
    log_path: String,
) -> Value {
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

    let mut hooks = Mapping::new();
    hooks.insert(
        Value::String("on_completed".to_string()),
        Value::String(tracking.hooks.on_completed.clone()),
    );
    hooks.insert(
        Value::String("on_requires_human".to_string()),
        Value::String(tracking.hooks.on_requires_human.clone()),
    );
    if let Some(value) = &tracking.hooks.on_doctor_setup {
        hooks.insert(
            Value::String("on_doctor_setup".to_string()),
            Value::String(value.clone()),
        );
    }

    let mut candidate = Mapping::new();
    candidate.insert(
        Value::String("agent_command".to_string()),
        Value::String(agent.agent_command.clone()),
    );
    candidate.insert(
        Value::String("agent_review_command".to_string()),
        Value::String(agent.agent_review_command.clone()),
    );
    candidate.insert(
        Value::String("commands".to_string()),
        Value::Mapping(commands),
    );
    candidate.insert(Value::String("hooks".to_string()), Value::Mapping(hooks));
    candidate.insert(
        Value::String("review_loop_limit".to_string()),
        Value::Number(review_loop_limit.into()),
    );
    candidate.insert(
        Value::String("log_path".to_string()),
        Value::String(log_path),
    );

    Value::Mapping(candidate)
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

    let mut candidate_value = build_candidate_value(agent, tracking, review_loop_limit, log_path);

    if merge_mode == WizardMergeMode::Interactive {
        if let Some(existing_mapping) = existing.mapping.as_ref() {
            let mut decider = |prompt: &MergePrompt| prompt_merge_decision(io, prompt);
            merge_known_template_keys(existing_mapping, &mut candidate_value, &mut decider)?;
        }
    }

    let mut yaml = serde_yaml::to_string(&candidate_value).map_err(|err| {
        format!(
            "Internal wizard error: failed to serialize generated config YAML: {}",
            err
        )
    })?;

    let mut unknown_keys_warning: Option<String> = None;
    if let Some(existing_mapping) = existing.mapping.as_ref() {
        if let Some((unknown_block, unknown_paths)) =
            render_unknown_keys_commented_block(existing_mapping)
        {
            yaml.push_str(&unknown_block);
            unknown_keys_warning = Some(format!(
                "Warning: Unknown/custom config keys were commented out and appended to the generated config: {}",
                unknown_paths.join(", ")
            ));
        }
    }

    let prompt_report = maybe_handle_prompt_install_update(io, merge_mode)?;

    let (backup_path, write_warnings) = validate_then_write_config(config_path, &yaml)?;

    let mut all_warnings = existing.warnings;
    if let Some(warning) = unknown_keys_warning {
        all_warnings.push(warning);
    }
    all_warnings.extend(write_warnings);

    if let Some(report) = prompt_report {
        report.print(io)?;
    }

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

#[derive(Debug, Clone)]
struct PromptWizardReport {
    trudge_path: PathBuf,
    review_path: PathBuf,
    trudge_status: String,
    review_status: String,
    missing_after: Vec<PathBuf>,
}

impl PromptWizardReport {
    fn print(&self, io: &mut dyn WizardIo) -> Result<(), String> {
        io.write_out("\nPrompt install/update summary:\n")?;
        io.write_out(&format!(
            "  {}: {}\n",
            self.trudge_path.display(),
            self.trudge_status
        ))?;
        io.write_out(&format!(
            "  {}: {}\n",
            self.review_path.display(),
            self.review_status
        ))?;

        if !self.missing_after.is_empty() {
            io.write_out("\nPrompts are still missing. Trudger requires both prompt files:\n")?;
            io.write_out(&format!("  - {}\n", self.trudge_path.display()))?;
            io.write_out(&format!("  - {}\n", self.review_path.display()))?;
            io.write_out("\nInstall them by either:\n")?;
            io.write_out("  - Rerun `trudger wizard` and accept prompt installation\n")?;
            io.write_out("  - Or run `./install.sh` from a repo checkout\n")?;
        }

        Ok(())
    }
}

fn wizard_home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "Missing HOME environment variable".to_string())
}

fn maybe_handle_prompt_install_update(
    io: &mut dyn WizardIo,
    merge_mode: WizardMergeMode,
) -> Result<Option<PromptWizardReport>, String> {
    if merge_mode != WizardMergeMode::Interactive {
        return Ok(None);
    }

    let home_dir = wizard_home_dir()?;
    let prompts = default_prompts(&home_dir);

    let mut trudge_state = detect_prompt_state(&prompts[0].path, prompts[0].contents)
        .map_err(|err| err.to_string())?;
    let mut review_state = detect_prompt_state(&prompts[1].path, prompts[1].contents)
        .map_err(|err| err.to_string())?;

    let mut trudge_status = String::new();
    let mut review_status = String::new();

    let any_missing = trudge_state == PromptState::Missing || review_state == PromptState::Missing;
    if any_missing {
        let install = prompt_install_missing_prompts(io)?;
        if install {
            if trudge_state == PromptState::Missing {
                write_prompt_if_missing(&prompts[0].path, prompts[0].contents)
                    .map_err(|err| err.to_string())?;
                trudge_state = PromptState::MatchesDefault;
                trudge_status = "installed".to_string();
            }
            if review_state == PromptState::Missing {
                write_prompt_if_missing(&prompts[1].path, prompts[1].contents)
                    .map_err(|err| err.to_string())?;
                review_state = PromptState::MatchesDefault;
                review_status = "installed".to_string();
            }
        } else {
            if trudge_state == PromptState::Missing {
                trudge_status = "skipped (missing)".to_string();
            }
            if review_state == PromptState::Missing {
                review_status = "skipped (missing)".to_string();
            }
        }
    }

    if trudge_state == PromptState::Differs {
        if prompt_overwrite_differing_prompt(io, &prompts[0].path)? {
            let backup = overwrite_prompt_with_backup(&prompts[0].path, prompts[0].contents, true)
                .map_err(|err| err.to_string())?;
            trudge_status = match backup {
                Some(path) => format!("updated (backup: {})", path.display()),
                None => "updated".to_string(),
            };
            trudge_state = PromptState::MatchesDefault;
        } else {
            trudge_status = "kept existing (differs)".to_string();
        }
    }

    if review_state == PromptState::Differs {
        if prompt_overwrite_differing_prompt(io, &prompts[1].path)? {
            let backup = overwrite_prompt_with_backup(&prompts[1].path, prompts[1].contents, true)
                .map_err(|err| err.to_string())?;
            review_status = match backup {
                Some(path) => format!("updated (backup: {})", path.display()),
                None => "updated".to_string(),
            };
            review_state = PromptState::MatchesDefault;
        } else {
            review_status = "kept existing (differs)".to_string();
        }
    }

    if trudge_status.is_empty() {
        trudge_status = match trudge_state {
            PromptState::Missing => "missing".to_string(),
            PromptState::MatchesDefault => "unchanged".to_string(),
            PromptState::Differs => "kept existing (differs)".to_string(),
        };
    }
    if review_status.is_empty() {
        review_status = match review_state {
            PromptState::Missing => "missing".to_string(),
            PromptState::MatchesDefault => "unchanged".to_string(),
            PromptState::Differs => "kept existing (differs)".to_string(),
        };
    }

    let mut missing_after = Vec::new();
    if !prompts[0].path.is_file() {
        missing_after.push(prompts[0].path.clone());
    }
    if !prompts[1].path.is_file() {
        missing_after.push(prompts[1].path.clone());
    }

    Ok(Some(PromptWizardReport {
        trudge_path: prompts[0].path.clone(),
        review_path: prompts[1].path.clone(),
        trudge_status,
        review_status,
        missing_after,
    }))
}

fn prompt_install_missing_prompts(io: &mut dyn WizardIo) -> Result<bool, String> {
    loop {
        io.write_out(
            "\nOne or more required prompt files are missing. Install missing prompts now? [Y/n] (default Y): ",
        )?;
        io.flush_out()?;

        let Some(input) = io.read_line()? else {
            return Err("Wizard aborted (stdin closed).".to_string());
        };
        let trimmed = input.trim().to_ascii_lowercase();
        match trimmed.as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => io.write_err("Please enter 'y' to install prompts or 'n' to skip.\n\n")?,
        }
    }
}

fn prompt_overwrite_differing_prompt(io: &mut dyn WizardIo, path: &Path) -> Result<bool, String> {
    loop {
        io.write_out(&format!(
            "\nPrompt file differs from defaults: {}\nOverwrite with built-in default (creates a .bak-YYYYMMDDTHHMMSSZ backup)? [y/N] (default N): ",
            path.display()
        ))?;
        io.flush_out()?;

        let Some(input) = io.read_line()? else {
            return Err("Wizard aborted (stdin closed).".to_string());
        };
        let trimmed = input.trim().to_ascii_lowercase();
        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "" | "n" | "no" => return Ok(false),
            _ => {
                io.write_err("Please enter 'y' to overwrite or 'n' to keep the existing file.\n\n")?
            }
        }
    }
}

fn render_unknown_keys_commented_block(existing: &Mapping) -> Option<(String, Vec<String>)> {
    let (unknown_mapping, unknown_paths) = extract_unknown_key_values(existing);
    if unknown_mapping.is_empty() {
        return None;
    }

    let rendered = match serde_yaml::to_string(&Value::Mapping(unknown_mapping)) {
        Ok(rendered) => strip_yaml_document_prefix(&rendered),
        Err(err) => {
            let mut block = String::new();
            block.push('\n');
            block.push_str(
                "# -----------------------------------------------------------------------------\n",
            );
            block.push_str(
                "# WARNING: Unknown/custom keys from your previous config were detected,\n",
            );
            block.push_str("# but could not be rendered as YAML.\n");
            block.push_str("# Reason: ");
            block.push_str(&err.to_string());
            block.push('\n');
            block.push_str("# Keys: ");
            block.push_str(&unknown_paths.join(", "));
            block.push('\n');
            block.push_str(
                "# -----------------------------------------------------------------------------\n",
            );
            return Some((block, unknown_paths));
        }
    };

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

    Some((block, unknown_paths))
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
    let Some((last, prefix)) = path.split_last() else {
        return false;
    };

    let mut current = root;
    for &segment in prefix {
        let nested = match current.as_mapping_mut() {
            Some(mapping) => mapping,
            None => return false,
        };

        current = match nested.get_mut(segment) {
            Some(value) => value,
            None => return false,
        };
    }

    let nested = match current.as_mapping_mut() {
        Some(mapping) => mapping,
        None => return false,
    };

    nested.remove(*last).is_some()
}

fn set_value_at_path(root: &mut Value, path: &[&str], value: Value) -> Result<(), String> {
    let Some((last, prefix)) = path.split_last() else {
        return Ok(());
    };

    let mut current = root;
    for (index, &segment) in prefix.iter().enumerate() {
        let nested = current.as_mapping_mut().ok_or_else(|| {
            format!(
                "Internal wizard error: expected YAML mapping at {}",
                path[..index].join(".")
            )
        })?;

        let next = nested
            .entry(Value::String(segment.to_string()))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        if !matches!(next, Value::Mapping(_)) {
            *next = Value::Mapping(Mapping::new());
        }
        current = next;
    }

    let nested = current.as_mapping_mut().ok_or_else(|| {
        format!(
            "Internal wizard error: expected YAML mapping at {}",
            path[..prefix.len()].join(".")
        )
    })?;
    nested.insert(Value::String((*last).to_string()), value);
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
        Some(value) => match serde_yaml::to_string(value) {
            Ok(rendered) => rendered.trim_end().to_string(),
            Err(err) => format!("<failed to render YAML: {}>", err),
        },
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
    next_backup_path_with_timestamp(config_path, file_name, &timestamp)
}

fn next_backup_path_with_timestamp(
    config_path: &Path,
    file_name: &str,
    timestamp: &str,
) -> PathBuf {
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
    use crate::prompt_defaults::{
        default_trudge_prompt_contents, default_trudge_review_prompt_contents, TRUDGE_PROMPT_REL,
        TRUDGE_REVIEW_PROMPT_REL,
    };
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use tempfile::TempDir;

    struct TestHomeGuard {
        old_home: Option<OsString>,
    }

    impl TestHomeGuard {
        fn new(home: &Path) -> Self {
            let old_home = env::var_os("HOME");
            env::set_var("HOME", home);

            let trudge = home.join(TRUDGE_PROMPT_REL);
            let review = home.join(TRUDGE_REVIEW_PROMPT_REL);
            if let Some(parent) = trudge.parent() {
                fs::create_dir_all(parent).expect("create prompts dir");
            }
            fs::write(&trudge, default_trudge_prompt_contents()).expect("write trudge prompt");
            fs::write(&review, default_trudge_review_prompt_contents())
                .expect("write review prompt");

            Self { old_home }
        }
    }

    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match self.old_home.take() {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }
        }
    }

    fn maybe_insert_doctor_setup(hooks: &mut Mapping, setup: &Option<String>) {
        if let Some(value) = setup {
            hooks.insert(
                Value::String("on_doctor_setup".to_string()),
                Value::String(value.clone()),
            );
        }
    }

    fn unserializable_yaml_value() -> Value {
        // `serde_yaml::to_string` can fail for certain YAML values (notably complex map keys);
        // exercise those paths so we don't reintroduce panics to maintain coverage.
        let mut inner = Mapping::new();
        inner.insert(
            Value::String("k".to_string()),
            Value::String("v".to_string()),
        );

        let mut outer = Mapping::new();
        outer.insert(Value::Mapping(inner), Value::String("x".to_string()));
        Value::Mapping(outer)
    }

    #[test]
    fn maybe_insert_doctor_setup_is_noop_for_none() {
        let mut hooks = Mapping::new();
        maybe_insert_doctor_setup(&mut hooks, &None);
        assert!(hooks.is_empty());
    }

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
        assert!(err.contains("agent_command"));

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
        assert!(nested.is_dir());
        assert!(config_path.is_file());
        assert!(result.backup_path.is_none());

        let contents = fs::read_to_string(&config_path).expect("read config");
        let loaded = load_config_from_str("<test>", &contents).expect("load config");
        assert_eq!(
            loaded.config.review_loop_limit.get(),
            templates.defaults.review_loop_limit
        );
        assert_eq!(
            loaded.config.log_path,
            Some(PathBuf::from(templates.defaults.log_path.clone()))
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
        assert_eq!(backups.len(), 1);
        assert_eq!(result.backup_path.as_ref(), Some(&backups[0]));
        let backup_contents = fs::read_to_string(&backups[0]).expect("read backup");
        assert_eq!(backup_contents, original);

        let new_contents = fs::read_to_string(&config_path).expect("read new config");
        assert_ne!(new_contents, original);

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
        assert!(result.backup_path.is_some());

        let contents = fs::read_to_string(&config_path).expect("read config");
        let loaded = load_config_from_str("<test>", &contents).expect("load config");
        assert_eq!(loaded.config.review_loop_limit.get(), 99);
        assert_eq!(loaded.config.log_path, Some(PathBuf::from("./custom.log")));
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
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("could not be parsed as YAML")));

        let backups = list_backups(temp.path(), "trudger.yml");
        assert_eq!(backups.len(), 1);
        let backup_contents = fs::read_to_string(&backups[0]).expect("read backup");
        assert_eq!(backup_contents, original);
        assert!(config_path.is_file());
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
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Unknown/custom config keys were commented out")));

        let new_contents = fs::read_to_string(&config_path).expect("read new config");
        assert!(new_contents.contains(
            "WARNING: Unknown/custom keys from your previous config were preserved below."
        ));
        assert!(new_contents.contains("# custom_top: 123"));
        assert!(!new_contents.contains("\ncustom_top:"));
        assert!(new_contents.contains("# commands:"));
        assert!(new_contents.contains("extra_cmd") && new_contents.contains("#   extra_cmd:"));
        assert!(new_contents.contains("# hooks:"));
        assert!(new_contents.contains("extra_hook") && new_contents.contains("#   extra_hook:"));

        // Ensure commented keys do not affect parsing.
        let loaded = load_config_from_str("<test>", &new_contents).expect("load config");
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn render_unknown_keys_commented_block_reports_render_errors() {
        let bad = unserializable_yaml_value();
        serde_yaml::to_string(&bad).expect_err("expected serde_yaml render error");

        let mut existing = Mapping::new();
        existing.insert(Value::String("custom".to_string()), bad);

        let (block, paths) =
            render_unknown_keys_commented_block(&existing).expect("expected unknown key block");
        assert!(block.contains("could not be rendered as YAML"));
        assert!(block.contains("# Keys: custom"));
        assert_eq!(paths, vec!["custom".to_string()]);
    }

    #[test]
    fn wizard_errors_when_generated_config_yaml_cannot_serialize() {
        let templates = load_embedded_wizard_templates().expect("templates");
        let agent = find_agent_template(&templates.agents, "codex").expect("agent");
        let tracking =
            find_tracking_template(&templates.tracking, "br-next-task").expect("tracking");

        let mut existing_value = build_candidate_value(
            agent,
            tracking,
            templates.defaults.review_loop_limit,
            templates.defaults.log_path.clone(),
        );
        set_value_at_path(
            &mut existing_value,
            &["commands", "reset_task"],
            unserializable_yaml_value(),
        )
        .expect("set reset_task");

        let existing = ExistingConfig {
            mapping: Some(existing_value.as_mapping().expect("mapping").clone()),
            defaults: ExistingDefaults::default(),
            warnings: Vec::new(),
        };

        let mut io = io::TestWizardIo::new(vec!["\n".to_string()]);
        let err = run_wizard_selected_with_existing(
            Path::new("trudger.yml"),
            &templates,
            existing,
            "codex",
            "br-next-task",
            WizardMergeMode::Interactive,
            &mut io,
        )
        .expect_err("expected yaml serialization error");
        assert!(err.contains("failed to serialize generated config YAML"));

        assert!(io.stdout.contains("<failed to render YAML:"));
    }

    #[test]
    fn merge_prompts_only_for_known_keys_that_differ() {
        let templates = load_embedded_wizard_templates().expect("templates");
        let agent = find_agent_template(&templates.agents, "codex").expect("agent");
        let tracking =
            find_tracking_template(&templates.tracking, "br-next-task").expect("tracking");

        let mut candidate_value = build_candidate_value(
            agent,
            tracking,
            templates.defaults.review_loop_limit,
            templates.defaults.log_path.clone(),
        );
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

        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].key, "commands.reset_task");
        assert_eq!(prompted_keys, vec!["commands.reset_task"]);
    }

    #[test]
    fn merge_keep_current_overrides_candidate_for_hooks_on_doctor_setup() {
        let templates = load_embedded_wizard_templates().expect("templates");
        let agent = find_agent_template(&templates.agents, "codex").expect("agent");
        let tracking =
            find_tracking_template(&templates.tracking, "br-next-task").expect("tracking");

        let mut candidate_value = build_candidate_value(
            agent,
            tracking,
            templates.defaults.review_loop_limit,
            templates.defaults.log_path.clone(),
        );
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

        assert_eq!(prompts.len(), 1);
        let merged = get_value_at_path(&candidate_value, &["hooks", "on_doctor_setup"])
            .and_then(|value| value.as_str())
            .expect("merged hooks.on_doctor_setup");
        assert_eq!(merged, "existing");
    }

    #[test]
    fn merge_decision_defaults_to_keep_current() {
        assert_eq!(parse_merge_decision(""), Some(MergeDecision::KeepCurrent));
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
        maybe_insert_doctor_setup(&mut hooks, &bd.hooks.on_doctor_setup);
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
        maybe_insert_doctor_setup(&mut hooks, &tracking.hooks.on_doctor_setup);
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
        assert_eq!(loaded.config.agent_command, agent.agent_command);
        assert_eq!(
            loaded.config.commands.next_task,
            Some(tracking.commands.next_task.clone())
        );
    }

    #[test]
    fn merge_decisions_are_driven_via_io_interpreter() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        let home = TempDir::new().expect("home dir");
        let _home_guard = TestHomeGuard::new(home.path());

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
        maybe_insert_doctor_setup(&mut hooks, &tracking.hooks.on_doctor_setup);
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
        assert_eq!(loaded.config.commands.reset_task, "keep-me");
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
        assert!(err.contains("agent_command"));

        let contents = fs::read_to_string(&config_path).expect("read config");
        assert_eq!(contents, "old");
        assert!(list_backups(temp.path(), "trudger.yml").is_empty());
    }

    #[test]
    fn wizard_selection_errors_when_agents_empty() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");

        let mut templates = load_embedded_wizard_templates().expect("templates");
        templates.agents.clear();

        let mut wizard_io = io::TestWizardIo::new(Vec::new());
        let err = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect_err("expected error");
        assert!(err.contains("No choices available"));
    }

    #[test]
    fn wizard_selection_errors_when_tracking_empty() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");

        let mut templates = load_embedded_wizard_templates().expect("templates");
        templates.tracking.clear();

        let mut wizard_io = io::TestWizardIo::new(vec!["codex\n".to_string()]);
        let err = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect_err("expected error");
        assert!(err.contains("No choices available"));
    }

    #[test]
    fn wizard_interactive_merge_propagates_io_errors() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        fs::write(&config_path, "{}\n").expect("write empty mapping");

        let templates = load_embedded_wizard_templates().expect("templates");
        let existing = read_existing_config(&config_path).expect("read existing");
        let mut io = io::TestWizardIo::new(Vec::new());
        let err = run_wizard_selected_with_existing(
            &config_path,
            &templates,
            existing,
            "codex",
            "br-next-task",
            WizardMergeMode::Interactive,
            &mut io,
        )
        .expect_err("expected error");
        assert!(err.contains("stdin closed"));
    }

    #[test]
    fn wizard_interactive_merge_skips_when_no_existing_mapping() {
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();
        let home = TempDir::new().expect("home dir");
        let _home_guard = TestHomeGuard::new(home.path());

        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");

        let templates = load_embedded_wizard_templates().expect("templates");
        let mut wizard_io =
            io::TestWizardIo::new(vec!["codex\n".to_string(), "br-next-task\n".to_string()]);
        let result = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Interactive,
            &mut wizard_io,
        )
        .expect("wizard");

        assert!(config_path.is_file());
        assert!(result.backup_path.is_none());
    }

    #[test]
    fn wizard_write_skips_create_dir_all_when_parent_is_empty() {
        // This test manipulates the process working directory; keep it serialized.
        let _guard = crate::unit_tests::ENV_MUTEX.lock().unwrap();

        let temp = TempDir::new().expect("temp dir");
        let original_dir = std::env::current_dir().expect("current_dir");
        std::env::set_current_dir(temp.path()).expect("set_current_dir");

        struct CwdGuard(std::path::PathBuf);

        impl Drop for CwdGuard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }

        let _cwd_guard = CwdGuard(original_dir);

        let yaml = r#"
agent_command: "agent"
agent_review_command: "review"
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

        validate_then_write_config(Path::new("trudger.yml"), yaml).expect("write config");
        assert!(temp.path().join("trudger.yml").is_file());

        // Empty paths have no parent; cover that branch without touching the repo cwd.
        let err = write_config_with_backup(Path::new(""), "x").expect_err("expected error");
        assert!(err.contains("Failed to copy") || err.contains("Failed to write"));
    }

    #[cfg(unix)]
    #[test]
    fn wizard_write_fails_when_parent_is_file() {
        let yaml = r#"
agent_command: "agent"
agent_review_command: "review"
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

        let err = validate_then_write_config(Path::new("/dev/null/trudger.yml"), yaml)
            .expect_err("expected error");
        assert!(err.contains("Failed to create directory"));
    }

    #[test]
    fn strip_yaml_document_prefix_handles_document_marker() {
        assert_eq!(
            strip_yaml_document_prefix("---\nfoo: bar\n"),
            "foo: bar\n".to_string()
        );
    }

    #[test]
    fn comment_out_yaml_lines_comments_empty_lines() {
        let rendered = "foo: bar\n\nbaz: qux\n";
        let out = comment_out_yaml_lines(rendered);
        assert!(out.contains("# foo: bar\n"));
        assert!(out.contains("#\n"));
        assert!(out.contains("# baz: qux\n"));
    }

    #[test]
    fn extract_unknown_key_values_ignores_non_string_keys() {
        let mut mapping = Mapping::new();
        mapping.insert(Value::Bool(true), Value::String("ignored".to_string()));
        let (_unknown, paths) = extract_unknown_key_values(&mapping);
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_unknown_nested_mapping_ignores_non_string_keys() {
        let mut nested = Mapping::new();
        nested.insert(Value::Bool(true), Value::String("ignored".to_string()));

        let mut root = Mapping::new();
        root.insert(
            Value::String("commands".to_string()),
            Value::Mapping(nested),
        );

        let result = extract_unknown_nested_mapping(&root, "commands", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn get_value_at_path_returns_none_for_empty_path() {
        assert!(get_value_at_path(&Value::Null, &[]).is_none());
    }

    #[test]
    fn get_mapping_value_at_path_returns_none_for_empty_path() {
        assert!(get_mapping_value_at_path(&Mapping::new(), &[]).is_none());
    }

    #[test]
    fn remove_value_at_path_covers_edge_cases() {
        let mut root = Value::Mapping(Mapping::new());
        assert!(!remove_value_at_path(&mut root, &[]));

        let mut root = Value::Null;
        assert!(!remove_value_at_path(&mut root, &["a"]));
        let mut root = Value::Null;
        assert!(!remove_value_at_path(&mut root, &["a", "b"]));

        let mut root = Value::Mapping(Mapping::new());
        assert!(!remove_value_at_path(&mut root, &["a", "b"]));

        let mut inner = Mapping::new();
        inner.insert(Value::String("b".to_string()), Value::Null);
        let mut outer = Mapping::new();
        outer.insert(Value::String("a".to_string()), Value::Mapping(inner));
        let mut root = Value::Mapping(outer);
        assert!(remove_value_at_path(&mut root, &["a", "b"]));
        assert!(get_value_at_path(&root, &["a", "b"]).is_none());
    }

    #[test]
    fn set_value_at_path_covers_edge_cases() {
        let mut root = Value::Mapping(Mapping::new());
        set_value_at_path(&mut root, &[], Value::Null).expect("ok");

        let mut root = Value::Null;
        let err = set_value_at_path(&mut root, &["a"], Value::Null).expect_err("expected err");
        assert!(err.contains("expected YAML mapping"));
        let mut root = Value::Null;
        let err = set_value_at_path(&mut root, &["a", "b"], Value::Null).expect_err("expected err");
        assert!(err.contains("expected YAML mapping"));

        let mut root = Value::Mapping(Mapping::new());
        set_value_at_path(&mut root, &["a", "b"], Value::String("x".to_string()))
            .expect("set nested");
        assert_eq!(
            get_value_at_path(&root, &["a", "b"]).and_then(|value| value.as_str()),
            Some("x")
        );

        let mut root_mapping = Mapping::new();
        root_mapping.insert(
            Value::String("a".to_string()),
            Value::String("nope".to_string()),
        );
        let mut root = Value::Mapping(root_mapping);
        set_value_at_path(&mut root, &["a", "b"], Value::Bool(true)).expect("set");
        assert_eq!(
            get_value_at_path(&root, &["a", "b"]).and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn merge_known_template_keys_removes_when_current_missing_and_keep_selected() {
        let existing = Mapping::new();
        let mut candidate_mapping = Mapping::new();
        candidate_mapping.insert(
            Value::String("agent_command".to_string()),
            Value::String("agent".to_string()),
        );
        let mut candidate = Value::Mapping(candidate_mapping);

        let mut decided = false;
        let mut decider = |_prompt: &MergePrompt| {
            decided = true;
            Ok(MergeDecision::KeepCurrent)
        };

        let prompts =
            merge_known_template_keys(&existing, &mut candidate, &mut decider).expect("merge");
        assert!(decided);
        assert_eq!(prompts.len(), 1);
        assert!(get_value_at_path(&candidate, &["agent_command"]).is_none());
    }

    #[test]
    fn format_yaml_value_and_merge_decision_parsing_cover_branches() {
        assert_eq!(format_yaml_value(&None), "<missing>");
        assert_eq!(
            format_yaml_value(&Some(Value::String("x".to_string()))),
            "x".to_string()
        );
        assert_eq!(
            parse_merge_decision("r"),
            Some(MergeDecision::ReplaceWithProposed)
        );
        assert_eq!(parse_merge_decision("nonsense"), None);
    }

    #[test]
    fn prompt_merge_decision_errors_when_stdin_closed() {
        let mut io = io::TestWizardIo::new(Vec::new());
        let prompt = MergePrompt {
            key: "x".to_string(),
            current: None,
            proposed: None,
        };

        let err = prompt_merge_decision(&mut io, &prompt).expect_err("expected err");
        assert!(err.contains("stdin closed"));
    }

    #[test]
    fn prompt_merge_decision_writes_error_on_invalid_input() {
        let mut io = io::TestWizardIo::new(vec!["bad\n".to_string(), "r\n".to_string()]);
        let prompt = MergePrompt {
            key: "x".to_string(),
            current: None,
            proposed: None,
        };

        let decision = prompt_merge_decision(&mut io, &prompt).expect("decision");
        assert_eq!(decision, MergeDecision::ReplaceWithProposed);
        assert!(io.stderr.contains("Please enter"));
    }

    #[test]
    fn prompt_merge_decision_propagates_write_out_errors() {
        #[derive(Default)]
        struct FailWriteOut {
            fail_on_call: usize,
            calls: usize,
        }

        impl WizardIo for FailWriteOut {
            fn write_out(&mut self, _s: &str) -> Result<(), String> {
                self.calls += 1;
                if self.calls == self.fail_on_call {
                    return Err("write_out failed".to_string());
                }
                Ok(())
            }

            fn write_err(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn flush_out(&mut self) -> Result<(), String> {
                Ok(())
            }

            fn read_line(&mut self) -> Result<Option<String>, String> {
                Ok(Some("k\n".to_string()))
            }
        }

        let prompt = MergePrompt {
            key: "x".to_string(),
            current: Some(Value::String("cur".to_string())),
            proposed: Some(Value::String("prop".to_string())),
        };

        for call in 1..=4 {
            let mut io = FailWriteOut {
                fail_on_call: call,
                calls: 0,
            };
            assert!(prompt_merge_decision(&mut io, &prompt).is_err());
        }

        // Ensure the trait methods that are not reached in the failure cases are still covered.
        let mut io = FailWriteOut {
            fail_on_call: usize::MAX,
            calls: 0,
        };
        assert!(io.write_err("x").is_ok());
        assert!(io.flush_out().is_ok());
        assert!(io.read_line().is_ok());
    }

    #[test]
    fn prompt_merge_decision_propagates_flush_errors() {
        struct FailFlush;

        impl WizardIo for FailFlush {
            fn write_out(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn write_err(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn flush_out(&mut self) -> Result<(), String> {
                Err("flush failed".to_string())
            }

            fn read_line(&mut self) -> Result<Option<String>, String> {
                Ok(Some("k\n".to_string()))
            }
        }

        let prompt = MergePrompt {
            key: "x".to_string(),
            current: None,
            proposed: None,
        };

        let mut io = FailFlush;
        let err = prompt_merge_decision(&mut io, &prompt).expect_err("expected err");
        assert!(err.contains("flush failed"));

        assert!(io.write_err("x").is_ok());
        assert!(io.read_line().is_ok());
    }

    #[test]
    fn prompt_merge_decision_propagates_read_errors() {
        struct FailRead;

        impl WizardIo for FailRead {
            fn write_out(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn write_err(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn flush_out(&mut self) -> Result<(), String> {
                Ok(())
            }

            fn read_line(&mut self) -> Result<Option<String>, String> {
                Err("read failed".to_string())
            }
        }

        let prompt = MergePrompt {
            key: "x".to_string(),
            current: None,
            proposed: None,
        };

        let mut io = FailRead;
        let err = prompt_merge_decision(&mut io, &prompt).expect_err("expected err");
        assert!(err.contains("read failed"));

        assert!(io.write_err("x").is_ok());
    }

    #[test]
    fn prompt_merge_decision_propagates_write_err_errors() {
        struct FailWriteErr;

        impl WizardIo for FailWriteErr {
            fn write_out(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn write_err(&mut self, _s: &str) -> Result<(), String> {
                Err("write_err failed".to_string())
            }

            fn flush_out(&mut self) -> Result<(), String> {
                Ok(())
            }

            fn read_line(&mut self) -> Result<Option<String>, String> {
                Ok(Some("bad\n".to_string()))
            }
        }

        let prompt = MergePrompt {
            key: "x".to_string(),
            current: None,
            proposed: None,
        };

        let mut io = FailWriteErr;
        let err = prompt_merge_decision(&mut io, &prompt).expect_err("expected err");
        assert!(err.contains("write_err failed"));
    }

    #[test]
    fn prompt_template_choice_covers_error_paths() {
        let mut io = io::TestWizardIo::new(Vec::new());
        let err = prompt_template_choice(&mut io, "x", Vec::new(), None).expect_err("err");
        assert!(err.contains("No choices available"));

        let mut io = io::TestWizardIo::new(Vec::new());
        let err = prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None)
            .expect_err("expected err");
        assert!(err.contains("stdin closed"));

        let mut io = io::TestWizardIo::new(vec!["\n".to_string(), "1\n".to_string()]);
        let id = prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None).expect("id");
        assert_eq!(id, "a");
        assert!(io.stderr.contains("Selection must not be empty"));

        let mut io = io::TestWizardIo::new(vec!["99\n".to_string(), "1\n".to_string()]);
        let id = prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None).expect("id");
        assert_eq!(id, "a");
        assert!(io.stderr.contains("Selection out of range"));
    }

    #[test]
    fn prompt_template_choice_propagates_write_out_errors() {
        #[derive(Default)]
        struct FailWriteOut {
            fail_on_call: usize,
            calls: usize,
        }

        impl WizardIo for FailWriteOut {
            fn write_out(&mut self, _s: &str) -> Result<(), String> {
                self.calls += 1;
                if self.calls == self.fail_on_call {
                    return Err("write_out failed".to_string());
                }
                Ok(())
            }

            fn write_err(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn flush_out(&mut self) -> Result<(), String> {
                Ok(())
            }

            fn read_line(&mut self) -> Result<Option<String>, String> {
                Ok(Some("a\n".to_string()))
            }
        }

        for call in 1..=3 {
            let mut io = FailWriteOut {
                fail_on_call: call,
                calls: 0,
            };
            assert!(prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None,).is_err());
        }

        // Cover the "blank for default" prompt branch.
        let mut io = FailWriteOut {
            fail_on_call: 3,
            calls: 0,
        };
        assert!(
            prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], Some("a"),).is_err()
        );

        let mut io = FailWriteOut {
            fail_on_call: usize::MAX,
            calls: 0,
        };
        assert!(io.write_err("x").is_ok());
        assert!(io.flush_out().is_ok());
        assert!(io.read_line().is_ok());
    }

    #[test]
    fn prompt_template_choice_propagates_flush_errors() {
        struct FailFlush;

        impl WizardIo for FailFlush {
            fn write_out(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn write_err(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn flush_out(&mut self) -> Result<(), String> {
                Err("flush failed".to_string())
            }

            fn read_line(&mut self) -> Result<Option<String>, String> {
                Ok(Some("a\n".to_string()))
            }
        }

        let mut io = FailFlush;
        let err = prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None)
            .expect_err("expected err");
        assert!(err.contains("flush failed"));

        assert!(io.write_err("x").is_ok());
        assert!(io.read_line().is_ok());
    }

    #[test]
    fn prompt_template_choice_propagates_read_errors() {
        struct FailRead;

        impl WizardIo for FailRead {
            fn write_out(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn write_err(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn flush_out(&mut self) -> Result<(), String> {
                Ok(())
            }

            fn read_line(&mut self) -> Result<Option<String>, String> {
                Err("read failed".to_string())
            }
        }

        let mut io = FailRead;
        let err = prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None)
            .expect_err("expected err");
        assert!(err.contains("read failed"));

        assert!(io.write_err("x").is_ok());
    }

    #[test]
    fn prompt_template_choice_propagates_write_err_errors() {
        struct FailWriteErr {
            inputs: std::collections::VecDeque<String>,
        }

        impl WizardIo for FailWriteErr {
            fn write_out(&mut self, _s: &str) -> Result<(), String> {
                Ok(())
            }

            fn write_err(&mut self, _s: &str) -> Result<(), String> {
                Err("write_err failed".to_string())
            }

            fn flush_out(&mut self) -> Result<(), String> {
                Ok(())
            }

            fn read_line(&mut self) -> Result<Option<String>, String> {
                Ok(self.inputs.pop_front())
            }
        }

        let mut io = FailWriteErr {
            inputs: vec!["\n".to_string()].into(),
        };
        let err = prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None)
            .expect_err("expected err");
        assert!(err.contains("write_err failed"));

        let mut io = FailWriteErr {
            inputs: vec!["99\n".to_string()].into(),
        };
        let err = prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None)
            .expect_err("expected err");
        assert!(err.contains("write_err failed"));
    }

    #[test]
    fn next_backup_path_with_timestamp_skips_existing_backups() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        let file_name = "trudger.yml";
        let timestamp = "20000101T000000Z";

        let first = temp.path().join(format!("{}.bak-{}", file_name, timestamp));
        fs::write(&first, "x").expect("write first backup");

        let next = next_backup_path_with_timestamp(&config_path, file_name, timestamp);
        assert!(next.ends_with(format!("{}.bak-{}-2", file_name, timestamp)));
    }

    #[test]
    fn next_backup_path_with_timestamp_returns_last_candidate_on_exhaustion() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        let file_name = "trudger.yml";
        let timestamp = "20000101T000000Z";

        let first = temp.path().join(format!("{}.bak-{}", file_name, timestamp));
        fs::write(&first, "x").expect("write first backup");
        for index in 2..=1000 {
            let candidate = temp
                .path()
                .join(format!("{}.bak-{}-{}", file_name, timestamp, index));
            fs::write(candidate, "x").expect("write backup");
        }

        let next = next_backup_path_with_timestamp(&config_path, file_name, timestamp);
        assert!(next.ends_with(format!("{}.bak-{}-1000", file_name, timestamp)));
    }

    #[test]
    fn wizard_errors_when_existing_config_is_not_utf8() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        fs::write(&config_path, [0xff]).expect("write invalid utf8 config");

        let templates = load_embedded_wizard_templates().expect("templates");
        let mut wizard_io = io::TestWizardIo::new(Vec::new());
        let err = run_wizard_with_io(
            &config_path,
            &templates,
            WizardMergeMode::Overwrite,
            &mut wizard_io,
        )
        .expect_err("expected error");
        assert!(err.contains("Failed to read"));
    }

    #[test]
    fn wizard_errors_on_unknown_template_ids() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");

        let templates = load_embedded_wizard_templates().expect("templates");
        let mut io = io::TestWizardIo::new(Vec::new());

        let existing = ExistingConfig {
            mapping: None,
            defaults: ExistingDefaults::default(),
            warnings: Vec::new(),
        };
        let err = run_wizard_selected_with_existing(
            &config_path,
            &templates,
            existing,
            "nope",
            "br-next-task",
            WizardMergeMode::Overwrite,
            &mut io,
        )
        .expect_err("expected error");
        assert!(err.contains("Unknown agent template id"));

        let existing = ExistingConfig {
            mapping: None,
            defaults: ExistingDefaults::default(),
            warnings: Vec::new(),
        };
        let err = run_wizard_selected_with_existing(
            &config_path,
            &templates,
            existing,
            "codex",
            "nope",
            WizardMergeMode::Overwrite,
            &mut io,
        )
        .expect_err("expected error");
        assert!(err.contains("Unknown tracking template id"));
    }

    #[test]
    fn validate_generated_config_propagates_validation_errors() {
        let invalid = r#"
agent_command: "agent"
agent_review_command: "review"
review_loop_limit: 1
log_path: "./log"
commands:
  next_task: ""
  task_show: "x"
  task_status: "x"
  task_update_in_progress: "x"
  reset_task: "x"
hooks:
  on_completed: "x"
  on_requires_human: "x"
"#;
        let err = validate_generated_config(invalid).expect_err("expected error");
        assert!(err.contains("commands.next_task"));
    }

    #[test]
    fn wizard_backup_copy_fails_when_existing_config_is_directory() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("trudger.yml");
        fs::create_dir(&config_path).expect("create existing config dir");

        let err = write_config_with_backup(&config_path, "x").expect_err("expected error");
        assert!(err.contains("Failed to copy"));
    }

    #[test]
    fn get_mapping_value_at_path_returns_none_when_path_missing_or_non_mapping() {
        let mut nested = Mapping::new();
        nested.insert(Value::String("c".to_string()), Value::Bool(true));
        let mut root = Mapping::new();
        root.insert(
            Value::String("a".to_string()),
            Value::String("x".to_string()),
        );
        root.insert(Value::String("b".to_string()), Value::Mapping(nested));

        assert!(get_mapping_value_at_path(&root, &["missing"]).is_none());
        assert!(get_mapping_value_at_path(&root, &["a", "b"]).is_none());
        assert!(get_mapping_value_at_path(&root, &["b", "missing"]).is_none());
    }

    #[test]
    fn get_value_at_path_returns_none_when_root_or_key_missing() {
        assert!(get_value_at_path(&Value::String("x".to_string()), &["a"]).is_none());
        assert!(get_value_at_path(&Value::Mapping(Mapping::new()), &["a"]).is_none());
    }

    #[test]
    fn merge_known_template_keys_errors_when_candidate_cannot_be_set() {
        let mut existing = Mapping::new();
        existing.insert(
            Value::String("agent_command".to_string()),
            Value::String("agent".to_string()),
        );
        let mut candidate = Value::String("nope".to_string());

        let mut decider = |_prompt: &MergePrompt| Ok(MergeDecision::KeepCurrent);
        let err = merge_known_template_keys(&existing, &mut candidate, &mut decider)
            .expect_err("expected error");
        assert!(err.contains("expected YAML mapping"));
    }

    #[test]
    fn strip_yaml_document_prefix_keeps_text_without_marker() {
        assert_eq!(
            strip_yaml_document_prefix("foo: bar\n"),
            "foo: bar\n".to_string()
        );
    }

    #[test]
    fn prompt_template_choice_accepts_non_numeric_id() {
        let mut io = io::TestWizardIo::new(vec!["custom\n".to_string()]);
        let id = prompt_template_choice(&mut io, "x", vec!["a: A".to_string()], None).expect("id");
        assert_eq!(id, "custom");
    }

    #[test]
    fn remove_value_at_path_returns_false_when_prefix_not_mapping() {
        let mut mapping = Mapping::new();
        mapping.insert(
            Value::String("a".to_string()),
            Value::String("x".to_string()),
        );
        let mut root = Value::Mapping(mapping);
        assert!(!remove_value_at_path(&mut root, &["a", "b"]));
    }
}
