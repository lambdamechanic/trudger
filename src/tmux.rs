use std::env;
use std::path::Path;
use std::process::Command;

use crate::shell::command_exists;

#[derive(Debug, Clone)]
pub(crate) struct TmuxState {
    enabled: bool,
    base_name: String,
    original_title: String,
}

impl TmuxState {
    pub(crate) fn new() -> Self {
        let enabled = env::var("TMUX").is_ok() && command_exists("tmux");
        if !enabled {
            return Self::disabled();
        }

        let session_name = env::var("TRUDGER_TMUX_SESSION_NAME")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| tmux_display("#S"));
        if let Some(value) = &session_name {
            env::set_var("TRUDGER_TMUX_SESSION_NAME", value);
        }

        let original_title = env::var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| tmux_display("#{pane_title}"))
            .unwrap_or_default();
        if !original_title.is_empty() {
            env::set_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE", &original_title);
        }

        let mut base_name = strip_trudger_title_suffixes(&original_title);

        if base_name.trim().is_empty() {
            base_name = default_tmux_base_name();
        }

        let state = Self {
            enabled: true,
            base_name,
            original_title,
        };
        state.select_pane(&state.base_name);
        state
    }

    pub(crate) fn disabled() -> Self {
        Self {
            enabled: false,
            base_name: String::new(),
            original_title: String::new(),
        }
    }

    fn select_pane(&self, name: &str) {
        if !self.enabled {
            return;
        }
        let _ = Command::new("tmux")
            .arg("select-pane")
            .arg("-T")
            .arg(name)
            .status();
    }

    pub(crate) fn update_name(
        &self,
        phase: &str,
        task_id: &str,
        completed: &[String],
        needs_human: &[String],
    ) {
        if !self.enabled {
            return;
        }
        let name = build_tmux_name(&self.base_name, phase, task_id, completed, needs_human);
        self.select_pane(&name);
    }

    pub(crate) fn restore(&self) {
        if !self.enabled {
            return;
        }
        if !self.original_title.is_empty() {
            self.select_pane(&self.original_title);
        }
    }
}

fn strip_trudger_title_suffixes(title: &str) -> String {
    // Trudger appends these segments when updating the pane title.
    const MARKERS: &[&str] = &[
        " COMPLETED [",
        " NEEDS_HUMAN [",
        " SOLVING ",
        " REVIEWING ",
        " HALTED ON ERROR ",
    ];

    let mut cut = title.len();
    for marker in MARKERS {
        if let Some(idx) = title.find(marker) {
            cut = cut.min(idx);
        }
    }

    title[..cut].to_string()
}

fn tmux_display(format: &str) -> Option<String> {
    let output = Command::new("tmux")
        .arg("display-message")
        .arg("-p")
        .arg(format)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn default_tmux_base_name() -> String {
    let host = Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .or_else(|| {
            Command::new("hostname").output().ok().and_then(|output| {
                if output.status.success() {
                    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "host".to_string());

    let folder = env::current_dir()
        .ok()
        .and_then(|path| path.file_name().map(|v| v.to_string_lossy().to_string()))
        .unwrap_or_default();
    // `env::args().next()` should always yield argv[0], but prefer a concrete
    // fallback over `unwrap_or_else` so coverage doesn't depend on an
    // effectively-unreachable closure.
    let command = env::args().next().unwrap_or("trudger".to_string());
    let command = Path::new(&command)
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or(command);
    format!("({}) {}: {}", host, folder, command)
}

fn format_task_list(label: &str, tasks: &[String]) -> String {
    if tasks.is_empty() {
        return String::new();
    }
    format!("{} [{}]", label, tasks.join(", "))
}

pub(crate) fn build_tmux_name(
    base_name: &str,
    phase: &str,
    task_id: &str,
    completed: &[String],
    needs_human: &[String],
) -> String {
    let mut base = base_name.to_string();
    if let Some((prefix, command)) = base_name.rsplit_once(": ") {
        if command == "fg" || command == "codex" {
            base = prefix.to_string();
        }
    }

    let activity = match phase {
        "SOLVING" => format!("SOLVING {}", task_id),
        "REVIEWING" => format!("REVIEWING {}", task_id),
        "ERROR" => format!("HALTED ON ERROR {}", task_id),
        _ => String::new(),
    };

    let mut parts = Vec::new();
    parts.push(base);
    let completed_segment = format_task_list("COMPLETED", completed);
    let needs_human_segment = format_task_list("NEEDS_HUMAN", needs_human);
    if !completed_segment.is_empty() {
        parts.push(completed_segment);
    }
    if !needs_human_segment.is_empty() {
        parts.push(needs_human_segment);
    }
    if !activity.is_empty() {
        parts.push(activity);
    }

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_pane_is_noop_when_disabled() {
        let state = TmuxState::disabled();
        state.select_pane("name");
    }
}
