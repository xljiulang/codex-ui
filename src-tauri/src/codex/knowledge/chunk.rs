//! 文本切块：按标题层级与段落聚合到目标长度，带重叠，并保留标题路径供引用。

/// 单个切块的目标字符数
pub const CHUNK_TARGET_CHARS: usize = 500;
/// 相邻切块的重叠字符数（保留上下文，降低答案被切断的概率）
pub const CHUNK_OVERLAP_CHARS: usize = 80;
/// 单块硬上限：超长段落按句读再切
pub const CHUNK_MAX_CHARS: usize = 1200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// 标题路径，如 `手册 › 第3章 故障 › 3.2 无法开机`；无标题时为空串
    pub title_path: String,
    pub text: String,
}

/// 解析 Markdown 风格标题（`#` 1-6 个 + 空格）
fn heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if rest.starts_with(' ') || rest.starts_with('\t') {
        Some(hashes)
    } else {
        None
    }
}

fn heading_text(line: &str) -> String {
    line.trim_start_matches('#').trim().to_string()
}

/// 字符数（按 Unicode 标量计），用于长度判定
fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// 取尾部 `max` 个字符作为重叠前缀
fn tail_chars(s: &str, max: usize) -> String {
    let count = char_len(s);
    if count <= max {
        return s.to_string();
    }
    s.chars().skip(count - max).collect()
}

/// 超长段落按中英文句读切分为不超过 `CHUNK_MAX_CHARS` 的片段
fn split_long_paragraph(paragraph: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in paragraph.chars() {
        current.push(ch);
        let len = char_len(&current);
        let is_break = matches!(ch, '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';' | '\n');
        // 优先在句读处断开（达到目标长度后），无句读则硬切到上限
        if len >= CHUNK_MAX_CHARS || (is_break && len >= CHUNK_TARGET_CHARS) {
            out.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out.retain(|s| !s.is_empty());
    out
}

/// 切块主流程：标题行更新标题栈并独占成块（便于精确定位章节），正文按段聚合。
pub fn chunk_text(text: &str) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut buffer = String::new();

    fn title_path(stack: &[(usize, String)]) -> String {
        stack
            .iter()
            .map(|(_, t)| t.clone())
            .collect::<Vec<_>>()
            .join(" › ")
    }

    /// 把积累正文封成一个块，返回是否真的封了
    fn flush(out: &mut Vec<Chunk>, stack: &[(usize, String)], buffer: &mut String) -> bool {
        let body = buffer.trim().to_string();
        buffer.clear();
        if body.is_empty() {
            return false;
        }
        out.push(Chunk {
            title_path: title_path(stack),
            text: body,
        });
        true
    }

    for raw_line in text.lines() {
        if let Some(level) = heading_level(raw_line) {
            flush(&mut out, &stack, &mut buffer);
            stack.retain(|(l, _)| *l < level);
            stack.push((level, heading_text(raw_line)));
            continue;
        }
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            // 空行作为段落边界：仅当已积累内容时补一个换行
            if !buffer.is_empty() && !buffer.ends_with('\n') {
                buffer.push('\n');
            }
            continue;
        }
        if !buffer.is_empty() && !buffer.ends_with('\n') {
            buffer.push('\n');
        }
        buffer.push_str(line);
        if char_len(&buffer) >= CHUNK_TARGET_CHARS {
            // 达到目标即封块，并带入尾部的重叠上下文
            let body = buffer.trim().to_string();
            let overlap = tail_chars(&body, CHUNK_OVERLAP_CHARS);
            if flush(&mut out, &stack, &mut buffer) && !overlap.trim().is_empty() {
                buffer.push_str(overlap.trim());
                buffer.push('\n');
            }
        }
    }
    flush(&mut out, &stack, &mut buffer);

    // 二次处理：把任何超长块按句读拆开（含标题单独成块的情形）
    let mut normalized: Vec<Chunk> = Vec::new();
    for chunk in out {
        if char_len(&chunk.text) <= CHUNK_MAX_CHARS {
            normalized.push(chunk);
            continue;
        }
        for piece in split_long_paragraph(&chunk.text) {
            normalized.push(Chunk {
                title_path: chunk.title_path.clone(),
                text: piece,
            });
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join(chunks: &[Chunk]) -> String {
        chunks
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn keeps_heading_path_and_splits_long_body() {
        let body = "故障现象描述。".repeat(300);
        let text = format!("# 售后手册\n\n## 第3章 故障\n\n### 3.2 无法开机\n\n{body}");
        let chunks = chunk_text(&text);
        assert!(chunks.len() >= 3, "长正文应被切成多块：{}", chunks.len());
        assert!(chunks
            .iter()
            .all(|c| c.title_path == "售后手册 › 第3章 故障 › 3.2 无法开机"));
        assert!(chunks.iter().all(|c| char_len(&c.text) <= CHUNK_MAX_CHARS));
        // 正文内容不丢（允许重叠带来的重复）
        let joined = join(&chunks);
        assert!(joined.contains("故障现象描述。"));
    }

    #[test]
    fn overlap_carries_context_between_chunks() {
        let text = "第一段内容。".repeat(200);
        let chunks = chunk_text(&text);
        assert!(chunks.len() >= 2);
        let first_tail = tail_chars(&chunks[0].text, CHUNK_OVERLAP_CHARS);
        assert!(
            chunks[1].text.contains(first_tail.trim()),
            "第二块应带上第一块尾部的重叠上下文"
        );
    }

    #[test]
    fn heading_only_chapter_is_kept_as_its_own_chunk() {
        let text = "# 标题\n\n## 有内容的章节\n\n正文一。\n\n## 只有标题的章节\n\n## 下一章\n\n正文二。";
        let chunks = chunk_text(text);
        let titles: Vec<String> = chunks.iter().map(|c| c.title_path.clone()).collect();
        assert!(titles.iter().any(|t| t == "标题 › 有内容的章节"));
        assert!(titles.iter().any(|t| t == "标题 › 下一章"));
        assert!(chunks.iter().any(|c| c.text.contains("正文二。")));
    }

    #[test]
    fn mixed_cjk_and_ascii_hard_wrap_without_punctuation() {
        let text = "a".repeat(CHUNK_MAX_CHARS * 2);
        let chunks = chunk_text(&text);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| char_len(&c.text) <= CHUNK_MAX_CHARS));
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        assert!(chunk_text("").is_empty());
        assert!(chunk_text("   \n\n  ").is_empty());
    }
}
