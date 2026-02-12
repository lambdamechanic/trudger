use serde::Serialize;
use std::io::Write;
use tempfile::NamedTempFile;

#[derive(Debug, Serialize)]
pub(crate) struct NotificationPayload {
    pub(crate) event: String,
    pub(crate) duration_ms: u128,
    pub(crate) folder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exit_code: Option<i32>,
    pub(crate) task_id: String,
    pub(crate) task_description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
}

impl NotificationPayload {
    pub(crate) fn write_to_temp_file(&self) -> Result<NamedTempFile, String> {
        let mut payload_file = NamedTempFile::new()
            .map_err(|err| format!("failed to create notification payload file: {err}"))?;
        serde_json::to_writer(&mut payload_file, self)
            .map_err(|err| format!("failed to serialize notification payload: {err}"))?;
        payload_file
            .write_all(b"\n")
            .map_err(|err| format!("failed to finalize notification payload file: {err}"))?;
        Ok(payload_file)
    }
}
