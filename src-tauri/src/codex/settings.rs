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
    /// 权限模式的启动初始值：read-only｜ask-for-approval｜help-me-approve｜full-access
    #[serde(default = "default_permission")]
    pub default_permission: String,
    /// 终端 Shell：cmd（命令提示符，默认）｜powershell
    #[serde(default = "default_terminal_shell")]
    pub terminal_shell: String,
}

fn default_permission() -> String {
    "ask-for-approval".into()
}

fn default_terminal_shell() -> String {
    "cmd".into()
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
        terminal_shell: default_terminal_shell(),
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

/// 主题对应的启动窗口背景色（RGBA）：dark→曜黑、light→晨光、其它含缺省→蓝夜。
/// 用于原生窗口在 Web 内容渲染前就与保存的主题配色一致，避免启动白屏/错色。
pub fn theme_background_rgba(theme: &str) -> (u8, u8, u8, u8) {
    match theme {
        "dark" => (0x0a, 0x0b, 0x10, 0xff),
        "light" => (0xf4, 0xf6, 0xfb, 0xff),
        _ => (0x0e, 0x11, 0x16, 0xff),
    }
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
        assert_eq!(s.terminal_shell, "cmd");
    }

    #[test]
    fn save_load_roundtrip_preserves_custom_settings() {
        let dir = TempDir::new().unwrap();
        let s = AppSettings {
            default_permission: "full-access".into(),
            terminal_shell: "powershell".into(),
            ..AppSettings::default()
        };

        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.default_permission, "full-access");
        assert_eq!(loaded.terminal_shell, "powershell");
        // 原子写不应残留临时文件
        assert!(!settings_path(dir.path())
            .with_extension("json.tmp")
            .exists());
    }

    #[test]
    fn theme_background_matches_theme_palette() {
        assert_eq!(theme_background_rgba("dark"), (0x0a, 0x0b, 0x10, 0xff));
        assert_eq!(theme_background_rgba("light"), (0xf4, 0xf6, 0xfb, 0xff));
        assert_eq!(theme_background_rgba("blue"), (0x0e, 0x11, 0x16, 0xff));
    }

    #[test]
    fn theme_background_falls_back_for_unknown_theme() {
        assert_eq!(theme_background_rgba(""), (0x0e, 0x11, 0x16, 0xff));
        assert_eq!(theme_background_rgba("neon"), (0x0e, 0x11, 0x16, 0xff));
    }
}
