//! 文档抽取：PDF / Word(docx) / Markdown / 纯文本 → 带标题标记的纯文本。
//!
//! 抽取结果统一成「Markdown 风格标题 + 正文段落」的文本，交给 [`super::chunk`] 切块，
//! 因此标题路径（引用出处）对四种格式都可用。

use std::io::Read;
use std::path::{Path, PathBuf};

use quick_xml::events::Event;
use quick_xml::Reader;

/// 支持入库的扩展名（小写，含点）
pub const SUPPORTED_EXTS: &[&str] = &[".md", ".markdown", ".txt", ".pdf", ".docx"];

/// 扫描目录时跳过的噪音目录名
const SKIP_DIRS: &[&str] = &[".git", ".svn", ".hg", "node_modules", "target"];

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default()
}

/// 该文件是否是支持入库的格式
pub fn is_supported(path: &Path) -> bool {
    SUPPORTED_EXTS.contains(&ext_of(path).as_str())
}

/// 递归收集目录下的支持文件（路径排序，跳过噪音目录）；传入单个文件时按是否支持返回。
pub fn collect_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        if is_supported(root) {
            out.push(root.to_path_buf());
        }
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                stack.push(path);
            } else if meta.is_file() && is_supported(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// 抽取单个文件的文本（按扩展名分派）
pub fn extract_file(path: &Path) -> Result<String, String> {
    match ext_of(path).as_str() {
        ".md" | ".markdown" | ".txt" => extract_plain(path),
        ".pdf" => extract_pdf(path),
        ".docx" => extract_docx(path),
        other => Err(format!("不支持的文件类型：{other}")),
    }
}

/// 纯文本：UTF-8 优先，失败回退 GB18030（国内售后文档常见编码）
fn extract_plain(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败：{e}"))?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(e) => {
            let bytes = e.into_bytes();
            let (text, _, _) = encoding_rs::GB18030.decode(&bytes);
            Ok(text.into_owned())
        }
    }
}

fn extract_pdf(path: &Path) -> Result<String, String> {
    pdf_extract::extract_text(path).map_err(|e| format!("PDF 解析失败：{e}"))
}

