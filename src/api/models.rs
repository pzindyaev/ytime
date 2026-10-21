// Mirrors of Yandex Tracker JSON — some fields are decoded but not used by
// the UI yet; keeping them documents the API shape.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

fn de_default_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<String> = Option::deserialize(d)?;
    Ok(v.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(default, deserialize_with = "de_default_string")]
    pub login: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default)]
    pub uid: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusRef {
    #[serde(default, deserialize_with = "de_default_string")]
    pub key: String,
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRef {
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default)]
    pub login: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Issue {
    #[serde(default, deserialize_with = "de_default_string")]
    pub key: String,
    #[serde(default, deserialize_with = "de_default_string")]
    pub summary: String,
    #[serde(default)]
    pub status: Option<StatusRef>,
    /// Issue type (Bug, Task, Epic…) — same shape as status.
    #[serde(rename = "type", default)]
    pub issue_type: Option<StatusRef>,
    #[serde(default)]
    pub assignee: Option<UserRef>,
    /// Raw Tracker timestamp (`YYYY-MM-DDThh:mm:ss.sss±hhmm`), parsed lazily.
    #[serde(rename = "updatedAt", default)]
    pub updated_at: Option<String>,
}

impl Issue {
    pub fn status_display(&self) -> &str {
        self.status
            .as_ref()
            .and_then(|s| s.display.as_deref())
            .unwrap_or("—")
    }

    pub fn type_display(&self) -> &str {
        self.issue_type
            .as_ref()
            .and_then(|t| t.display.as_deref())
            .unwrap_or("—")
    }

    pub fn type_key(&self) -> &str {
        self.issue_type
            .as_ref()
            .map(|t| t.key.as_str())
            .unwrap_or("")
    }

    pub fn title(&self) -> String {
        format!("{} {}", self.key, self.summary)
    }
}

/// Body of `POST /v2/issues/{key}/worklog`.
#[derive(Debug, Serialize)]
pub struct WorklogPost {
    pub start: String,
    /// ISO-8601 duration, e.g. `PT1H30M`.
    pub duration: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRef {
    #[serde(default, deserialize_with = "de_default_string")]
    pub key: String,
    #[serde(default)]
    pub display: Option<String>,
}

/// A worklog record as returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorklogRecord {
    #[serde(default)]
    pub issue: Option<IssueRef>,
    /// Work start, raw Tracker timestamp.
    #[serde(default, deserialize_with = "de_default_string")]
    pub start: String,
    /// ISO-8601 duration.
    #[serde(default, deserialize_with = "de_default_string")]
    pub duration: String,
    #[serde(default)]
    pub comment: Option<String>,
}

impl WorklogRecord {
    pub fn issue_key(&self) -> &str {
        self.issue.as_ref().map(|i| i.key.as_str()).unwrap_or("?")
    }

    pub fn comment_or_empty(&self) -> &str {
        self.comment.as_deref().unwrap_or("")
    }
}

/// Parses Tracker timestamps (`2024-05-20T10:00:00.000+0300` or RFC3339).
pub fn parse_dt(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let s = s.trim();
    chrono::DateTime::parse_from_rfc3339(s)
        .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .ok()
}
