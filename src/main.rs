use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod config;

use crate::config::load_config;
const PROMPT_TRUDGE: &str = ".codex/prompts/trudge.md";
const PROMPT_REVIEW: &str = ".codex/prompts/trudge_review.md";
const DEFAULT_CONFIG_REL: &str = ".config/trudger.yml";

fn usage() {
    println!(
        "Usage: ./trudger [options] [task_id ...]\n\n\
Loop over ready br tasks and run Codex solve+review prompts.\n\
If task IDs are provided, they run first (in order) before br ready tasks.\n\
Configuration is loaded from ~/.config/trudger.yml by default.\n\n\
Options:\n\
  -c, --config PATH   Load configuration from PATH instead of ~/.config/trudger.yml.\n\
  -h, --help          Show this help text."
    );
}

fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "Missing HOME environment variable".to_string())
}

fn require_file(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("Missing {}: {}", label, path.display()));
    }
    Ok(())
}

fn bootstrap_config_error(default_path: &Path) -> String {
    format!(
        "Missing config file: {}\n\n\
Sample configurations:\n\n\
1) Trudgeable with hooks\n\
   - Selects the next ready br task labeled \"trudgeable\".\n\
   - On completion, removes the \"trudgeable\" label.\n\
   - On requires-human, removes \"trudgeable\" and adds \"human-required\".\n\
   mkdir -p ~/.config && curl -fsSL https://raw.githubusercontent.com/lambdamechanic/trudger/main/sample_configuration/trudgeable-with-hooks.yml -o ~/.config/trudger.yml\n\n\
2) Robot triage\n\
   - Selects tasks via `bv --robot-next`.\n\
   - No label changes (hooks are no-ops).\n\
   mkdir -p ~/.config && curl -fsSL https://raw.githubusercontent.com/lambdamechanic/trudger/main/sample_configuration/robot-triage.yml -o ~/.config/trudger.yml",
        default_path.display()
    )
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1).peekable();
    let mut config_path: Option<PathBuf> = None;
    let mut config_path_source_flag = false;
    let mut _manual_tasks: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            "-c" | "--config" => {
                let Some(value) = args.next() else {
                    eprintln!("Missing value for {}", arg);
                    usage();
                    return ExitCode::from(1);
                };
                if value.is_empty() {
                    eprintln!("Missing value for {}", arg);
                    usage();
                    return ExitCode::from(1);
                }
                config_path = Some(PathBuf::from(value));
                config_path_source_flag = true;
            }
            "--" => {
                _manual_tasks.extend(args.map(|v| v));
                break;
            }
            _ if arg.starts_with("--config=") => {
                let value = arg.trim_start_matches("--config=");
                if value.is_empty() {
                    eprintln!("Missing value for --config");
                    usage();
                    return ExitCode::from(1);
                }
                config_path = Some(PathBuf::from(value));
                config_path_source_flag = true;
            }
            _ if arg.starts_with('-') => {
                eprintln!("Unknown option: {}", arg);
                usage();
                return ExitCode::from(1);
            }
            _ => {
                _manual_tasks.push(arg);
            }
        }
    }

    let home = match home_dir() {
        Ok(dir) => dir,
        Err(message) => {
            eprintln!("{}", message);
            return ExitCode::from(1);
        }
    };

    let prompt_trudge = home.join(PROMPT_TRUDGE);
    let prompt_review = home.join(PROMPT_REVIEW);
    if let Err(message) = require_file(&prompt_trudge, "prompt file") {
        eprintln!("{}", message);
        return ExitCode::from(1);
    }
    if let Err(message) = require_file(&prompt_review, "prompt file") {
        eprintln!("{}", message);
        return ExitCode::from(1);
    }

    let default_config = home.join(DEFAULT_CONFIG_REL);
    let config_path = config_path.unwrap_or_else(|| default_config.clone());
    if !config_path.is_file() {
        if config_path_source_flag {
            eprintln!("Missing config file: {}", config_path.display());
        } else {
            eprintln!("{}", bootstrap_config_error(&default_config));
        }
        return ExitCode::from(1);
    }

    let _config = match load_config(&config_path) {
        Ok(loaded) => loaded.config,
        Err(message) => {
            eprintln!("{}", message);
            return ExitCode::from(1);
        }
    };

    // Placeholder: main loop implemented in follow-up tasks.

    ExitCode::SUCCESS
}
