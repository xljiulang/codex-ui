//! 知识库路径与库名约定。
//!
//! - **索引数据**：`%APPDATA%\com.codexui.app\kbs\<库名>-<hash8>.sqlite`
//!   （与 codex-ui 的 `app_data_dir()` 同一目录，硬编码在此，应用不再传路径）；
//! - **模型与运行库**：与 `codexui-kb.exe` 同目录（自包含）。
//!
//! ```text
//! %APPDATA%\com.codexui.app\      # 应用数据目录本身（与 cache/ logs/ wechat/ 并列）
//! └─ kbs/<库名>-<hash8>.sqlite (+ -wal / -shm)
//!
//! <exe 目录>/                      # 发布版 = {app}\bin\，开发版 = src-tauri\target\debug\
//! ├─ codexui-kb.exe
//! ├─ onnxruntime.dll
//! └─ model/bge-small-zh-v1.5/{model.onnx, tokenizer.json, config.json, ...}
//! ```

use std::path::{Path, PathBuf};

/// 默认向量模型：BAAI/bge-small-zh-v1.5（中文，512 维），随安装包铺在 CLI 同目录 `model/`
pub const MODEL_ID: &str = "bge-small-zh-v1.5";
/// 向量维度：与 [`MODEL_ID`] 对应，入库与检索时校验，避免换模型后混用
pub const EMBED_DIM: usize = 512;
/// 应用标识符（Tauri `identifier`）：数据目录名由它拼出，改 identifier 必须同步这里
pub const APP_IDENTIFIER: &str = "com.codexui.app";
/// 库名最大字符数
pub const KB_NAME_MAX_CHARS: usize = 64;
/// 库文件名主体的最大字符数（[`safe_name`] 的截断长度）
const SAFE_NAME_MAX_CHARS: usize = 32;
/// 名称中禁止出现的字符（Windows 文件名 + 路径语义）
const INVALID_NAME_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

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

/// 知识库数据根目录：`%APPDATA%\com.codexui.app`（应用数据目录本身，库文件在其下的 `kbs\`）
///
/// 与 codex-ui 的 `app_data_dir()` 同一目录（Tauri v2 在 Windows 用
/// `%APPDATA%/<identifier>`）。`APPDATA` 缺失时报错而不回退，避免数据落到两处。
pub fn data_dir() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| {
            "无法定位知识库数据目录：环境变量 APPDATA 未设置（期望 %APPDATA%\\com.codexui.app）"
                .to_string()
        })?;
    Ok(appdata.join(APP_IDENTIFIER))
}

/// 知识库文件目录（`<data_dir>/kbs`）
pub fn kbs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("kbs")
}

/// 规范化目录路径：统一反斜杠、去尾分隔符、小写（Windows 大小写不敏感）。
/// 仅用于来源比较与归属判定，展示仍用原始路径。
pub fn normalize_dir(dir: &str) -> String {
    let mut s = dir.trim().replace('/', "\\");
    // 保留 `D:\` 这类驱动器根的末尾反斜杠：长度 > 3 才裁剪
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s.to_lowercase()
}

/// 规范化库名：去首尾空白、内部连续空白折叠为单个空格、小写（大小写不敏感）。
/// 返回 `(规范化名, 展示名)`；非法名给出可操作中文错误。
pub fn normalize_kb_name(name: &str) -> Result<(String, String), String> {
    let display = name
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if display.is_empty() {
        return Err("库名不能为空".to_string());
    }
    if display
        .chars()
        .any(|c| INVALID_NAME_CHARS.contains(&c) || c.is_control())
    {
        return Err(format!(
            "库名不能包含字符 < > : \" / \\ | ? * 或控制字符：{display}（库名不是路径）"
        ));
    }
    if display == "." || display == ".." {
        return Err(format!("库名不能是「{display}」"));
    }
    if display.chars().count() > KB_NAME_MAX_CHARS {
        return Err(format!(
            "库名过长（最多 {KB_NAME_MAX_CHARS} 个字符）：{display}"
        ));
    }
    if !display.chars().any(|c| c.is_alphanumeric()) {
        return Err(format!("库名需要包含字母、数字或中日韩文字：{display}"));
    }
    Ok((display.to_lowercase(), display))
}

