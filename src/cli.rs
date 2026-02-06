use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "trudger", disable_help_subcommand = true)]
pub(crate) struct Cli {
    #[arg(short = 'c', long = "config", global = true, value_name = "PATH")]
    pub(crate) config: Option<PathBuf>,

    #[arg(
        short = 't',
        long = "task",
        global = true,
        action = clap::ArgAction::Append,
        value_name = "TASK_ID"
    )]
    pub(crate) task: Vec<String>,

    #[arg(value_name = "ARG", hide = true)]
    pub(crate) positional: Vec<String>,

    #[command(subcommand)]
    pub(crate) command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum CliCommand {
    Doctor,
}

pub(crate) fn parse_manual_tasks(raw_values: &[String]) -> Result<Vec<String>, String> {
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
            tasks.push(trimmed.to_string());
        }
    }
    Ok(tasks)
}
