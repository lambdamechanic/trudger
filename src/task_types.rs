use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::num::NonZeroU64;

const TASK_ID_MAX_LEN: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskIdError {
    Empty,
    TooLong { max: usize, len: usize },
    InvalidStart { ch: char },
    InvalidChar { ch: char },
}

impl fmt::Display for TaskIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => f.write_str("task_id must not be empty"),
            Self::TooLong { max, len } => {
                write!(f, "task_id must be at most {} characters (got {})", max, len)
            }
            Self::InvalidStart { .. } => f.write_str("task_id must start with an ASCII letter or digit"),
            Self::InvalidChar { ch } => write!(
                f,
                "task_id contains invalid character {:?}; allowed: ASCII letters/digits plus '-', '_', '.', ':'",
                ch
            ),
        }
    }
}

impl std::error::Error for TaskIdError {}

#[cfg(test)]
mod tests {
    use super::TaskIdError;

    #[test]
    fn task_id_error_display_formats() {
        assert_eq!(TaskIdError::Empty.to_string(), "task_id must not be empty");
        assert_eq!(
            TaskIdError::TooLong { max: 200, len: 201 }.to_string(),
            "task_id must be at most 200 characters (got 201)"
        );

        // Display doesn't currently include the offending char for InvalidStart; still cover it.
        assert_eq!(
            TaskIdError::InvalidStart { ch: '-' }.to_string(),
            "task_id must start with an ASCII letter or digit"
        );

        let rendered = TaskIdError::InvalidChar { ch: '$' }.to_string();
        assert!(rendered.contains("invalid character"));
        assert!(rendered.contains('$'));
        assert!(rendered.contains("allowed: ASCII letters/digits"));
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct TaskId(String);

impl TaskId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TaskId {
    type Error = TaskIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(TaskIdError::Empty);
        }
        if trimmed.len() > TASK_ID_MAX_LEN {
            return Err(TaskIdError::TooLong {
                max: TASK_ID_MAX_LEN,
                len: trimmed.len(),
            });
        }

        debug_assert!(!trimmed.is_empty());
        let mut chars = trimmed.chars();
        let first = chars.next().unwrap();
        if !first.is_ascii_alphanumeric() {
            return Err(TaskIdError::InvalidStart { ch: first });
        }

        for ch in chars {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':') {
                continue;
            }
            return Err(TaskIdError::InvalidChar { ch });
        }
        Ok(Self(trimmed.to_string()))
    }
}

impl TryFrom<&str> for TaskId {
    type Error = TaskIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_string())
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskStatus {
    Ready,
    Open,
    InProgress,
    Closed,
    Blocked,
    Unknown(String),
}

impl TaskStatus {
    pub(crate) fn parse(token: &str) -> Option<Self> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(match trimmed {
            "ready" => Self::Ready,
            "open" => Self::Open,
            "in_progress" => Self::InProgress,
            "closed" => Self::Closed,
            "blocked" => Self::Blocked,
            other => Self::Unknown(other.to_string()),
        })
    }

    pub(crate) fn is_ready(&self) -> bool {
        matches!(self, Self::Ready | Self::Open)
    }

    pub(crate) fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Ready => "ready",
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Closed => "closed",
            Self::Blocked => "blocked",
            Self::Unknown(value) => value,
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Solving,
    Reviewing,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ReviewLoopLimit(NonZeroU64);

impl ReviewLoopLimit {
    pub(crate) fn new(value: u64) -> Result<Self, String> {
        let Some(value) = NonZeroU64::new(value) else {
            return Err("must be a positive integer (got 0)".to_string());
        };
        Ok(Self(value))
    }

    pub(crate) fn get(self) -> u64 {
        self.0.get()
    }
}

impl<'de> Deserialize<'de> for ReviewLoopLimit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        ReviewLoopLimit::new(value).map_err(D::Error::custom)
    }
}

impl fmt::Display for ReviewLoopLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}
