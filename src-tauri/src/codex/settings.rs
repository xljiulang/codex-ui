use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub codex_path: Option<String>,
    pub sound_enabled: bool,
    pub enter_to_send: bool,
    pub followup_mode: String,
    pub theme: String,
    /// 权限模式的启动初始值：ask-for-approval｜help-me-approve｜full-access
    #[serde(default = "default_permission")]
    pub default_permission: String,
}

fn default_permission() -> String {
    "ask-for-approval".into()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            codex_path: None,
            sound_enabled: true,
            enter_to_send: true,
            followup_mode: "adjust".into(),
            theme: "blue".into(),
            default_permission: default_permission(),
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
    // 先写临时文件再替换，避免写入中途崩溃留下截断的 settings.json
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    if p.exists() {
        fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_missing_default_permission_falls_back() {
        let dir = TempDir::new().unwrap();
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();

        let s = load(dir.path());
        assert_eq!(s.default_permission, "ask-for-approval");
    }

    #[test]
    fn save_load_roundtrip_preserves_default_permission() {
        let dir = TempDir::new().unwrap();
        let s = AppSettings {
            default_permission: "full-access".into(),
            ..AppSettings::default()
        };

        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.default_permission, "full-access");
        // 原子写不应残留临时文件
        assert!(!settings_path(dir.path())
            .with_extension("json.tmp")
            .exists());
    }
}
