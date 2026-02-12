use clap::Parser;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::cli::{parse_manual_tasks, Cli, CliCommand};
use crate::config::{load_config, NotificationScope};
use crate::doctor::run_doctor_mode;
use crate::logger::Logger;
use crate::run_loop::{
    dispatch_notification_hook, finish_current_task_context, quit, reset_task_on_exit, run_loop,
    validate_config, NotificationEvent, Quit, RuntimeState,
};
use crate::tmux::TmuxState;
use crate::wizard::run_wizard_cli;

const PROMPT_TRUDGE: &str = ".codex/prompts/trudge.md";
const PROMPT_REVIEW: &str = ".codex/prompts/trudge_review.md";
const DEFAULT_CONFIG_REL: &str = ".config/trudger.yml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Run,
    Wizard,
    Doctor,
}

fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "Missing HOME environment variable".to_string())
}

pub(crate) fn require_file(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("Missing {}: {}", label, path.display()));
    }
    Ok(())
}

fn bootstrap_config_error(default_path: &Path) -> String {
    format!(
        "Missing config file: {}\n\n\
To generate a config interactively, run:\n\
  trudger wizard\n\n\
To generate a config at a non-default path, run:\n\
  trudger wizard --config PATH",
        default_path.display()
    )
}

pub(crate) fn render_prompt(path: &Path) -> Result<String, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read prompt {}: {}", path.display(), err))?;
    let mut out = String::new();
    let mut in_frontmatter = false;
    let mut first_line = true;

    for line in content.lines() {
        if first_line && line == "---" {
            in_frontmatter = true;
            first_line = false;
            continue;
        }
        first_line = false;
        if in_frontmatter && line == "---" {
            in_frontmatter = false;
            continue;
        }
        if !in_frontmatter {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.ends_with('\n') {
        out.pop();
    }
    Ok(out)
}

fn run_with_cli_impl<F>(cli: Cli, wizard_runner: F) -> Result<(), Quit>
where
    F: FnOnce(&Path) -> Result<(), Quit>,
{
    let mode = match cli.command {
        Some(CliCommand::Doctor) => AppMode::Doctor,
        Some(CliCommand::Wizard) => AppMode::Wizard,
        None => AppMode::Run,
    };

    let manual_tasks = match parse_manual_tasks(&cli.task) {
        Ok(tasks) => tasks,
        Err(message) => {
            eprintln!("{}", message);
            return Err(Quit {
                code: 1,
                reason: message,
            });
        }
    };
    if mode == AppMode::Doctor && !manual_tasks.is_empty() {
        let message = "-t/--task is not supported in doctor mode.".to_string();
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: message,
        });
    }
    if mode == AppMode::Wizard && !manual_tasks.is_empty() {
        let message = "-t/--task is not supported in wizard mode.".to_string();
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: message,
        });
    }
    if mode == AppMode::Run && !cli.positional.is_empty() {
        let message = format!(
            "Positional arguments are not supported.\nMigration: pass manual task ids via -t/--task (for example: trudger -t {}).",
            cli.positional.join(" -t ")
        );
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: "positional_args_not_supported".to_string(),
        });
    }
    if mode == AppMode::Wizard && !cli.positional.is_empty() {
        let message = "Positional arguments are not supported in wizard mode.".to_string();
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: message,
        });
    }

    let config_path = cli.config;
    let config_path_source_flag = config_path.is_some();
    let home = home_dir().map_err(|message| Quit {
        code: 1,
        reason: message,
    })?;

    let default_config = home.join(DEFAULT_CONFIG_REL);
    let config_path = config_path.unwrap_or_else(|| default_config.clone());

    if mode == AppMode::Wizard {
        return wizard_runner(&config_path);
    }

    if !config_path.is_file() {
        if config_path_source_flag {
            eprintln!("Missing config file: {}", config_path.display());
        } else {
            eprintln!("{}", bootstrap_config_error(&default_config));
        }
        return Err(Quit {
            code: 1,
            reason: format!("missing_config:{}", config_path.display()),
        });
    }

    let loaded = load_config(&config_path).map_err(|message| Quit {
        code: 1,
        reason: message,
    })?;

    // Capture the absolute invocation working directory once for stable notification payloads.
    let invocation_folder = env::current_dir()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let mut logger = Logger::new(loaded.config.log_path.clone());

    if mode == AppMode::Doctor {
        return run_doctor_mode(&loaded.config, &config_path, &logger);
    }

    if matches!(
        loaded.config.hooks.effective_notification_scope(),
        Some(NotificationScope::AllLogs)
    ) {
        logger.configure_all_logs_notification(
            loaded.config.hooks.on_notification.as_deref(),
            &config_path,
            invocation_folder.clone(),
        );
    }

    if let Err(message) = validate_config(&loaded.config, &manual_tasks) {
        eprintln!("{}", message);
        return Err(quit(&logger, &message, 1));
    }

    let prompt_trudge = home.join(PROMPT_TRUDGE);
    let prompt_review = home.join(PROMPT_REVIEW);
    if let Err(message) = require_file(&prompt_trudge, "prompt file") {
        eprintln!("{}", message);
        return Err(quit(&logger, &message, 1));
    }
    if let Err(message) = require_file(&prompt_review, "prompt file") {
        eprintln!("{}", message);
        return Err(quit(&logger, &message, 1));
    }

    let prompt_trudge_content =
        render_prompt(&prompt_trudge).map_err(|message| quit(&logger, &message, 1))?;
    let prompt_review_content =
        render_prompt(&prompt_review).map_err(|message| quit(&logger, &message, 1))?;

    let interrupt_flag = Arc::new(AtomicBool::new(false));
    if let Err(err) = ctrlc::set_handler({
        let interrupt_flag = Arc::clone(&interrupt_flag);
        move || {
            interrupt_flag.store(true, Ordering::SeqCst);
        }
    }) {
        eprintln!("Failed to set interrupt handler: {}", err);
    }

    let mut state = RuntimeState {
        config: loaded.config,
        config_path,
        invocation_folder,
        prompt_trudge: prompt_trudge_content,
        prompt_review: prompt_review_content,
        logger,
        tmux: TmuxState::new(),
        interrupt_flag,
        manual_tasks,
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
        run_started_at: Instant::now(),
        current_task_started_at: None,
        run_exit_code: 0,
    };

    if env::var("TRUDGER_TEST_FORCE_ERR").is_ok() {
        return Err(quit(&state.logger, "error", 1));
    }

    if matches!(
        state.config.hooks.effective_notification_scope(),
        Some(NotificationScope::AllLogs)
    ) {
        state
            .logger
            .mark_all_logs_run_started_at(state.run_started_at);
    }

    dispatch_notification_hook(&state, None, NotificationEvent::RunStart);
    let result = run_loop(&mut state);
    reset_task_on_exit(&state, &result);
    finish_current_task_context(&mut state);
    state.tmux.restore();
    state.run_exit_code = result.as_ref().err().map(|quit| quit.code).unwrap_or(0);
    dispatch_notification_hook(&state, None, NotificationEvent::RunEnd);
    result
}

pub(crate) fn run_with_cli(cli: Cli) -> Result<(), Quit> {
    run_with_cli_impl(cli, run_wizard_cli)
}

#[cfg(test)]
pub(crate) fn run_with_cli_for_test<F>(cli: Cli, wizard_runner: F) -> Result<(), Quit>
where
    F: FnOnce(&Path) -> Result<(), Quit>,
{
    run_with_cli_impl(cli, wizard_runner)
}

pub(crate) fn run_with_args(args: Vec<OsString>) -> Result<(), Quit> {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            // clap's `Error::print()` uses termcolor and can bypass Rust's test output
            // capturing. Rendering it ourselves keeps CLI errors capture-friendly.
            eprintln!("{err}");
            return Err(Quit {
                code: err.exit_code(),
                reason: "cli_parse".to_string(),
            });
        }
    };
    run_with_cli(cli)
}

pub(crate) fn main_with_args(args: Vec<OsString>) -> ExitCode {
    let result = run_with_args(args);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(quit) => quit.exit_code(),
    }
}

pub(crate) fn main() -> ExitCode {
    main_with_args(env::args_os().collect())
}
