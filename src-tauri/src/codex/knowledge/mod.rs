//! 售后知识库（单机、纯离线）：索引与检索全部由外部子进程 `codexui-kb.exe` 完成
//! （见 `crates/knowledge-cli/`），codex-ui 只负责进程编排、事件与设置页数据。
//!
//! 之所以拆成独立进程：向量化会话（ONNX Runtime）与检索时的全量向量原本跑在 UI 进程里，
//! 会随语料规模吃内存；放进短命子进程后，UI 进程内存不随知识库规模增长。
//!
//! 分工（**库由库名标识，与本模块无关的路径参数一律不传**）：
//! ```text
//! {app}\bin\                       # codexui-kb 自包含目录（开发版为 src-tauri\target\debug\）
//! ├─ codexui-kb.exe
//! ├─ onnxruntime.dll
//! └─ model/bge-small-zh-v1.5/{model.onnx, tokenizer.json, config.json, ...}
//!
//! %APPDATA%\com.codexui.app\                       # 应用数据目录（CLI 自行解析）
//! └─ kbs/<库名>-<hash8>.sqlite (+ -wal / -shm)
//! ```
//! 库名缺省由前端按会话工作目录名派生（`D:\售后\手册` → `手册`），因此不同目录的同名库
//! 会共享；跨目录共享同一个库时，双方使用同一个库名即可。

pub mod cli;
pub mod commands;
pub mod hint;

pub use commands::KnowledgeState;
