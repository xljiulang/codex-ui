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
    /// 被禁用的动态工具（`namespace.tool`，如 codexui.get_usage）；空 = 全部启用
    #[serde(default)]
    pub dynamic_tools_disabled: Vec<String>,
    /// 毛玻璃特效（Windows 11 Mica / Windows 10 Acrylic 窗口背景），默认开启
    #[serde(default = "default_glass_effect")]
    pub glass_effect: bool,
    /// 最后活跃会话 id：退出后留档，下次启动恢复该会话（None = 无记录，启动开设置页）
    #[serde(default)]
    pub last_session_id: Option<String>,
    /// Zen 本地代理开关（默认关闭）
    #[serde(default)]
    pub zen_proxy_enabled: bool,
    /// Zen 本地代理监听端口（默认 18080）
    #[serde(default = "default_zen_proxy_port")]
    pub zen_proxy_port: u16,
    /// Zen 代理转发上游 API 请求地址（默认 opencode.ai/zen/v1）
    #[serde(default = "default_zen_proxy_base_url")]
    pub zen_proxy_base_url: String,
}

fn default_zen_proxy_port() -> u16 {
    crate::codex::zen_proxy::DEFAULT_ZEN_PROXY_PORT
}

fn default_zen_proxy_base_url() -> String {
    crate::codex::zen_proxy::DEFAULT_ZEN_BASE_URL.to_string()
}

fn default_permission() -> String {
    "ask-for-approval".into()
}

fn default_terminal_shell() -> String {
    "cmd".into()
}

fn default_glass_effect() -> bool {
    true
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
            dynamic_tools_disabled: Vec::new(),
            glass_effect: default_glass_effect(),
            last_session_id: None,
            zen_proxy_enabled: false,
            zen_proxy_port: default_zen_proxy_port(),
            zen_proxy_base_url: default_zen_proxy_base_url(),
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

/// 毛玻璃深色档：light 主题用浅色 Mica，其余（blue/dark）用深色，与应用主题一致
pub fn glass_theme_dark(theme: &str) -> bool {
    theme != "light"
}

/// 毛玻璃回退着色基色（Acrylic tint），与主题背景一致：
/// dark→曜黑、light→晨光、其它含缺省→蓝夜
pub fn glass_tint_rgb(theme: &str) -> (u8, u8, u8) {
    let (r, g, b, _) = theme_background_rgba(theme);
    (r, g, b)
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
    fn dynamic_tools_disabled_defaults_empty_and_roundtrips() {
        let dir = TempDir::new().unwrap();
        // 缺失字段回退空（全部启用）
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();
        let s = load(dir.path());
        assert!(s.dynamic_tools_disabled.is_empty());

        // 保存自定义禁用列表往返一致
        let s = AppSettings {
            dynamic_tools_disabled: vec!["codexui.get_usage".into()],
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.dynamic_tools_disabled, vec!["codexui.get_usage"]);
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

    #[test]
    fn glass_effect_defaults_true_and_roundtrips() {
        let dir = TempDir::new().unwrap();
        // 旧 settings.json 缺失 glass_effect：应回退默认 true（毛玻璃开启）
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();
        assert!(load(dir.path()).glass_effect);

        // 显式关闭与开启均可往返一致
        let s = AppSettings {
            glass_effect: false,
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        assert!(!load(dir.path()).glass_effect);

        let s = AppSettings::default();
        save(dir.path(), &s).unwrap();
        assert!(load(dir.path()).glass_effect);
    }

    #[test]
    fn glass_helpers_follow_theme() {
        assert!(glass_theme_dark("blue"));
        assert!(glass_theme_dark("dark"));
        assert!(!glass_theme_dark("light"));
        assert_eq!(glass_tint_rgb("blue"), (0x0e, 0x11, 0x16));
        assert_eq!(glass_tint_rgb("dark"), (0x0a, 0x0b, 0x10));
        assert_eq!(glass_tint_rgb("light"), (0xf4, 0xf6, 0xfb));
        assert_eq!(glass_tint_rgb(""), (0x0e, 0x11, 0x16));
    }

    #[test]
    fn last_session_id_defaults_none_and_roundtrips() {
        let dir = TempDir::new().unwrap();
        // 旧 settings.json 缺失 last_session_id：应回退 None（启动开设置页）
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();
        assert_eq!(load(dir.path()).last_session_id, None);

        // 保存会话 id 后往返一致；显式置回 None 同样往返
        let s = AppSettings {
            last_session_id: Some("thr-abc".into()),
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        assert_eq!(
            load(dir.path()).last_session_id.as_deref(),
            Some("thr-abc")
        );

        let s = AppSettings::default();
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()).last_session_id, None);
    }

    #[test]
    fn zen_proxy_defaults_and_roundtrips() {
        let dir = TempDir::new().unwrap();
        // 旧 settings.json 缺失 zen_proxy 字段：回退关闭 + 默认端口
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();
        let s = load(dir.path());
        assert!(!s.zen_proxy_enabled);
        assert_eq!(s.zen_proxy_port, crate::codex::zen_proxy::DEFAULT_ZEN_PROXY_PORT);
        assert_eq!(
            s.zen_proxy_base_url,
            crate::codex::zen_proxy::DEFAULT_ZEN_BASE_URL
        );

        // 自定义端口与开关往返一致
        let s = AppSettings {
            zen_proxy_enabled: true,
            zen_proxy_port: 19090,
            zen_proxy_base_url: "https://custom.example.com/v1".into(),
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path());
        assert!(loaded.zen_proxy_enabled);
        assert_eq!(loaded.zen_proxy_port, 19090);
        assert_eq!(loaded.zen_proxy_base_url, "https://custom.example.com/v1");
    }
}
