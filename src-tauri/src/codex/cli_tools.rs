//! 捆绑 CLI 工具检测：判断 codex 生效 PATH 上是否存在 ast-grep / fd / rg。
//!
//! 应用启动 codex 时会前插 `<应用目录>/bin` 到 PATH（`prepend_bin_path`/`apply_codex_env_in`），
//! 这里的 `cli_tools_available` 按与 codex 子进程一致的生效 PATH 解析这些工具名，
//! 前端据此在默认协作模式的 `developer_instructions` 里注入对应使用说明。

use std::env::split_paths;
use std::path::Path;

use crate::codex::app_server::{app_exe_dir, prepend_bin_path};

/// 已知捆绑工具的基名（保持稳定的遍历顺序，前端说明也按此顺序展示）。
const TOOL_NAMES: [&str; 3] = ["ast-grep", "fd", "rg"];

/// Windows 上 PATH 解析的可执行扩展名（`ast-grep.exe`/`sg.bat` 等）。
const WINDOWS_EXTS: [&str; 4] = ["exe", "bat", "cmd", "com"];

/// 在给定 PATH 字符串上解析已知捆绑 CLI，返回命中的基名（按 `TOOL_NAMES` 顺序、去重）。
pub fn available_tools_in(path: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for dir in split_paths(Path::new(path)) {
        for name in TOOL_NAMES {
            if found.iter().any(|f| f == name) {
                continue;
            }
            let base = dir.join(name);
            let hit = WINDOWS_EXTS.iter().any(|ext| base.with_extension(ext).is_file())
                || base.is_file();
            if hit {
                found.push(name.to_string());
            }
        }
    }
    found
}

/// 当前 codex 生效 PATH 上可用的捆绑 CLI 工具基名列表（失败/缺失时回退，不报错）。
#[tauri::command]
pub fn cli_tools_available() -> Vec<String> {
    let existing = std::env::var("PATH").unwrap_or_default();
    let effective = match app_exe_dir() {
        Some(dir) => prepend_bin_path(&existing, &dir)
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|| existing.clone()),
        None => existing,
    };
    available_tools_in(&effective)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn touch(dir: &Path, name: &str, content: &[u8]) {
        fs::write(dir.join(name), content).unwrap();
    }

    fn path_only(dir: &std::path::Path) -> String {
        std::env::join_paths([PathBuf::from(dir)])
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn detects_all_bundled_tools_on_path() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "ast-grep.exe", b"MZ");
        touch(tmp.path(), "fd.exe", b"MZ");
        touch(tmp.path(), "rg.exe", b"MZ");
        assert_eq!(
            available_tools_in(&path_only(tmp.path())),
            vec!["ast-grep".to_string(), "fd".to_string(), "rg".to_string()]
        );
    }

    #[test]
    fn missing_tools_are_omitted() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "rg.exe", b"MZ");
        assert_eq!(
            available_tools_in(&path_only(tmp.path())),
            vec!["rg".to_string()]
        );
    }

    #[test]
    fn path_without_tools_returns_empty() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            available_tools_in(&path_only(tmp.path())),
            Vec::<String>::new()
        );
    }

    #[test]
    fn bat_and_extensionless_are_recognized() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "ast-grep.bat", b"@echo off");
        touch(tmp.path(), "fd", b"#!/bin/sh");
        let found = available_tools_in(&path_only(tmp.path()));
        assert!(found.contains(&"ast-grep".to_string()));
        assert!(found.contains(&"fd".to_string()));
        assert!(!found.contains(&"rg".to_string()));
    }

    #[test]
    fn repeated_path_entries_dedupe() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "rg.exe", b"MZ");
        let path = std::env::join_paths([
            PathBuf::from(tmp.path()),
            PathBuf::from(tmp.path()),
        ])
        .unwrap()
        .to_string_lossy()
        .into_owned();
        assert_eq!(available_tools_in(&path), vec!["rg".to_string()]);
    }
}
