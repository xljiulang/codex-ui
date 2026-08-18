//! 历史会话迁移：启动后把 `%USERPROFILE%\.codex\sessions` 复制到应用
//! `CODEX_HOME/sessions`（仅当目标不存在时），源目录保留，失败静默。

use std::fs;
use std::path::Path;

use crate::codex::model_config::codex_home;

/// 递归复制目录（含空目录），文件用 `fs::copy` 保留内容。
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建目录失败: {e}"))?;
    for entry in fs::read_dir(src).map_err(|e| format!("读取目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|e| format!("复制文件失败: {e}"))?;
        }
    }
    Ok(())
}

/// 递归判断目录是否已包含任意文件；目录缺失/为空/只有空子目录时返回 false。
fn dir_contains_files(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if dir_contains_files(&path) {
                return true;
            }
        } else {
            return true;
        }
    }
    false
}

/// 复制迁移：源不是目录，或目标已包含任意会话文件时返回 false（跳过）；成功返回 true。
/// 先复制到同级临时目录再原子改名，避免目标出现半成品；失败尽力清理临时目录。
pub fn migrate_sessions_in(src: &Path, dst: &Path) -> Result<bool, String> {
    if !src.is_dir() {
        return Ok(false);
    }
    if dst.exists() {
        if dir_contains_files(dst) {
            // 目标已有会话文件：跳过，避免覆盖/混合
            return Ok(false);
        }
        // 目标为空目录（含只有空子目录）：清理后走原子迁移路径
        fs::remove_dir_all(dst).map_err(|e| format!("清理空目标目录失败: {e}"))?;
    }
    let parent = dst.parent().ok_or_else(|| "目标路径无父目录".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("创建父目录失败: {e}"))?;
    let tmp = parent.join("sessions.migrating.tmp");
    if tmp.exists() {
        // 清理上次崩溃可能残留的临时目录
        fs::remove_dir_all(&tmp).map_err(|e| format!("清理临时目录失败: {e}"))?;
    }
    if let Err(e) = copy_dir_recursive(src, &tmp) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, dst).map_err(|e| {
        let _ = fs::remove_dir_all(&tmp);
        format!("迁移改名失败: {e}")
    })?;
    Ok(true)
}

/// 启动后的一次性迁移：`%USERPROFILE%\.codex\sessions` → `CODEX_HOME/sessions`。
/// best-effort：`USERPROFILE` 缺失或前置条件不满足时静默跳过。
pub fn migrate_sessions_once() -> Result<bool, String> {
    let Some(user_profile) = std::env::var_os("USERPROFILE") else {
        return Ok(false);
    };
    let src = Path::new(&user_profile).join(".codex").join("sessions");
    let dst = codex_home()?.join("sessions");
    migrate_sessions_in(&src, &dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_tree(root: &Path) {
        fs::create_dir_all(root.join("2026").join("01")).unwrap();
        fs::write(root.join("2026").join("01").join("session-a.jsonl"), "hello").unwrap();
        fs::write(root.join("2026").join("session-b.jsonl"), "world").unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
    }

    #[test]
    fn migrates_nested_tree_when_target_missing() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        make_tree(&src);
        let dst = tmp.path().join("dst").join("sessions");

        assert!(migrate_sessions_in(&src, &dst).unwrap());
        assert_eq!(
            fs::read_to_string(dst.join("2026").join("01").join("session-a.jsonl")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(dst.join("2026").join("session-b.jsonl")).unwrap(),
            "world"
        );
        assert!(dst.join("empty").is_dir());
        // 原子写不残留临时目录
        assert!(!tmp.path().join("dst").join("sessions.migrating.tmp").exists());
    }

    #[test]
    fn skips_when_target_exists() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        make_tree(&src);
        let dst = tmp.path().join("sessions");
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("keep.txt"), "keep").unwrap();

        assert!(!migrate_sessions_in(&src, &dst).unwrap());
        assert_eq!(fs::read_to_string(dst.join("keep.txt")).unwrap(), "keep");
        assert!(!dst.join("2026").exists());
    }

    #[test]
    fn migrates_when_target_is_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        make_tree(&src);
        let dst = tmp.path().join("sessions");
        fs::create_dir_all(&dst).unwrap();

        assert!(migrate_sessions_in(&src, &dst).unwrap());
        assert_eq!(
            fs::read_to_string(dst.join("2026").join("session-b.jsonl")).unwrap(),
            "world"
        );
        assert!(!tmp.path().join("sessions.migrating.tmp").exists());
    }

    #[test]
    fn migrates_when_target_only_has_empty_subdirs() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        make_tree(&src);
        let dst = tmp.path().join("sessions");
        fs::create_dir_all(dst.join("2026")).unwrap();

        assert!(migrate_sessions_in(&src, &dst).unwrap());
        assert_eq!(
            fs::read_to_string(dst.join("2026").join("01").join("session-a.jsonl")).unwrap(),
            "hello"
        );
        assert!(!tmp.path().join("sessions.migrating.tmp").exists());
    }

    #[test]
    fn skips_when_source_missing() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("no-src");
        let dst = tmp.path().join("sessions");

        assert!(!migrate_sessions_in(&src, &dst).unwrap());
        assert!(!dst.exists());
    }
}
