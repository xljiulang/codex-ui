//! codex-ui 自有环境变量的统一约定与解析。
//!
//! 命名空间：所有本应用自有的环境变量都用 `CODEXUI_` 前缀，后接大写下划线的功能段，
//! 功能段按「组件_用途」组织（例：`CODEXUI_COMPAT_TRACE` 控制兼容代理内容诊断日志，
//! 将来的 `CODEXUI_COMPAT_TRACE_MAX_MB`、`CODEXUI_WECHAT_DEBUG` 沿用同一前缀与解析规则）。
//!
//! 取值约定（大小写不敏感、自动 trim）：
//! - 真值：`1` / `true` / `on` / `yes`
//! - 假值：`0` / `false` / `off` / `no`（空串等同未设置）
//! - 其它取值：视为「无法识别」，按调用方给出的默认值处理，并由 `invalid_value` 供调用方记 warning
//!
//! 环境变量**只在进程启动时读取一次**（GUI 进程运行期间改环境变量不会生效），
//! 因此这些函数都按「读一次、算一次」的语义实现，不做缓存。

/// 本应用自有环境变量的统一前缀。
pub(crate) const ENV_PREFIX: &str = "CODEXUI_";

/// 环境变量取值的三态解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnvFlag {
    /// 真值：`1` / `true` / `on` / `yes`。
    On,
    /// 假值：`0` / `false` / `off` / `no`，或未设置/空串。
    Off,
    /// 无法识别的非空取值（原样带回，供调用方记 warning）。
    Invalid(String),
}

/// 解析一个环境变量取值（纯函数，便于单测）。
pub(crate) fn parse(value: Option<&str>) -> EnvFlag {
    let Some(raw) = value else {
        return EnvFlag::Off;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return EnvFlag::Off;
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => EnvFlag::On,
        "0" | "false" | "off" | "no" => EnvFlag::Off,
        _ => EnvFlag::Invalid(trimmed.to_string()),
    }
}

/// 读取 `CODEXUI_<SUFFIX>` 并解析：真值 → true，其余（含未设置、假值、无法识别）→ false。
pub(crate) fn flag(suffix: &str) -> bool {
    matches!(
        parse(std::env::var(full_name(suffix).as_str()).ok().as_deref()),
        EnvFlag::On
    )
}

/// `CODEXUI_<SUFFIX>` 的取值无法识别时返回原值（未设置、空、真/假值都返回 None），供记 warning。
pub(crate) fn invalid_value(suffix: &str) -> Option<String> {
    match parse(std::env::var(full_name(suffix).as_str()).ok().as_deref()) {
        EnvFlag::Invalid(raw) => Some(raw),
        _ => None,
    }
}

/// 拼接完整变量名：`CODEXUI_<SUFFIX>`（suffix 传大写功能段，如 `COMPAT_TRACE`）。
pub(crate) fn full_name(suffix: &str) -> String {
    format!("{ENV_PREFIX}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 环境变量读写不是线程安全的，测试串行化；结束后恢复原值。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env(value: Option<&str>, f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        let key = full_name("TEST_FLAG");
        let old = std::env::var_os(&key);
        match value {
            Some(v) => std::env::set_var(&key, v),
            None => std::env::remove_var(&key),
        }
        f();
        match old {
            Some(v) => std::env::set_var(&key, v),
            None => std::env::remove_var(&key),
        }
    }

    #[test]
    fn parse_recognizes_truthy_falsy_and_invalid() {
        for raw in ["1", "true", "TRUE", " on ", "Yes", "yEs"] {
            assert_eq!(parse(Some(raw)), EnvFlag::On, "{raw}");
        }
        for raw in ["0", "false", "FALSE", " off ", "No", ""] {
            assert_eq!(parse(Some(raw)), EnvFlag::Off, "{raw}");
        }
        // 未设置
        assert_eq!(parse(None), EnvFlag::Off);
        // 无法识别：原样（trim 后）带回
        assert_eq!(
            parse(Some("  maybe  ")),
            EnvFlag::Invalid("maybe".to_string())
        );
    }

    #[test]
    fn flag_reads_truthy_values_and_defaults_off() {
        with_env(None, || assert!(!flag("TEST_FLAG")));
        with_env(Some("1"), || assert!(flag("TEST_FLAG")));
        with_env(Some("ON"), || assert!(flag("TEST_FLAG")));
        with_env(Some("0"), || assert!(!flag("TEST_FLAG")));
        with_env(Some("off"), || assert!(!flag("TEST_FLAG")));
        // 无法识别的取值按关闭处理
        with_env(Some("abc"), || assert!(!flag("TEST_FLAG")));
    }

    #[test]
    fn invalid_value_only_reports_unrecognized_non_empty() {
        with_env(None, || assert_eq!(invalid_value("TEST_FLAG"), None));
        with_env(Some("1"), || assert_eq!(invalid_value("TEST_FLAG"), None));
        with_env(Some("off"), || assert_eq!(invalid_value("TEST_FLAG"), None));
        with_env(Some("  abc "), || {
            assert_eq!(invalid_value("TEST_FLAG"), Some("abc".to_string()));
        });
    }

    #[test]
    fn full_name_uses_namespace_prefix() {
        assert_eq!(full_name("COMPAT_TRACE"), "CODEXUI_COMPAT_TRACE");
        assert!(full_name("COMPAT_TRACE").starts_with(ENV_PREFIX));
    }
}
