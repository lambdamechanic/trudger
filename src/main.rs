use std::process::ExitCode;

mod app;
mod cli;
mod config;
mod doctor;
mod logger;
mod run_loop;
mod shell;
mod tmux;

#[cfg(test)]
mod unit_tests;

fn main() -> ExitCode {
    app::main()
}
