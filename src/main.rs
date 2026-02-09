use std::process::ExitCode;

mod app;
mod cli;
mod config;
mod doctor;
mod logger;
mod prompt_defaults;
mod run_loop;
mod shell;
mod task_types;
mod tmux;
mod wizard;
mod wizard_templates;

#[cfg(test)]
mod unit_tests;

fn main() -> ExitCode {
    app::main()
}
