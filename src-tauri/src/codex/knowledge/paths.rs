//! 知识库数据目录约定（只涉及索引数据；模型与运行库不在这里）。
//!
//! ```text
//! <app data dir>/knowledge/
//! └─ kbs/<目录名>-<hash8>.sqlite (+ -wal / -shm)
//! ```
//!
//! 模型与 `onnxruntime.dll` 与 `codexui-kb.exe` 同目录（发布版 `{app}\bin\`、
//! 开发版 `src-tauri\target\debug\`），由子进程自行解析，codex-ui 不做任何物化。
//! 库文件名与槽位顺延规则都在子进程里（`crates/knowledge-cli/src/paths.rs`），
//! 保证与既有知识库完全一致。

use std::path::{Path, PathBuf};

/// 知识库数据根目录（传给子进程的 `--data-dir`）
pub fn knowledge_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("knowledge")
}
