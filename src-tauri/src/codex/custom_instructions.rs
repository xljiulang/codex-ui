//! 自定义指令读写：管理 `CODEX_HOME/AGENTS.md`（与 config.toml、model_catalog_json 目标文件同目录）。
//!
//! 文件不存在时读取会自动创建空文件；保存自动创建目录与文件，允许保存空内容
//! （Codex 会跳过空指令文件）。

use serde::Serialize;
use std::fs;
use std::path::Path;

use crate::codex::model_config::{atomic_write, codex_home};

/// `custom_instructions_read` 的返回结构。
#[derive(Debug, Clone, Serialize)]
pub struct CustomInstructionsState {
    pub agents_path: String,
    pub exists: bool,
    pub content: String,
}

fn agents_md_path_in(home: &Path) -> std::path::PathBuf {
    home.join("AGENTS.md")
}

/// 读取 AGENTS.md 状态（真实 CODEX_HOME）。
pub fn read_state() -> Result<CustomInstructionsState, String> {
    read_state_in(&codex_home()?)
}

fn read_state_in(home: &Path) -> Result<CustomInstructionsState, String> {
    let path = agents_md_path_in(home);
    if !path.is_file() {
        // 不存在则创建：保证设置页可直接编辑
        atomic_write(&path, "")?;
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取 AGENTS.md 失败: {e}"))?;
    Ok(CustomInstructionsState {
        agents_path: path.to_string_lossy().into_owned(),
        exists: true,
        content,
    })
}

/// 保存 AGENTS.md（真实 CODEX_HOME）：原文写入，允许空内容。
pub fn save(content: &str) -> Result<(), String> {
    save_in(&codex_home()?, content)
}

fn save_in(home: &Path, content: &str) -> Result<(), String> {
    atomic_write(&agents_md_path_in(home), content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_file_is_created_empty_and_exists_true() {
        let dir = TempDir::new().unwrap();
        let state = read_state_in(dir.path()).unwrap();
        assert!(state.exists);
        assert_eq!(state.content, "");
        assert_eq!(state.agents_path, agents_md_path_in(dir.path()).to_string_lossy());
        assert_eq!(
            fs::read_to_string(agents_md_path_in(dir.path())).unwrap(),
            ""
        );
    }

    #[test]
    fn save_creates_file_with_content_and_no_tmp_residue() {
        let dir = TempDir::new().unwrap();
        let content = "# AGENTS.md\n\nWindows + PowerShell 环境。\n";
        save_in(dir.path(), content).unwrap();

        let path = agents_md_path_in(dir.path());
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
        assert!(!path.with_extension("md.tmp").exists());

        let state = read_state_in(dir.path()).unwrap();
        assert!(state.exists);
        assert_eq!(state.content, content);
    }

    #[test]
    fn save_creates_parent_dir_when_missing() {
        let base = TempDir::new().unwrap();
        let home = base.path().join("nested").join(".codex");
        save_in(&home, "hello").unwrap();
        assert_eq!(fs::read_to_string(agents_md_path_in(&home)).unwrap(), "hello");
    }

    #[test]
    fn save_allows_empty_content() {
        let dir = TempDir::new().unwrap();
        save_in(dir.path(), "").unwrap();
        let state = read_state_in(dir.path()).unwrap();
        assert!(state.exists);
        assert_eq!(state.content, "");
    }
}
