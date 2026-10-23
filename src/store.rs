use std::path::Path;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::api::models::Issue;

/// OAuth token, persisted with 0600 permissions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Tokens {
    pub access_token: String,
}

/// A timer that survives app restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTimer {
    pub issue_key: String,
    pub summary: String,
    pub description: String,
    pub started_at: DateTime<Local>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_timer: Option<ActiveTimer>,
    /// Pinned tasks for quick tracking, added via Tracker search.
    #[serde(default)]
    pub favorites: Vec<Issue>,
}

pub fn set_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

impl PersistedState {
    pub fn load() -> Self {
        let path = crate::config::state_path();
        if !path.exists() {
            return Self::default();
        }
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = crate::config::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, &raw)?;
        set_private(&path);
        Ok(())
    }
}

/// Timer details for the stop-and-log review modal.
#[derive(Debug, Clone)]
pub struct RunningTimer {
    pub issue: Issue,
    pub description: String,
    pub started_at: DateTime<Local>,
}

impl RunningTimer {
    pub fn elapsed(&self) -> std::time::Duration {
        (Local::now() - self.started_at)
            .to_std()
            .unwrap_or_default()
    }
}
