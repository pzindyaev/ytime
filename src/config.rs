use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoundingMode {
    Up,
    Nearest,
    Down,
}

impl RoundingMode {
    pub fn cycle(self) -> Self {
        match self {
            RoundingMode::Up => RoundingMode::Nearest,
            RoundingMode::Nearest => RoundingMode::Down,
            RoundingMode::Down => RoundingMode::Up,
        }
    }
}

impl fmt::Display for RoundingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RoundingMode::Up => write!(f, "up"),
            RoundingMode::Nearest => write!(f, "nearest"),
            RoundingMode::Down => write!(f, "down"),
        }
    }
}

/// Which header carries the organization id. `Auto` tries `X-Org-Id` then
/// `X-Cloud-Org-Id` and persists the working choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrgHeader {
    XOrgId,
    XCloudOrgId,
}

impl OrgHeader {
    pub fn header_name(self) -> &'static str {
        match self {
            OrgHeader::XOrgId => "X-Org-Id",
            OrgHeader::XCloudOrgId => "X-Cloud-Org-Id",
        }
    }
}

impl fmt::Display for OrgHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.header_name())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RoundingConfig {
    /// Rounding unit in minutes.
    pub minutes: u32,
    pub mode: RoundingMode,
}

impl Default for RoundingConfig {
    fn default() -> Self {
        Self {
            minutes: 30,
            mode: RoundingMode::Up,
        }
    }
}

impl RoundingConfig {
    pub const UNITS: [u32; 5] = [1, 5, 15, 30, 60];

    pub fn cycle_unit(&mut self) {
        let idx = Self::UNITS
            .iter()
            .position(|&u| u == self.minutes)
            .unwrap_or(4);
        self.minutes = Self::UNITS[(idx + 1) % Self::UNITS.len()];
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// OAuth application id registered at https://oauth.yandex.ru
    pub client_id: Option<String>,
    /// App secret — Yandex requires it at the token exchange for most
    /// registered apps ("Wrong client secret" otherwise).
    pub client_secret: Option<String>,
    /// Organization id (Tracker → Administration → Organizations).
    pub org_id: Option<String>,
    pub rounding: RoundingConfig,
    /// Override if auto-detection picks the wrong one.
    pub org_header: Option<OrgHeader>,
    pub api_base: String,
    pub oauth_base: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            client_id: None,
            client_secret: None,
            org_id: None,
            rounding: RoundingConfig::default(),
            org_header: None,
            api_base: "https://api.tracker.yandex.net".to_string(),
            oauth_base: "https://oauth.yandex.ru".to_string(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "ytime")
        .expect("no home directory")
        .config_dir()
        .to_path_buf()
}

pub fn state_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "ytime")
        .expect("no home directory")
        .data_local_dir()
        .join("state.json")
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        let cfg: Config = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(&path, raw)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let cfg = Config {
            client_id: Some("abc".into()),
            org_id: Some("12345".into()),
            rounding: RoundingConfig {
                minutes: 15,
                mode: RoundingMode::Nearest,
            },
            org_header: Some(OrgHeader::XCloudOrgId),
            ..Config::default()
        };
        let raw = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&raw).unwrap();
        assert_eq!(back.client_id.as_deref(), Some("abc"));
        assert_eq!(back.org_id.as_deref(), Some("12345"));
        assert_eq!(back.rounding.minutes, 15);
        assert_eq!(back.rounding.mode, RoundingMode::Nearest);
        assert_eq!(back.org_header, Some(OrgHeader::XCloudOrgId));
        assert_eq!(back.api_base, "https://api.tracker.yandex.net");
    }

    #[test]
    fn partial_toml_falls_back_to_defaults() {
        let cfg: Config = toml::from_str("client_id = \"x\"").unwrap();
        assert_eq!(cfg.client_id.as_deref(), Some("x"));
        assert_eq!(cfg.rounding.minutes, 30);
    }
}
