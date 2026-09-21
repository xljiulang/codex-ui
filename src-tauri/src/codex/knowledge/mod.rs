//! 售后知识库（单机、纯离线）：索引与检索全部由外部子进程 `codexui-kb.exe` 完成
//! （见 `crates/knowledge-cli/`），codex-ui 只负责进程编排、事件与设置页数据。
//!
//! 之所以拆成独立进程：向量化会话（ONNX Runtime）与检索时的全量向量原本跑在 UI 进程里，
//! 会随语料规模吃内存；放进短命子进程后，UI 进程内存不随知识库规模增长。
//!
//! 分工：
//! ```text
//! {app}\bin\                       # codexui-kb 自包含目录（开发版为 src-tauri\target\debug\）
//! ├─ codexui-kb.exe
//! ├─ onnxruntime.dll
//! └─ model/bge-small-zh-v1.5/{model.onnx, tokenizer.json, config.json, ...}
//!
//! <app data dir>/knowledge/        # 只有索引数据（布局与旧实现一致，既有知识库无需重建）
//! └─ kbs/<目录名>-<hash8>.sqlite (+ -wal / -shm)
//! ```
//! 知识库与会话工作目录一对一：同一目录下的所有会话共享同一个库。

pub mod cli;
pub mod commands;
pub mod paths;

pub use commands::KnowledgeState;
