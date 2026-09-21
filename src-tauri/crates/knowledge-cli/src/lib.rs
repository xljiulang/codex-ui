//! codex-ui 知识库引擎与 CLI。
//!
//! 本 crate 承载原本跑在 codex-ui 进程内的知识库实现：文档抽取 → 切块 →
//! 本地 ONNX 向量化 → SQLite（向量 + FTS5 关键词）混合检索，并把它暴露成
//! 一次性命令行进程（`codexui-kb`），由 codex-ui 以子进程方式调用。
//!
//! 这样做的唯一目的是内存隔离：向量化会话（ONNX Runtime）与检索时的全量向量
//! 都只存在于这个短命进程里，进程退出即释放，UI 进程内存不随语料规模增长。
//!
//! 数据布局与 codex-ui 既有实现完全一致（因此既有知识库无需重建）：
//! ```text
//! <app data dir>/knowledge/
//! ├─ model/bge-small-zh-v1.5/{model.onnx, tokenizer.json, config.json, ...}
//! └─ kbs/<目录名>-<hash8>.sqlite (+ -wal / -shm)
//! ```

pub mod chunk;
pub mod cli;
pub mod embed;
pub mod extract;
pub mod index;
pub mod paths;
pub mod search;
pub mod store;

pub use cli::{run, run_with};
