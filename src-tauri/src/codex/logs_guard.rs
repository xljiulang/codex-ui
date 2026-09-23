//! codex 日志库膨胀防护：为 `CODEX_HOME/logs_2.sqlite` 的 `logs` 表应用
//! 一个幂等的 `BEFORE INSERT` 触发器，使后续日志 INSERT 全部被 `RAISE(IGNORE)`
//! 丢弃，从根本上阻止该库（及其 WAL）因 codex 持续写入 TRACE 日志而无限增长。
//!
//! 全部为 best-effort：文件缺失/被锁/表不存在一律以结果或错误串返回，由调用方决定
//! 如何记录，绝不 panic、不创建空库。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::codex::model_config;

/// 阻断 `logs` 表写入的幂等触发器（与 codex 社区通用修复一致）。
pub const BLOCK_INSERT_TRIGGER_SQL: &str =
    "CREATE TRIGGER IF NOT EXISTS block_logs_before_insert BEFORE INSERT ON logs \
     BEGIN SELECT RAISE(IGNORE); END;";

/// 等待 `logs_2.sqlite` 出现的最大时长（应用端轮询主循环的步数与步长）。
pub(crate) const WAIT_STEPS: u32 = 12;
pub(crate) const WAIT_STEP: Duration = Duration::from_millis(250);

/// 在指定 SQLite 库上应用阻断触发器。
///
/// - 文件存在 → 打开（只读写、不带 CREATE）、设 3s 忙等待、执行触发器，成功返回 `Ok(true)`；
/// - 文件不存在 → `Ok(false)`，且**不创建**任何新文件；
/// - 打开/执行失败 → `Err`（含被锁、`logs` 表缺失等）。
pub fn apply_logs_guard(db: &Path) -> Result<bool, String> {
    if !db.is_file() {
        return Ok(false);
    }
    let conn =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| format!("打开日志库失败（{}）：{e}", db.display()))?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|e| format!("设置日志库忙等待失败：{e}"))?;
    conn.execute_batch(BLOCK_INSERT_TRIGGER_SQL)
        .map_err(|e| format!("应用日志库阻断触发器失败（{}）：{e}", db.display()))?;
    Ok(true)
}

/// 解析真实 `CODEX_HOME` 目录下的目标日志库并应用阻断触发器。
///
/// - `Ok(Some(path))`：触发器已应用；
/// - `Ok(None)`：库文件尚不存在（跳过，未创建）；
/// - `Err`：无法定位 `CODEX_HOME` 或执行失败。
pub fn apply_at_codex_home(db_name: &str) -> Result<Option<PathBuf>, String> {
    let home = model_config::codex_home()?;
    let db = home.join(db_name);
    let applied = apply_logs_guard(&db)?;
    Ok(if applied { Some(db) } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 设置/移除指定环境变量后执行闭包，结束后恢复原值（与现有测试模块一致）。
    fn with_envs(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        let old: Vec<(&str, Option<std::ffi::OsString>)> = pairs
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (k, v) in pairs {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        f();
        for (k, v) in old {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    /// 建一个含 `logs` 表的临时库并返回路径。
    fn db_with_logs_table(dir: &Path) -> PathBuf {
        let db = dir.join("logs_2.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                level TEXT NOT NULL,
                target TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.close().unwrap();
        db
    }

    fn trigger_count(db: &Path) -> i64 {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='trigger' AND name='block_logs_before_insert'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn applies_idempotent_trigger() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db_with_logs_table(tmp.path());
        assert_eq!(apply_logs_guard(&db).unwrap(), true);
        assert_eq!(trigger_count(&db), 1);
        // 再次执行应幂等成功，不重复建触发器
        assert_eq!(apply_logs_guard(&db).unwrap(), true);
        assert_eq!(trigger_count(&db), 1);
    }

    #[test]
    fn trigger_ignores_inserts() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db_with_logs_table(tmp.path());
        apply_logs_guard(&db).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        // on_conflict 不是命中 IGNORE，这里用普通 INSERT 验证 IGNORE 会丢弃
        conn.execute(
            "INSERT INTO logs (ts, level, target) VALUES (1, 'info', 'test')",
            [],
        )
        .unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "RAISE(IGNORE) 应丢弃对 logs 的插入");
    }

    #[test]
    fn missing_file_returns_false_without_creating() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("nope.sqlite");
        assert_eq!(apply_logs_guard(&db).unwrap(), false);
        assert!(!db.exists(), "不应为缺失的库创建空文件");
    }

    #[test]
    fn missing_logs_table_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("other.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE other (x INTEGER);")
            .unwrap();
        conn.close().unwrap();
        assert!(apply_logs_guard(&db).is_err());
    }

    #[test]
    fn apply_at_codex_home_resolves_and_skips_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let home_s = tmp.path().to_string_lossy().into_owned();
        with_envs(
            &[("CODEX_HOME", Some(&home_s)), ("USERPROFILE", None)],
            || {
                // 目录下无库：跳过且不创建
                assert_eq!(apply_at_codex_home("logs_2.sqlite").unwrap(), None);
                assert!(!tmp.path().join("logs_2.sqlite").exists());
                // 建库后：应用并返回路径
                let db = db_with_logs_table(tmp.path());
                let res = apply_at_codex_home("logs_2.sqlite").unwrap();
                assert_eq!(res.as_deref(), Some(db.as_path()));
                assert_eq!(trigger_count(&db), 1);
            },
        );
    }
}
