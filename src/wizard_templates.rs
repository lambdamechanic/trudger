use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct AgentTemplate {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) agent_command: String,
    pub(crate) agent_review_command: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct TrackingTemplate {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) commands: TrackingCommands,
    pub(crate) hooks: TrackingHooks,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct TrackingCommands {
    pub(crate) next_task: String,
    pub(crate) task_show: String,
    pub(crate) task_status: String,
    pub(crate) task_update_in_progress: String,
    pub(crate) reset_task: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct TrackingHooks {
    pub(crate) on_completed: String,
    pub(crate) on_requires_human: String,
    #[serde(default)]
    pub(crate) on_doctor_setup: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct DefaultsTemplate {
    pub(crate) review_loop_limit: u64,
    pub(crate) log_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WizardTemplates {
    pub(crate) agents: Vec<AgentTemplate>,
    pub(crate) tracking: Vec<TrackingTemplate>,
    pub(crate) defaults: DefaultsTemplate,
}

const AGENT_CODEX: &str = include_str!("../config_templates/agents/codex.yml");
const AGENT_CLAUDE: &str = include_str!("../config_templates/agents/claude.yml");
const AGENT_PI: &str = include_str!("../config_templates/agents/pi.yml");

const TRACKING_BR_NEXT_TASK: &str = include_str!("../config_templates/tracking/br-next-task.yml");
const TRACKING_BD_LABELS: &str = include_str!("../config_templates/tracking/bd-labels.yml");

const DEFAULTS: &str = include_str!("../config_templates/defaults.yml");

pub(crate) fn load_embedded_wizard_templates() -> Result<WizardTemplates, String> {
    let agents = vec![
        parse_template("config_templates/agents/codex.yml", AGENT_CODEX)?,
        parse_template("config_templates/agents/claude.yml", AGENT_CLAUDE)?,
        parse_template("config_templates/agents/pi.yml", AGENT_PI)?,
    ];

    let tracking = vec![
        parse_template(
            "config_templates/tracking/br-next-task.yml",
            TRACKING_BR_NEXT_TASK,
        )?,
        parse_template(
            "config_templates/tracking/bd-labels.yml",
            TRACKING_BD_LABELS,
        )?,
    ];

    let defaults: DefaultsTemplate = parse_template("config_templates/defaults.yml", DEFAULTS)?;

    validate_required_templates(&agents, &tracking, &defaults)?;

    Ok(WizardTemplates {
        agents,
        tracking,
        defaults,
    })
}

fn parse_template<T: for<'a> Deserialize<'a>>(path: &str, contents: &str) -> Result<T, String> {
    serde_yaml::from_str(contents)
        .map_err(|err| format!("Failed to parse embedded {}: {}", path, err))
}

fn validate_required_templates(
    agents: &[AgentTemplate],
    tracking: &[TrackingTemplate],
    defaults: &DefaultsTemplate,
) -> Result<(), String> {
    // Duplicate IDs make selection ambiguous; treat this as a build-time data error.
    validate_unique_ids(agents.iter().map(|t| t.id.as_str()), "agent template")?;
    validate_unique_ids(tracking.iter().map(|t| t.id.as_str()), "tracking template")?;

    require_ids(
        agents.iter().map(|t| t.id.as_str()),
        "agent template",
        &["codex", "claude", "pi"],
    )?;
    require_ids(
        tracking.iter().map(|t| t.id.as_str()),
        "tracking template",
        &["br-next-task", "bd-labels"],
    )?;

    if defaults.review_loop_limit == 0 {
        return Err(
            "Embedded defaults invalid: review_loop_limit must be non-zero. Reinstall/upgrade trudger."
                .to_string(),
        );
    }
    if defaults.log_path.trim().is_empty() {
        return Err(
            "Embedded defaults invalid: log_path must be non-empty. Reinstall/upgrade trudger."
                .to_string(),
        );
    }

    Ok(())
}

fn validate_unique_ids<'a, I>(ids: I, label: &str) -> Result<(), String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(format!(
                "Embedded {} id '{}' is duplicated. Reinstall/upgrade trudger.",
                label, id
            ));
        }
    }
    Ok(())
}

