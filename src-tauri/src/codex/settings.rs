use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// "ask-for-approval" | "help-me-approve" | "full-access"
    pub permission_mode: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub codex_path: Option<String>,
    pub sound_enabled: bool,
    pub enter_to_send: bool,
    pub followup_mode: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            permission_mode: "ask-for-approval".into(),
            model: None,
            effort: None,
            codex_path: None,
            sound_enabled: true,
            enter_to_send: true,
            followup_mode: "adjust".into(),
        }
    }
}

pub fn settings_path(app_dir: &Path) -> std::path::PathBuf {
    app_dir.join("settings.json")
}

pub fn load(app_dir: &Path) -> AppSettings {
    let p = settings_path(app_dir);
    fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app_dir: &Path, s: &AppSettings) -> Result<(), String> {
    let p = settings_path(app_dir);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(&p, text).map_err(|e| e.to_string())
}
