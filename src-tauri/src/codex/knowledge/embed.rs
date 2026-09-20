//! 本地向量化：fastembed + 随包 ONNX 模型（`<app data dir>/knowledge/model/<MODEL_ID>/`），
//! ONNX Runtime 以动态加载方式从 `<应用目录>/bin/onnxruntime.dll` 载入，全程不出网。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use fastembed::{
    InitOptionsUserDefined, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};

use super::paths;

/// 单批送入模型的文本条数
pub const EMBED_BATCH: usize = 64;

/// 模型文件清单（缺一不可）
const MODEL_FILES: &[&str] = &[
    "model.onnx",
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
];

struct EmbedState {
    inner: Mutex<Option<TextEmbedding>>,
    init: Mutex<()>,
}

static EMBED: OnceLock<EmbedState> = OnceLock::new();
/// ONNX Runtime 只需动态加载一次（`ort` 要求在任何会话创建前完成）
static ORT_LOADED: OnceLock<bool> = OnceLock::new();

fn state() -> &'static EmbedState {
    EMBED.get_or_init(|| EmbedState {
        inner: Mutex::new(None),
        init: Mutex::new(()),
    })
}

/// onnxruntime.dll 查找顺序：显式环境变量 → `<应用目录>/bin/` → `<应用目录>/`
pub fn ort_dll_path() -> Result<PathBuf, String> {
    if let Some(custom) = std::env::var_os("CODEXUI_ORT_DYLIB") {
        let path = PathBuf::from(custom);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("CODEXUI_ORT_DYLIB 指向的文件不存在：{}", path.display()));
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .ok_or_else(|| "无法定位应用目录".to_string())?;
    for candidate in [exe_dir.join("bin").join("onnxruntime.dll"), exe_dir.join("onnxruntime.dll")] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("未找到 onnxruntime.dll（应随安装包放在应用目录的 bin/ 下）".into())
}

/// 模型是否就绪：文件齐全 + ONNX Runtime 可定位
pub fn model_ready(app_dir: &Path) -> bool {
    model_dir_ready(app_dir) && ort_dll_path().is_ok()
}

fn model_dir_ready(app_dir: &Path) -> bool {
    let dir = paths::model_dir(app_dir);
    MODEL_FILES.iter().all(|f| dir.join(f).is_file())
}

fn read_model_files(app_dir: &Path) -> Result<(Vec<u8>, TokenizerFiles), String> {
    let dir = paths::model_dir(app_dir);
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let path = dir.join(name);
        std::fs::read(&path).map_err(|e| {
            format!(
                "读取模型文件失败 {}（请确认已随包分发或手动放入该目录）：{e}",
                path.display()
            )
        })
    };
    let onnx = read("model.onnx")?;
    let files = TokenizerFiles {
        tokenizer_file: read("tokenizer.json")?,
        config_file: read("config.json")?,
        special_tokens_map_file: read("special_tokens_map.json")?,
        tokenizer_config_file: read("tokenizer_config.json")?,
    };
    Ok((onnx, files))
}

fn ensure_ort_loaded() -> Result<(), String> {
    if *ORT_LOADED.get_or_init(|| {
        let loaded = (|| -> Result<(), String> {
            let dll = ort_dll_path()?;
            ort::init_from(&dll)
                .map_err(|e| format!("加载 ONNX Runtime 失败 {}：{e}", dll.display()))?
                .commit();
            Ok(())
        })();
        if let Err(e) = &loaded {
            // 失败不缓存：允许用户补齐 DLL 后重试
            eprintln!("[codex-ui] {e}");
        }
        loaded.is_ok()
    }) {
        Ok(())
    } else {
        Err(ort_dll_path()
            .err()
            .unwrap_or_else(|| "ONNX Runtime 初始化失败".to_string()))
    }
}

/// 构建（或复用）embedding 会话；失败不缓存，便于补齐模型/DLL 后重试
fn with_embedder<T>(
    app_dir: &Path,
    f: impl FnOnce(&mut TextEmbedding) -> Result<T, String>,
) -> Result<T, String> {
    let st = state();
    let _guard = st
        .init
        .lock()
        .map_err(|_| "embedding 初始化锁中毒".to_string())?;
    let mut guard = st
        .inner
        .lock()
        .map_err(|_| "embedding 会话锁中毒".to_string())?;
    if guard.is_none() {
        ensure_ort_loaded()?;
        let (onnx, files) = read_model_files(app_dir)?;
        let model = UserDefinedEmbeddingModel::new(onnx, files);
        let embedding = TextEmbedding::try_new_from_user_defined(
            model,
            InitOptionsUserDefined::new(),
        )
        .map_err(|e| format!("初始化向量模型失败：{e}"))?;
        *guard = Some(embedding);
    }
    f(guard.as_mut().expect("embedding 会话已初始化"))
}

/// 批量向量化：返回与入参等长的向量列表（维度必须是 [`paths::EMBED_DIM`]）
pub fn embed_texts(app_dir: &Path, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let vectors = with_embedder(app_dir, |embedding| {
        embedding
            .embed(texts, Some(EMBED_BATCH))
            .map_err(|e| format!("向量化失败：{e}"))
    })?;
    if vectors.len() != texts.len() {
        return Err(format!(
            "向量化结果条数不匹配（期望 {}，实际 {}）",
            texts.len(),
            vectors.len()
        ));
    }
    for v in &vectors {
        if v.len() != paths::EMBED_DIM {
            return Err(format!(
                "向量维度不匹配（模型输出 {} 维，期望 {} 维），请检查 knowledge/model/{} 是否为 {}",
                v.len(),
                paths::EMBED_DIM,
                paths::MODEL_ID,
                paths::MODEL_ID
            ));
        }
    }
    Ok(vectors)
}

/// 单条向量化（检索查询用）
pub fn embed_query(app_dir: &Path, query: &str) -> Result<Vec<f32>, String> {
    let mut out = embed_texts(app_dir, &[query.to_string()])?;
    out.pop().ok_or_else(|| "向量化返回空结果".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_model_reports_actionable_error() {
        let dir = TempDir::new().unwrap();
        assert!(!model_ready(dir.path()));
        let err = embed_texts(dir.path(), &["你好".to_string()]).unwrap_err();
        assert!(
            err.contains("模型文件") || err.contains("onnxruntime.dll"),
            "错误信息应可指导修复：{err}"
        );
    }

    #[test]
    fn empty_input_short_circuits() {
        let dir = TempDir::new().unwrap();
        assert!(embed_texts(dir.path(), &[]).unwrap().is_empty());
    }

    #[test]
    fn ort_dll_hint_points_to_app_bin() {
        // 测试环境没有 DLL：错误信息必须写明期望位置
        if std::env::var_os("CODEXUI_ORT_DYLIB").is_none() {
            let err = ort_dll_path().unwrap_err();
            assert!(err.contains("bin"), "{err}");
        }
    }
}
