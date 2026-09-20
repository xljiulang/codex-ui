//! 售后知识库（单机、纯离线）：本地文档抽取 → 本地 ONNX 向量化 →
//! SQLite（向量 + FTS5 关键词）混合检索，经 codex-ui 动态工具暴露给 agent。
//!
//! 数据全部位于 `<app data dir>/knowledge/`：
//! ```text
//! knowledge/
//! ├─ model/bge-small-zh-v1.5/{model.onnx, tokenizer.json, config.json, ...}
//! └─ kbs/<目录名>-<hash8>.sqlite (+ -wal / -shm)
//! ```
//! 知识库与会话工作目录一对一：同一目录下的所有会话共享同一个库。

pub mod chunk;
pub mod commands;
pub mod embed;
pub mod extract;
pub mod index;
pub mod paths;
pub mod search;
pub mod store;

pub use commands::KnowledgeState;

use std::path::Path;

use crate::codex::bundled::{self, BundleSpec};

/// 物化随包模型归档（`<exe 目录>/marketplaces/knowledge-model.tar.gz` → `knowledge/model/`）。
/// 复用既有 `materialize`：临时目录完整解压 → 整体改名 → 写 `.materialization-key`。
pub(crate) fn materialize_model(app_dir: &Path) -> Result<bundled::BootOutcome, String> {
    let archive_dir = bundled::archive_dir().ok_or_else(|| "无法定位应用目录".to_string())?;
    let spec = BundleSpec {
        archive_name: paths::MODEL_ARCHIVE,
        dest: paths::model_root(app_dir),
        top: paths::MODEL_ID,
    };
    Ok(bundled::bootstrap_one(&spec, &archive_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn model_materialize_reports_missing_archive_without_touching_disk() {
        let dir = TempDir::new().unwrap();
        let archive_dir = dir.path().join("marketplaces");
        std::fs::create_dir_all(&archive_dir).unwrap();
        let spec = BundleSpec {
            archive_name: paths::MODEL_ARCHIVE,
            dest: paths::model_root(dir.path()),
            top: paths::MODEL_ID,
        };
        // 归档缺失时明确返回 ArchiveMissing（不建空目录、不报错阻断启动）
        assert_eq!(
            bundled::bootstrap_one(&spec, &archive_dir),
            bundled::BootOutcome::ArchiveMissing
        );
        assert!(!paths::model_dir(dir.path()).exists());
    }
}
