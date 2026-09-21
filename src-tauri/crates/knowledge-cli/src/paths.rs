//! 知识库路径约定：
//! - **模型与运行库**：与 `codexui-kb.exe` 同目录（自包含），即 `<exe 目录>/model/<MODEL_ID>/`
//!   与 `<exe 目录>/onnxruntime.dll`；
//! - **索引数据**：`--data-dir`（`<app data dir>/knowledge/`）下只放 `kbs/`。
//!
//! ```text
//! <exe 目录>/                      # 发布版 = {app}\bin\，开发版 = src-tauri\target\debug\
//! ├─ codexui-kb.exe
//! ├─ onnxruntime.dll
//! └─ model/bge-small-zh-v1.5/{model.onnx, tokenizer.json, config.json, ...}
//!
//! <data-dir>/
//! └─ kbs/<目录名>-<hash8>.sqlite (+ -wal / -shm)
//! ```

use std::path::{Path, PathBuf};

/// 默认向量模型：BAAI/bge-small-zh-v1.5（中文，512 维），随安装包铺在 CLI 同目录 `model/`
pub const MODEL_ID: &str = "bge-small-zh-v1.5";
/// 向量维度：与 [`MODEL_ID`] 对应，入库与检索时校验，避免换模型后混用
pub const EMBED_DIM: usize = 512;

/// `codexui-kb.exe` 所在目录（模型与 onnxruntime.dll 都与它同目录）
pub fn exe_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .ok_or_else(|| "无法定位 codexui-kb.exe 所在目录".to_string())
}

/// 模型根目录：与 CLI 同目录的 `model/`
pub fn model_root() -> Result<PathBuf, String> {
    Ok(exe_dir()?.join("model"))
}

/// 当前模型目录（内含 `model.onnx` / `tokenizer.json` 等）
pub fn model_dir() -> Result<PathBuf, String> {
    Ok(model_root()?.join(MODEL_ID))
}

/// 各工作目录的知识库文件目录
pub fn kbs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("kbs")
}

/// 规范化工作目录：统一反斜杠、去尾分隔符、小写（Windows 路径大小写不敏感）。
/// 仅用于派生库文件名与归属比较，展示与写库仍用原始路径。
pub fn normalize_workspace(cwd: &str) -> String {
    let mut s = cwd.trim().replace('/', "\\");
    // 保留 `D:\` 这类驱动器根与非 UNC 根的末尾反斜杠处理：长度 > 3 才裁剪
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s.to_lowercase()
}

/// 目录名安全化：取最后一段目录名，非法字符替换为 `_`，最多 32 个字符；
/// 结果为空时回退 `kb`，保证库文件名始终可读且合法。
pub fn safe_name(cwd: &str) -> String {
    let normalized = normalize_workspace(cwd);
    let last = normalized
        .trim_end_matches('\\')
        .rsplit('\\')
        .next()
        .unwrap_or("");
    let mut out = String::new();
    for ch in last.chars().take(32) {
        out.push(if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            ch
        } else {
            '_'
        });
    }
    let out = out.trim_matches(['.', '_']).to_string();
    if out.is_empty() {
        "kb".to_string()
    } else {
        out
    }
}

/// FNV-1a 64 位：派生稳定短哈希（非安全用途）。
fn fnv1a64(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 8 位十六进制短哈希（大小写、斜杠、尾分隔符差异归一后一致）
pub fn hash8(cwd: &str) -> String {
    format!("{:08x}", (fnv1a64(&normalize_workspace(cwd)) & 0xffff_ffff) as u32)
}

/// 库文件名主干（不含 `.sqlite` 与冲突后缀）
pub fn kb_stem(cwd: &str) -> String {
    format!("{}-{}", safe_name(cwd), hash8(cwd))
}

/// 首选库文件路径（冲突时 [`crate::store::KbStore::open`] 会顺延槽位）
pub fn kb_path(data_dir: &Path, cwd: &str) -> PathBuf {
    kbs_dir(data_dir).join(format!("{}.sqlite", kb_stem(cwd)))
}
