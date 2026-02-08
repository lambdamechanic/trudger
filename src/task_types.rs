use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::num::NonZeroU64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct TaskId(String);

impl TaskId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TaskId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err("task_id must not be empty".to_string());
        }
        Ok(Self(trimmed.to_string()))
    }
}

impl TryFrom<&str> for TaskId {
    type Error = String;

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
