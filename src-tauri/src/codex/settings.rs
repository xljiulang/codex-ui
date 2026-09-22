use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub codex_path: Option<String>,
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
    /// 兼容代理开关（默认关闭）
    #[serde(default)]
    pub compat_proxy_enabled: bool,
    /// 兼容代理监听端口（默认 18080）
    #[serde(default = "default_compat_proxy_port")]
    pub compat_proxy_port: u16,
    /// 兼容代理转发上游 API 请求地址（默认 opencode.ai/zen/v1）
    #[serde(default = "default_compat_proxy_base_url")]
    pub compat_proxy_base_url: String,
    /// 兼容代理「回合收尾约束和助推」（口嗨检测 + 自动续跑 + 首轮教学 + 标签剥离），默认开启
    #[serde(default = "default_compat_proxy_nudge_enabled")]
    pub compat_proxy_nudge_enabled: bool,
    /// 兼容代理「OpenCode 客户端身份」（识别头 + opencode User-Agent + 免费层请求体门禁补丁；
    /// 勾选即对所有上游一律生效，不再判断 host），默认开启
    #[serde(default = "default_compat_proxy_identity_enabled")]
    pub compat_proxy_identity_enabled: bool,
    /// codex 错误发 Windows 系统通知（窗口无前台焦点时），默认开启
    #[serde(default = "default_error_notify_enabled")]
    pub error_notify_enabled: bool,
    /// 会话提权/交互/完成（审批、提问、MCP 表单、计划就绪、回合正常完成）
    /// 发 Windows 系统通知，默认开启
    #[serde(default = "default_interaction_notify_enabled")]
    pub interaction_notify_enabled: bool,
}

fn default_compat_proxy_port() -> u16 {
    crate::codex::compat_proxy::DEFAULT_COMPAT_PROXY_PORT
}

fn default_compat_proxy_base_url() -> String {
    crate::codex::compat_proxy::DEFAULT_COMPAT_BASE_URL.to_string()
}

fn default_compat_proxy_nudge_enabled() -> bool {
    true
}

fn default_compat_proxy_identity_enabled() -> bool {
    true
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

fn default_error_notify_enabled() -> bool {
    true
}

fn default_interaction_notify_enabled() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            codex_path: None,
            enter_to_send: true,
            followup_mode: "adjust".into(),
            theme: "blue".into(),
            default_permission: default_permission(),
            terminal_shell: default_terminal_shell(),
            dynamic_tools_disabled: Vec::new(),
            glass_effect: default_glass_effect(),
            last_session_id: None,
            compat_proxy_enabled: false,
            compat_proxy_port: default_compat_proxy_port(),
            compat_proxy_base_url: default_compat_proxy_base_url(),
            compat_proxy_nudge_enabled: default_compat_proxy_nudge_enabled(),
            compat_proxy_identity_enabled: default_compat_proxy_identity_enabled(),
            error_notify_enabled: default_error_notify_enabled(),
            interaction_notify_enabled: default_interaction_notify_enabled(),
        }
    }
}

pub fn settings_path(app_dir: &Path) -> std::path::PathBuf {
    app_dir.join("settings.json")
}

/// 改名兼容：把旧键 `zen_proxy_*` 的值搬到新键 `compat_proxy_*`。
///
/// 2026-09-22 把 Zen 代理改名为「兼容代理」，五个设置键随之改名。这里做**读兼容**：
/// 旧 `settings.json` 里只有旧键时，先原地改名为新键再解析（下次 `save` 自然只写新键）；
/// 新旧键并存时以**新键**为准，避免覆盖用户已经用新版本保存过的值；两个都没有时不动，
/// 由各字段的 `serde(default)` 回退默认值。非对象 JSON 或读取失败时原样返回，交给既有
/// 「解析失败即回退默认」的路径处理。
fn migrate_legacy_zen_proxy_keys(raw: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.to_string();
    };
    let Some(obj) = value.as_object_mut() else {
        return raw.to_string();
    };
    let mut changed = false;
    for (old, new) in [
        ("zen_proxy_enabled", "compat_proxy_enabled"),
        ("zen_proxy_port", "compat_proxy_port"),
        ("zen_proxy_base_url", "compat_proxy_base_url"),
        ("zen_proxy_nudge_enabled", "compat_proxy_nudge_enabled"),
        (
            "zen_proxy_identity_enabled",
            "compat_proxy_identity_enabled",
        ),
    ] {
        if let Some(legacy) = obj.remove(old) {
            // 新键已存在时优先保留新值（丢弃旧值），两者都没有则直接搬过来
            obj.entry(new).or_insert(legacy);
            changed = true;
        }
    }
    if !changed {
        return raw.to_string();
    }
    serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string())
}