/// 库名 → 文件名主体：非法/空白字符替换为 `_`，最多 32 个字符；结果为空时回退 `kb`。
pub fn safe_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars().take(SAFE_NAME_MAX_CHARS) {
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

/// 8 位十六进制短哈希（大小写、首尾空白差异归一后一致）
pub fn hash8(normalized_name: &str) -> String {
    format!("{:08x}", (fnv1a64(normalized_name) & 0xffff_ffff) as u32)
}

/// 库文件名主干（不含 `.sqlite` 与冲突后缀）；入参应为 [`normalize_kb_name`] 的规范化结果
pub fn kb_stem(normalized_name: &str) -> String {
    format!("{}-{}", safe_name(normalized_name), hash8(normalized_name))
}

/// 首选库文件路径（冲突时 [`crate::store::KbStore::create`] 会顺延槽位）
pub fn kb_path(data_dir: &Path, normalized_name: &str) -> PathBuf {
    kbs_dir(data_dir).join(format!("{}.sqlite", kb_stem(normalized_name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kb_name_normalizes_case_and_whitespace() {
        let (norm, display) = normalize_kb_name("  售后   手册  ").unwrap();
        assert_eq!(norm, "售后 手册");
        assert_eq!(display, "售后 手册");
        assert_eq!(normalize_kb_name("ShouHou").unwrap().0, "shouhou");
        assert_eq!(
            normalize_kb_name("ShouHou").unwrap().0,
            normalize_kb_name("shouhou").unwrap().0
        );
    }

    #[test]
    fn kb_name_rejects_paths_and_invalid_chars() {
        for bad in [
            "D:\\kb",
            "a/b",
            "a:b",
            "a*b",
            "a?b",
            "a|b",
            "a<b",
            "a\"b",
        ] {
            let err = normalize_kb_name(bad).unwrap_err();
            assert!(err.contains("库名"), "{bad} 应被拒：{err}");
        }
        assert!(normalize_kb_name("..").unwrap_err().contains("不能是"));
        assert!(normalize_kb_name("   ").unwrap_err().contains("不能为空"));
        assert!(normalize_kb_name("---").unwrap_err().contains("需要包含"));
        let long = "库".repeat(KB_NAME_MAX_CHARS + 1);
        assert!(normalize_kb_name(&long).unwrap_err().contains("过长"));
    }

    #[test]
    fn kb_file_stem_is_stable_and_readable() {
        let (norm, _) = normalize_kb_name("售后手册").unwrap();
        let stem = kb_stem(&norm);
        assert!(stem.starts_with("售后手册-"));
        assert_eq!(stem, kb_stem(&normalize_kb_name(" 售后手册 ").unwrap().0));
    }

    #[test]
    fn safe_name_falls_back_for_punctuation_only_names() {
        assert_eq!(safe_name("..."), "kb");
        assert_eq!(safe_name("a"), "a");
        assert_eq!(safe_name("a b"), "a_b");
    }

    #[test]
    fn normalize_dir_unifies_separators_case_and_trailing() {
        assert_eq!(normalize_dir("D:/Work/售后/"), "d:\\work\\售后");
        assert_eq!(normalize_dir("d:\\work\\售后"), "d:\\work\\售后");
        assert_eq!(normalize_dir("  D:\\Work  "), "d:\\work");
    }

    #[test]
    fn data_dir_follows_appdata_and_identifier() {
        let previous = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", "C:\\Users\\tester\\AppData\\Roaming");
        let dir = data_dir().unwrap();
        match previous {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        // 数据根 = 应用数据目录本身（不再有 knowledge 这一层），库文件在其下的 kbs/
        assert_eq!(
            dir,
            std::path::Path::new("C:\\Users\\tester\\AppData\\Roaming").join(APP_IDENTIFIER)
        );
        assert!(dir.ends_with(APP_IDENTIFIER));
        assert_eq!(kbs_dir(&dir).file_name().unwrap(), "kbs");
    }
}
