use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::task_types::TaskId;

#[derive(Debug, Parser)]
#[command(
    name = "trudger",
    about = "Trudger slowly and unimaginatively trudges through your tasks.",
    long_about = "Trudger selects tasks (or accepts manual task IDs), runs an agent solve + review loop, and then verifies the task is closed or escalated for human input.\n\nIf you do not have a config yet, run `trudger wizard` to generate one.\n\nTask context is provided to configured commands via TRUDGER_* environment variables.",
    disable_help_subcommand = true
)]
pub(crate) struct Cli {
    /// Load configuration from PATH instead of ~/.config/trudger.yml.
    #[arg(
        short = 'c',
        long = "config",
        global = true,
        value_name = "PATH",
        help = "Load configuration from PATH instead of ~/.config/trudger.yml."
    )]
    pub(crate) config: Option<PathBuf>,

    /// Run a specific task first (repeatable; also supports comma-separated lists).
    #[arg(
        short = 't',
        long = "task",
        global = true,
        action = clap::ArgAction::Append,
        value_name = "TASK_ID",
        help = "Run a specific task first (repeatable; also supports comma-separated lists)."
    )]
    pub(crate) task: Vec<String>,

    #[arg(value_name = "ARG", hide = true)]
    pub(crate) positional: Vec<String>,

    #[command(subcommand)]
    pub(crate) command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum CliCommand {
    #[command(about = "Interactively generate a trudger config from embedded templates.")]
    /// Interactively generate a trudger config from embedded templates.
    Wizard,

    #[command(about = "Run configuration/command validation against a temporary scratch task DB.")]
    /// Run configuration/command validation against a temporary scratch task DB.
    Doctor,
}

pub(crate) fn parse_manual_tasks(raw_values: &[String]) -> Result<Vec<TaskId>, String> {
    let mut tasks = Vec::new();
    for raw in raw_values {
        for (index, segment) in raw.split(',').enumerate() {
            let trimmed = segment.trim();
            if trimmed.is_empty() {
                return Err(format!(
                    "Invalid -t/--task value: empty segment in {:?} at index {}.",
                    raw, index
                ));
            }
            // `TaskId` validation currently rejects only empty strings, which we already checked.
            let task_id = TaskId::try_from(trimmed).unwrap();
            tasks.push(task_id);
        }
    }
    Ok(tasks)
}