pub fn load(app_dir: &Path) -> AppSettings {
    let p = settings_path(app_dir);
    fs::read_to_string(&p)
        .ok()
        .map(|s| migrate_legacy_zen_proxy_keys(&s))
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
    fn error_notify_defaults_true_and_roundtrips() {
        let dir = TempDir::new().unwrap();
        // 旧 settings.json 缺失 error_notify_enabled：应回退默认 true（错误发系统通知）
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();
        assert!(load(dir.path()).error_notify_enabled);

        // 显式关闭与开启均可往返一致
        let s = AppSettings {
            error_notify_enabled: false,
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        assert!(!load(dir.path()).error_notify_enabled);

        let s = AppSettings::default();
        save(dir.path(), &s).unwrap();
        assert!(load(dir.path()).error_notify_enabled);
    }

    #[test]
    fn interaction_notify_defaults_true_and_roundtrips() {
        let dir = TempDir::new().unwrap();
        // 旧 settings.json 缺失 interaction_notify_enabled：应回退默认 true（交互发系统通知）
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();
        assert!(load(dir.path()).interaction_notify_enabled);

        // 显式关闭与开启均可往返一致
        let s = AppSettings {
            interaction_notify_enabled: false,
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        assert!(!load(dir.path()).interaction_notify_enabled);

        let s = AppSettings::default();
        save(dir.path(), &s).unwrap();
        assert!(load(dir.path()).interaction_notify_enabled);
    }

    #[test]
    fn legacy_sound_enabled_key_is_ignored() {
        let dir = TempDir::new().unwrap();
        // 已移除的提示音开关留在旧 settings.json 里：解析必须容忍该未知键，
        // 同文件里的其它字段照常生效（否则 load 会整体回退默认值）
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":false,"enter_to_send":false,"followup_mode":"queue","theme":"dark"}"#,
        )
        .unwrap();
        let s = load(dir.path());
        assert!(!s.enter_to_send);
        assert_eq!(s.followup_mode, "queue");
        assert_eq!(s.theme, "dark");
        assert!(s.interaction_notify_enabled);
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
    fn compat_proxy_defaults_and_roundtrips() {
        let dir = TempDir::new().unwrap();
        // 旧 settings.json 缺失 compat_proxy 字段：回退关闭 + 默认端口
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();
        let s = load(dir.path());
        assert!(!s.compat_proxy_enabled);
        assert_eq!(
            s.compat_proxy_port,
            crate::codex::compat_proxy::DEFAULT_COMPAT_PROXY_PORT
        );
        assert_eq!(
            s.compat_proxy_base_url,
            crate::codex::compat_proxy::DEFAULT_COMPAT_BASE_URL
        );
        // 两个行为开关同样缺失：回退默认开启（行为与旧版本一致）
        assert!(s.compat_proxy_nudge_enabled);
        assert!(s.compat_proxy_identity_enabled);

        // 自定义端口与开关往返一致
        let s = AppSettings {
            compat_proxy_enabled: true,
            compat_proxy_port: 19090,
            compat_proxy_base_url: "https://custom.example.com/v1".into(),
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path());
        assert!(loaded.compat_proxy_enabled);
        assert_eq!(loaded.compat_proxy_port, 19090);
        assert_eq!(
            loaded.compat_proxy_base_url,
            "https://custom.example.com/v1"
        );
        assert!(loaded.compat_proxy_nudge_enabled);
        assert!(loaded.compat_proxy_identity_enabled);
    }

    #[test]
    fn compat_proxy_behavior_switches_default_true_and_roundtrip() {
        let dir = TempDir::new().unwrap();
        // 旧 settings.json 缺失两个行为开关：回退默认开启
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"sound_enabled":true,"enter_to_send":true,"followup_mode":"adjust","theme":"blue"}"#,
        )
        .unwrap();
        let s = load(dir.path());
        assert!(s.compat_proxy_nudge_enabled);
        assert!(s.compat_proxy_identity_enabled);

        // 显式关闭与开启均可往返一致
        let s = AppSettings {
            compat_proxy_nudge_enabled: false,
            compat_proxy_identity_enabled: false,
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path());
        assert!(!loaded.compat_proxy_nudge_enabled);
        assert!(!loaded.compat_proxy_identity_enabled);

        let s = AppSettings {
            compat_proxy_nudge_enabled: true,
            compat_proxy_identity_enabled: false,
            ..AppSettings::default()
        };
        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path());
        assert!(loaded.compat_proxy_nudge_enabled);
        assert!(!loaded.compat_proxy_identity_enabled);
    }

    /// 改名兼容：旧键 `zen_proxy_*` 仍能被读入（写回时只写新键）。
    #[test]
    fn legacy_zen_proxy_keys_are_still_read() {
        let dir = TempDir::new().unwrap();
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"codex_path":null,"theme":"blue",
                "zen_proxy_enabled":true,"zen_proxy_port":19191,
                "zen_proxy_base_url":"https://legacy.example.com/v1",
                "zen_proxy_nudge_enabled":false,"zen_proxy_identity_enabled":false}"#,
        )
        .unwrap();

        let s = load(dir.path());
        assert!(s.compat_proxy_enabled, "旧键 enabled 应被读入");
        assert_eq!(s.compat_proxy_port, 19191, "旧键 port 应被读入");
        assert_eq!(
            s.compat_proxy_base_url, "https://legacy.example.com/v1",
            "旧键 base_url 应被读入"
        );
        assert!(!s.compat_proxy_nudge_enabled, "旧键 nudge 应被读入");
        assert!(!s.compat_proxy_identity_enabled, "旧键 identity 应被读入");

        // 保存后写的是新键：文件里不再出现旧键
        save(dir.path(), &s).unwrap();
        let raw = fs::read_to_string(&p).unwrap();
        assert!(raw.contains("compat_proxy_enabled"), "{raw}");
        assert!(!raw.contains("zen_proxy_"), "不应再写出旧键：{raw}");
    }

    /// 改名兼容：新旧键同时存在时以**新键**为准（旧键被丢弃，不覆盖新值）。
    #[test]
    fn new_keys_win_over_legacy_keys() {
        let dir = TempDir::new().unwrap();
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"compat_proxy_enabled":false,"compat_proxy_port":20001,
                "compat_proxy_base_url":"https://new.example.com/v1",
                "compat_proxy_nudge_enabled":true,"compat_proxy_identity_enabled":true,
                "zen_proxy_enabled":true,"zen_proxy_port":19999,
                "zen_proxy_base_url":"https://old.example.com/v1",
                "zen_proxy_nudge_enabled":false,"zen_proxy_identity_enabled":false}"#,
        )
        .unwrap();

        let s = load(dir.path());
        assert!(!s.compat_proxy_enabled);
        assert_eq!(s.compat_proxy_port, 20001);
        assert_eq!(s.compat_proxy_base_url, "https://new.example.com/v1");
        assert!(s.compat_proxy_nudge_enabled);
        assert!(s.compat_proxy_identity_enabled);
    }

    /// 改名兼容：只写新键时正常读入（旧键缺失不影响）。
    #[test]
    fn new_keys_only_are_read() {
        let dir = TempDir::new().unwrap();
        let p = settings_path(dir.path());
        fs::write(
            &p,
            r#"{"compat_proxy_enabled":true,"compat_proxy_port":19292,
                "compat_proxy_base_url":"https://only-new.example.com/v1"}"#,
        )
        .unwrap();

        let s = load(dir.path());
        assert!(s.compat_proxy_enabled);
        assert_eq!(s.compat_proxy_port, 19292);
        assert_eq!(s.compat_proxy_base_url, "https://only-new.example.com/v1");
        // 未给出的两个行为开关回退默认（开启）
        assert!(s.compat_proxy_nudge_enabled);
        assert!(s.compat_proxy_identity_enabled);
    }

    /// 迁移函数本身：旧键改名、新键优先、非对象/坏 JSON 原样返回、无旧键时也原样返回。
    #[test]
    fn migrate_legacy_keys_rewrites_only_when_needed() {
        // 只有旧键 → 全部改名为新键，旧键一个不留
        let migrated = migrate_legacy_zen_proxy_keys(
            r#"{"theme":"blue","zen_proxy_port":19191,"zen_proxy_enabled":true}"#,
        );
        assert!(
            migrated.contains("\"compat_proxy_port\":19191"),
            "{migrated}"
        );
        assert!(
            migrated.contains("\"compat_proxy_enabled\":true"),
            "{migrated}"
        );
        assert!(!migrated.contains("zen_proxy"), "{migrated}");
        assert!(
            migrated.contains("\"theme\":\"blue\""),
            "其它键应保留：{migrated}"
        );

        // 新旧并存 → 保留新值
        let migrated =
            migrate_legacy_zen_proxy_keys(r#"{"compat_proxy_port":20001,"zen_proxy_port":19999}"#);
        assert!(
            migrated.contains("\"compat_proxy_port\":20001"),
            "{migrated}"
        );
        assert!(!migrated.contains("zen_proxy"), "{migrated}");

        // 没有旧键：原样返回（不做无意义的重新序列化）
        let raw = r#"{ "theme" : "blue" }"#;
        assert_eq!(migrate_legacy_zen_proxy_keys(raw), raw);

        // 坏 JSON / 非对象：原样返回，交给既有「解析失败回退默认」路径
        assert_eq!(migrate_legacy_zen_proxy_keys("not json"), "not json");
        assert_eq!(migrate_legacy_zen_proxy_keys("[1,2]"), "[1,2]");
    }
}