/// DOCX：读 `word/document.xml`，段落按 `w:pStyle` 的 Heading 级别还原为 Markdown 标题
fn extract_docx(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("打开 docx 失败：{e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("docx 不是有效的 zip 包：{e}"))?;
    let mut entry = archive
        .by_name("word/document.xml")
        .map_err(|e| format!("docx 缺少 word/document.xml：{e}"))?;
    let mut xml = String::new();
    entry
        .read_to_string(&mut xml)
        .map_err(|e| format!("读取 docx 正文失败：{e}"))?;
    docx_xml_to_text(&xml)
}

/// 把 `word/document.xml` 转成 Markdown 风格文本（纯函数，便于测试）
pub fn docx_xml_to_text(xml: &str) -> Result<String, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut lines: Vec<String> = Vec::new();
    let mut paragraph = String::new();
    let mut heading: usize = 0;
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "w:p" => {
                    paragraph.clear();
                    heading = 0;
                }
                "w:pStyle" => {
                    heading = heading_from_attrs(&e);
                }
                "w:t" => in_text = true,
                "w:tab" => paragraph.push('\t'),
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.name().as_ref() {
                // docx 里 `w:pStyle` 常写成自闭合标签，必须在 Empty 分支同样处理
                "w:pStyle" => heading = heading_from_attrs(&e),
                "w:br" | "w:cr" => paragraph.push('\n'),
                "w:tab" => paragraph.push('\t'),
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if in_text {
                    // 0.42 起文本事件以 &str 承载「未反转义」内容，统一走 escape::unescape
                    let raw = t.into_inner();
                    let text = match quick_xml::escape::unescape(&raw) {
                        Ok(unescaped) => unescaped.into_owned(),
                        Err(_) => raw.into_owned(),
                    };
                    paragraph.push_str(&text);
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                "w:t" => in_text = false,
                "w:p" => {
                    let text = paragraph.trim().to_string();
                    paragraph.clear();
                    if text.is_empty() {
                        continue;
                    }
                    if heading > 0 {
                        lines.push(format!("{} {}", "#".repeat(heading), text));
                    } else {
                        lines.push(text);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("docx 正文解析失败：{e}")),
            _ => {}
        }
    }
    Ok(lines.join("\n\n"))
}

/// Word 标题样式 → Markdown 级别（`Heading1`/`heading 2`/`标题 3` 等）
fn heading_level_from_style(style: &str) -> usize {
    let lower = style.to_lowercase();
    if !(lower.contains("heading") || lower.contains("标题") || lower.contains("title")) {
        return 0;
    }
    let level: usize = lower
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(1);
    level.clamp(1, 6)
}

/// 从 `w:pStyle` 元素属性里取 `w:val` 并映射为标题级别
fn heading_from_attrs(e: &quick_xml::events::BytesStart<'_>) -> usize {
    let mut level = 0;
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == "w:val" {
            if let Ok(value) = attr.normalized_value(quick_xml::XmlVersion::Explicit1_0) {
                level = heading_level_from_style(&value);
            }
        }
    }
    level
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn plain_text_utf8_and_gb18030() {
        let dir = TempDir::new().unwrap();
        let utf8 = dir.path().join("a.md");
        std::fs::write(&utf8, "# 标题\n\n正文").unwrap();
        assert_eq!(extract_file(&utf8).unwrap(), "# 标题\n\n正文");

        let gbk = dir.path().join("b.txt");
        let (encoded, _, _) = encoding_rs::GB18030.encode("中文内容");
        std::fs::write(&gbk, encoded.as_ref()).unwrap();
        assert_eq!(extract_file(&gbk).unwrap(), "中文内容");
    }

    #[test]
    fn collect_files_filters_extensions_and_noise_dirs() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("docs/手册.md"), "x").unwrap();
        std::fs::write(dir.path().join("docs/图片.png"), "x").unwrap();
        std::fs::write(dir.path().join("node_modules/readme.md"), "x").unwrap();
        let files = collect_files(dir.path());
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("手册.md"));
    }

    #[test]
    fn collect_files_accepts_single_file() {
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(collect_files(&f), vec![f.clone()]);
        let png = dir.path().join("a.png");
        std::fs::write(&png, "x").unwrap();
        assert!(collect_files(&png).is_empty());
        assert!(!is_supported(&png));
    }

    #[test]
    fn docx_xml_becomes_markdown_with_heading_levels() {
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>售后手册</w:t></w:r></w:p>
    <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>3.2 无法开机</w:t></w:r></w:p>
    <w:p><w:r><w:t>请检查电源适配器。</w:t></w:r><w:r><w:br/><w:t>然后重启设备。</w:t></w:r></w:p>
    <w:p/>
  </w:body>
</w:document>"#;
        let text = docx_xml_to_text(xml).unwrap();
        assert!(text.contains("# 售后手册"));
        assert!(text.contains("## 3.2 无法开机"));
        assert!(text.contains("请检查电源适配器。"));
        assert!(text.contains("然后重启设备。"));
    }

    #[test]
    fn docx_heading_style_accepts_chinese_and_plain_names() {
        assert_eq!(heading_level_from_style("Heading3"), 3);
        assert_eq!(heading_level_from_style("heading 2"), 2);
        assert_eq!(heading_level_from_style("标题 4"), 4);
        assert_eq!(heading_level_from_style("Normal"), 0);
        assert_eq!(heading_level_from_style("Heading9"), 6);
    }

    #[test]
    fn unsupported_extension_reports_error() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("a.xlsx");
        std::fs::write(&p, "x").unwrap();
        assert!(extract_file(&p).unwrap_err().contains("不支持"));
    }
}
