use std::io::{self, Write};

pub(super) trait WizardIo {
    fn write_out(&mut self, s: &str) -> Result<(), String>;
    fn write_err(&mut self, s: &str) -> Result<(), String>;
    fn flush_out(&mut self) -> Result<(), String>;
    fn read_line(&mut self) -> Result<Option<String>, String>;
}

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

impl WizardIo for TerminalWizardIo {
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

#[cfg(test)]
use std::collections::VecDeque;

#[cfg(test)]
pub(super) struct TestWizardIo {
    inputs: VecDeque<String>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

#[cfg(test)]
impl TestWizardIo {
    pub(super) fn new(inputs: Vec<String>) -> Self {
        Self {
            inputs: inputs.into(),
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

#[cfg(test)]
impl WizardIo for TestWizardIo {
    fn write_out(&mut self, s: &str) -> Result<(), String> {
        self.stdout.push_str(s);
        Ok(())
    }

    fn write_err(&mut self, s: &str) -> Result<(), String> {
        self.stderr.push_str(s);
        Ok(())
    }

    fn flush_out(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn read_line(&mut self) -> Result<Option<String>, String> {
        Ok(self.inputs.pop_front())
    }
}