fn require_ids<'a, I>(ids: I, label: &str, required: &[&str]) -> Result<(), String>
where
    I: IntoIterator<Item = &'a str>,
{
    let available: std::collections::HashSet<&str> = ids.into_iter().collect();
    let mut missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|id| !available.contains(id))
        .collect();
    missing.sort_unstable();
    if missing.is_empty() {
        return Ok(());
    }

    Err(format!(
        "Embedded {} missing required ids: {}. This indicates the binary was built without expected template data; reinstall/upgrade trudger.",
        label,
        missing.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_templates_parse_and_have_required_ids() {
        let templates = load_embedded_wizard_templates().expect("load embedded templates");

        let agent_ids: std::collections::HashSet<&str> =
            templates.agents.iter().map(|t| t.id.as_str()).collect();
        for id in ["codex", "claude", "pi"] {
            assert!(
                agent_ids.contains(id),
                "missing required agent template: {id}"
            );
        }

        let tracking_ids: std::collections::HashSet<&str> =
            templates.tracking.iter().map(|t| t.id.as_str()).collect();
        for id in ["br-next-task", "bd-labels"] {
            assert!(
                tracking_ids.contains(id),
                "missing required tracking template: {id}"
            );
        }

        assert!(templates.defaults.review_loop_limit > 0);
        assert!(!templates.defaults.log_path.trim().is_empty());
    }

    #[test]
    fn missing_required_agent_ids_error_is_actionable() {
        fn agent(id: &str) -> AgentTemplate {
            AgentTemplate {
                id: id.to_string(),
                label: id.to_string(),
                description: "desc".to_string(),
                agent_command: "cmd".to_string(),
                agent_review_command: "review".to_string(),
            }
        }

        fn tracking(id: &str) -> TrackingTemplate {
            TrackingTemplate {
                id: id.to_string(),
                label: id.to_string(),
                description: "desc".to_string(),
                commands: TrackingCommands {
                    next_task: "x".to_string(),
                    task_show: "x".to_string(),
                    task_status: "x".to_string(),
                    task_update_in_progress: "x".to_string(),
                    reset_task: "x".to_string(),
                },
                hooks: TrackingHooks {
                    on_completed: "x".to_string(),
                    on_requires_human: "x".to_string(),
                    on_doctor_setup: None,
                },
            }
        }

        let agents = vec![agent("codex")];
        let tracking = vec![tracking("br-next-task"), tracking("bd-labels")];
        let defaults = DefaultsTemplate {
            review_loop_limit: 1,
            log_path: "./x".to_string(),
        };

        let err = validate_required_templates(&agents, &tracking, &defaults)
            .expect_err("expected missing required agent ids error");
        assert!(err.contains("missing required ids"), "err: {err}");
        assert!(err.contains("agent template"), "err: {err}");
        assert!(err.contains("claude"), "err: {err}");
        assert!(err.contains("pi"), "err: {err}");
        assert!(err.contains("reinstall/upgrade"), "err: {err}");
    }

    #[test]
    fn missing_required_tracking_ids_error_is_actionable() {
        fn agent(id: &str) -> AgentTemplate {
            AgentTemplate {
                id: id.to_string(),
                label: id.to_string(),
                description: "desc".to_string(),
                agent_command: "cmd".to_string(),
                agent_review_command: "review".to_string(),
            }
        }

        fn tracking(id: &str) -> TrackingTemplate {
            TrackingTemplate {
                id: id.to_string(),
                label: id.to_string(),
                description: "desc".to_string(),
                commands: TrackingCommands {
                    next_task: "x".to_string(),
                    task_show: "x".to_string(),
                    task_status: "x".to_string(),
                    task_update_in_progress: "x".to_string(),
                    reset_task: "x".to_string(),
                },
                hooks: TrackingHooks {
                    on_completed: "x".to_string(),
                    on_requires_human: "x".to_string(),
                    on_doctor_setup: None,
                },
            }
        }

        let agents = vec![agent("codex"), agent("claude"), agent("pi")];
        let tracking = vec![tracking("br-next-task")];
        let defaults = DefaultsTemplate {
            review_loop_limit: 1,
            log_path: "./x".to_string(),
        };

        let err = validate_required_templates(&agents, &tracking, &defaults)
            .expect_err("expected missing required tracking ids error");
        assert!(err.contains("missing required ids"), "err: {err}");
        assert!(err.contains("tracking template"), "err: {err}");
        assert!(err.contains("bd-labels"), "err: {err}");
        assert!(err.contains("reinstall/upgrade"), "err: {err}");
    }
}
