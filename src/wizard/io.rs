#[cfg(test)]
use std::collections::VecDeque;

pub(super) trait WizardIo {
    fn write_out(&mut self, s: &str) -> Result<(), String>;
    fn write_err(&mut self, s: &str) -> Result<(), String>;
    fn flush_out(&mut self) -> Result<(), String>;
    fn read_line(&mut self) -> Result<Option<String>, String>;
}

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
