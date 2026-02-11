use std::io::{self, IsTerminal, Write};
use std::path::Path;

use crate::run_loop::Quit;

use super::{run_wizard_with_io, WizardMergeMode, WizardResult};

pub(super) struct TerminalWizardIo {
    stdin: io::Stdin,
    stdout: io::Stdout,
    stderr: io::Stderr,
}

impl TerminalWizardIo {
    pub(super) fn new() -> Self {
        Self {
            stdin: io::stdin(),
            stdout: io::stdout(),
            stderr: io::stderr(),
        }
    }
}

impl super::io::WizardIo for TerminalWizardIo {
    fn write_out(&mut self, s: &str) -> Result<(), String> {
        self.stdout
            .write_all(s.as_bytes())
            .map_err(|err| format!("Failed to write stdout: {}", err))
    }

    fn write_err(&mut self, s: &str) -> Result<(), String> {
        self.stderr
            .write_all(s.as_bytes())
            .map_err(|err| format!("Failed to write stderr: {}", err))
    }

    fn flush_out(&mut self) -> Result<(), String> {
        self.stdout
            .flush()
            .map_err(|err| format!("Failed to flush stdout: {}", err))
    }

    fn read_line(&mut self) -> Result<Option<String>, String> {
        let mut input = String::new();
        let bytes = self
            .stdin
            .read_line(&mut input)
            .map_err(|err| format!("Failed to read selection: {}", err))?;
        if bytes == 0 {
            Ok(None)
        } else {
            Ok(Some(input))
        }
    }
}

pub(crate) fn run_wizard_interactive(config_path: &Path) -> Result<WizardResult, String> {
    let templates = super::load_embedded_wizard_templates()?;
    let mut io = TerminalWizardIo::new();
    run_wizard_with_io(
        config_path,
        &templates,
        WizardMergeMode::Interactive,
        &mut io,
    )
}

pub(crate) fn run_wizard_cli(config_path: &Path) -> Result<(), Quit> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        let message =
            "trudger wizard requires an interactive terminal (stdin and stdout must be a TTY)."
                .to_string();
        eprintln!("{}", message);
        return Err(Quit {
            code: 1,
            reason: message,
        });
    }

    let result = run_wizard_interactive(config_path).map_err(|message| Quit {
        code: 1,
        reason: message,
    })?;

    for warning in result.warnings {
        eprintln!("{}", warning);
    }

    println!("Wrote config to {}", result.config_path.display());
    if let Some(backup) = result.backup_path {
        println!("Backup: {}", backup.display());
    }

    Ok(())
}
