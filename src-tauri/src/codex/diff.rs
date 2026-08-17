use serde::{Deserialize, Serialize};

/// diff 预览参数（由主窗口传入新窗口）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffPreviewParams {
    pub path: String,
    pub kind: String,
    pub diff: String,
    pub workspace: String,
}

/// 内联 diff 行（serde 标签枚举，字段 camelCase 与前端一致）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DiffRow {
    Ctx {
        #[serde(rename = "oldNo")]
        old_no: u32,
        #[serde(rename = "newNo")]
        new_no: u32,
        text: String,
    },
    Del {
        #[serde(rename = "oldNo")]
        old_no: u32,
        text: String,
    },
    Add {
        #[serde(rename = "newNo")]
        new_no: u32,
        text: String,
    },
    Sep,
}

#[derive(Debug, Clone, PartialEq)]
enum DiffLineKind {
    Ctx,
    Add,
    Del,
}

struct DiffHunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<(DiffLineKind, String)>,
}

/// 按行拆分文本（去掉末尾空元素，兼容 \r\n）
fn split_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = content
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    if lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize, usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_part, after) = rest.split_once(" +")?;
    let (new_part, tail) = after.split_once(" @@")?;
    let _ = tail;
    let parse_count = |s: &str| -> (usize, usize) {
        if let Some((a, b)) = s.split_once(',') {
            (a.parse().unwrap_or(1), b.parse().unwrap_or(0))
        } else {
            (s.parse().unwrap_or(1), 1)
        }
    };
    let (os, oc) = parse_count(old_part);
    let (ns, nc) = parse_count(new_part);
    Some((os, oc, ns, nc))
}

/// 解析 unified diff 的 hunks；忽略 diff 头与 “\ No newline” 标记与空行
fn parse_unified_diff(diff: &str) -> Result<Vec<DiffHunk>, String> {
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current: Option<DiffHunk> = None;
    for raw in diff.split('\n') {
        let line = raw.trim_end_matches('\r');
        if line.starts_with('\\') || line.is_empty() {
            continue;
        }
        if let Some((os, oc, ns, nc)) = parse_hunk_header(line) {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(DiffHunk {
                old_start: os,
                old_count: oc,
                new_start: ns,
                new_count: nc,
                lines: Vec::new(),
            });
            continue;
        }
        if let Some(h) = current.as_mut() {
            if let Some(rest) = line.strip_prefix('+') {
                h.lines.push((DiffLineKind::Add, rest.to_string()));
            } else if let Some(rest) = line.strip_prefix('-') {
                h.lines.push((DiffLineKind::Del, rest.to_string()));
            } else {
                let text = line.strip_prefix(' ').unwrap_or(line).to_string();
                h.lines.push((DiffLineKind::Ctx, text));
            }
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    Ok(hunks)
}

fn validate_hunks(
    hunks: &[DiffHunk],
    old_lines: Option<&[String]>,
    new_lines: &[String],
) -> Result<(), String> {
    for h in hunks {
        let ctx = h
            .lines
            .iter()
            .filter(|(k, _)| *k == DiffLineKind::Ctx)
            .count();
        let del = h
            .lines
            .iter()
            .filter(|(k, _)| *k == DiffLineKind::Del)
            .count();
        let add = h
            .lines
            .iter()
            .filter(|(k, _)| *k == DiffLineKind::Add)
            .count();
        if ctx + del != h.old_count || ctx + add != h.new_count {
            return Err("diff hunk 计数不一致".to_string());
        }
        if h.new_start.saturating_sub(1) > new_lines.len() {
            return Err("diff hunk 超出新内容范围".to_string());
        }
        if let Some(old) = old_lines {
            if h.old_start.saturating_sub(1) > old.len() {
                return Err("diff hunk 超出旧内容范围".to_string());
            }
        }
        validate_hunk_content(h, old_lines, new_lines)?;
    }
    Ok(())
}

/// 校验 hunk 的上下文/删除/新增行与新旧文件内容逐行一致，
/// 防止 diff 过期时静默重建出错误内容。
fn validate_hunk_content(
    h: &DiffHunk,
    old_lines: Option<&[String]>,
    new_lines: &[String],
) -> Result<(), String> {
    let mut oi = h.old_start.saturating_sub(1);
    let mut ni = h.new_start.saturating_sub(1);
    for (kind, text) in &h.lines {
        match kind {
            DiffLineKind::Ctx => {
                if let Some(old) = old_lines {
                    if oi >= old.len() || old[oi] != *text {
                        return Err("diff 上下文与旧文件内容不一致（diff 可能已过期）".into());
                    }
                }
                if ni >= new_lines.len() || new_lines[ni] != *text {
                    return Err("diff 上下文与新文件内容不一致（diff 可能已过期）".into());
                }
                oi += 1;
                ni += 1;
            }
            DiffLineKind::Del => {
                if let Some(old) = old_lines {
                    if oi >= old.len() || old[oi] != *text {
                        return Err("diff 删除行与旧文件内容不一致（diff 可能已过期）".into());
                    }
                }
                oi += 1;
            }
            DiffLineKind::Add => {
                if ni >= new_lines.len() || new_lines[ni] != *text {
                    return Err("diff 新增行与新文件内容不一致（diff 可能已过期）".into());
                }
                ni += 1;
            }
        }
    }
    Ok(())
}

/// 反向应用 unified diff：由新内容重建旧内容（自底向上替换）
pub fn apply_reverse_unified_diff(new_content: &str, diff: &str) -> Result<String, String> {
    let hunks = parse_unified_diff(diff)?;
    let mut new_lines = split_lines(new_content);
    validate_hunks(&hunks, None, &new_lines)?;
    for h in hunks.iter().rev() {
        let mut old_region: Vec<String> = Vec::new();
        let mut new_region_len = 0usize;
        for (kind, text) in &h.lines {
            match kind {
                DiffLineKind::Del => old_region.push(text.clone()),
                DiffLineKind::Ctx => {
                    old_region.push(text.clone());
                    new_region_len += 1;
                }
                DiffLineKind::Add => new_region_len += 1,
            }
        }
        let start = h.new_start.saturating_sub(1).min(new_lines.len());
        let del_count = new_region_len.min(new_lines.len().saturating_sub(start));
        new_lines.splice(start..start + del_count, old_region);
    }
    Ok(new_lines.join("\n"))
}

/// 构建单栏内联行：hunk 外未变行逐行配对，hunk 内旧行（红）在上、新行（绿）在下、中间分隔
pub fn build_inline_rows(
    old_content: &str,
    new_content: &str,
    diff: &str,
) -> Result<Vec<DiffRow>, String> {
    let hunks = parse_unified_diff(diff)?;
    let old_lines = split_lines(old_content);
    let new_lines = split_lines(new_content);
    validate_hunks(&hunks, Some(&old_lines), &new_lines)?;
    let mut rows: Vec<DiffRow> = Vec::new();
    let mut old_idx = 0usize;
    let mut new_idx = 0usize;

    macro_rules! emit_ctx_pair {
        () => {{
            let o = old_lines.get(old_idx).cloned();
            let n = new_lines.get(new_idx).cloned();
            rows.push(DiffRow::Ctx {
                old_no: (old_idx + 1) as u32,
                new_no: (new_idx + 1) as u32,
                text: o.or(n).unwrap_or_default(),
            });
            old_idx += 1;
            new_idx += 1;
        }};
    }

    for h in &hunks {
        let pre = (h.old_start.saturating_sub(1).saturating_sub(old_idx))
            .max(h.new_start.saturating_sub(1).saturating_sub(new_idx));
        for _ in 0..pre {
            emit_ctx_pair!();
        }

        rows.extend(build_hunk_rows(h));
        old_idx += h.old_count;
        new_idx += h.new_count;
    }
    while old_idx < old_lines.len() || new_idx < new_lines.len() {
        emit_ctx_pair!();
    }
    Ok(rows)
}

/// 由 git 生成的单文件 unified diff 直接构建内联行（提交内容不可变，无需磁盘内容校验）。
/// 与 build_inline_rows 的 hunk 内逻辑一致：上下文在前、删除行、分隔、新增行、上下文在后；
/// 不补全 hunk 外内容（git diff 输出本就只含变更区域，适合历史提交展示）。
pub fn build_commit_diff_rows(diff: &str) -> Result<Vec<DiffRow>, String> {
    let hunks = parse_unified_diff(diff)?;
    let mut rows: Vec<DiffRow> = Vec::new();
    for h in &hunks {
        rows.extend(build_hunk_rows(h));
    }
    Ok(rows)
}

/// 构建单个 hunk 的内联行。
///
/// 一个 hunk 可能包含多个变更组（如 ctx→del→add→ctx→add→ctx）。
/// 每遇到上下文行先冲刷当前挂起的变更组（删除行 → 分隔 → 新增行），
/// 再输出该上下文行，保证新旧两侧行号各自单调，不再出现
/// “新增行号 417 之后又出现 405”这类乱序。
fn build_hunk_rows(h: &DiffHunk) -> Vec<DiffRow> {
    let mut rows: Vec<DiffRow> = Vec::new();
    let mut o = h.old_start;
    let mut n = h.new_start;
    let mut pending_dels: Vec<(u32, String)> = Vec::new();
    let mut pending_adds: Vec<(u32, String)> = Vec::new();

    macro_rules! flush_pending {
        () => {{
            let has_del = !pending_dels.is_empty();
            let has_add = !pending_adds.is_empty();
            if has_del || has_add {
                rows.extend(
                    pending_dels
                        .drain(..)
                        .map(|(old_no, text)| DiffRow::Del { old_no, text }),
                );
                if has_del && has_add {
                    rows.push(DiffRow::Sep);
                }
                rows.extend(
                    pending_adds
                        .drain(..)
                        .map(|(new_no, text)| DiffRow::Add { new_no, text }),
                );
            }
        }};
    }

    for (kind, text) in &h.lines {
        match kind {
            DiffLineKind::Ctx => {
                flush_pending!();
                rows.push(DiffRow::Ctx {
                    old_no: o as u32,
                    new_no: n as u32,
                    text: text.clone(),
                });
                o += 1;
                n += 1;
            }
            DiffLineKind::Del => {
                pending_dels.push((o as u32, text.clone()));
                o += 1;
            }
            DiffLineKind::Add => {
                pending_adds.push((n as u32, text.clone()));
                n += 1;
            }
        }
    }
    flush_pending!();
    rows
}

/// Windows 路径解析：绝对盘符路径/UNC 直接返回，相对路径按工作目录拼接；
/// drive-relative（`C:foo`）无法确定驱动器当前目录，直接报错。
fn resolve_path(p: &str, root: &str) -> Result<String, String> {
    let h = p.replace('\\', "/");
    let bytes = h.as_bytes();
    let is_drive_abs = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'/';
    if is_drive_abs || h.starts_with("//") {
        return Ok(h.replace('/', "\\"));
    }
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(format!("不支持驱动器相对路径: {p}"));
    }
    let trimmed = h.trim_start_matches('/');
    let base = root.replace('\\', "/").trim_end_matches('/').to_string();
    Ok(format!("{}/{}", base, trimmed).replace('/', "\\"))
}

/// 一次 IPC 完成：路径解析 + 读文件 + 反向重建 + 内联行生成
#[tauri::command]
pub fn build_diff_preview(params: DiffPreviewParams) -> Result<Vec<DiffRow>, String> {
    let abs = resolve_path(&params.path, &params.workspace)?;
    let new_content = if params.kind == "delete" {
        String::new()
    } else {
        std::fs::read_to_string(&abs)
            .map_err(|e| format!("无法读取文件 {}: {}", abs, e))?
    };
    let old_content = apply_reverse_unified_diff(&new_content, &params.diff)?;
    build_inline_rows(&old_content, &new_content, &params.diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skips_headers_and_no_newline() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n\\ No newline at end of file\n";
        let hunks = parse_unified_diff(diff).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[0].old_count, 2);
        assert_eq!(hunks[0].new_start, 1);
        assert_eq!(hunks[0].new_count, 2);
        assert_eq!(hunks[0].lines.len(), 3);
    }

    #[test]
    fn parse_missing_counts_defaults_one() {
        let hunks = parse_unified_diff("@@ -5 +7 @@\n x").unwrap();
        assert_eq!(hunks[0].old_start, 5);
        assert_eq!(hunks[0].old_count, 1);
        assert_eq!(hunks[0].new_start, 7);
        assert_eq!(hunks[0].new_count, 1);
    }

    const REPLACE_DIFF: &str = "@@ -1,3 +1,3 @@\n a\n-b\n+X\n c";

    #[test]
    fn reverse_single_hunk() {
        assert_eq!(
            apply_reverse_unified_diff("a\nX\nc", REPLACE_DIFF).unwrap(),
            "a\nb\nc"
        );
    }

    #[test]
    fn reverse_multi_hunk() {
        let old_lines: Vec<String> = (1..=10).map(|i| format!("line{}", i)).collect();
        let mut new_lines = old_lines.clone();
        new_lines[2] = "changed3".into();
        new_lines[7] = "changed8".into();
        let diff = "@@ -3,1 +3,1 @@\n-line3\n+changed3\n@@ -8,1 +8,1 @@\n-line8\n+changed8";
        assert_eq!(
            apply_reverse_unified_diff(&new_lines.join("\n"), diff).unwrap(),
            old_lines.join("\n")
        );
    }

    #[test]
    fn reverse_add_and_delete() {
        assert_eq!(
            apply_reverse_unified_diff("n1\nn2", "@@ -0,0 +1,2 @@\n+n1\n+n2").unwrap(),
            ""
        );
        assert_eq!(
            apply_reverse_unified_diff("", "@@ -1,3 +0,0 @@\n-a\n-b\n-c").unwrap(),
            "a\nb\nc"
        );
    }

    #[test]
    fn reverse_trailing_newline_in_diff() {
        assert_eq!(
            apply_reverse_unified_diff("a\nX\nc", &format!("{}\n", REPLACE_DIFF)).unwrap(),
            "a\nb\nc"
        );
    }

    #[test]
    fn inline_rows_alignment_and_numbers() {
        let rows = build_inline_rows("a\nb\nc", "a\nX\nc", REPLACE_DIFF).unwrap();
        assert_eq!(rows.len(), 5);
        assert!(matches!(&rows[0], DiffRow::Ctx { old_no: 1, new_no: 1, text } if text == "a"));
        assert!(matches!(&rows[1], DiffRow::Del { old_no: 2, text } if text == "b"));
        assert!(matches!(&rows[2], DiffRow::Sep));
        assert!(matches!(&rows[3], DiffRow::Add { new_no: 2, text } if text == "X"));
        assert!(matches!(&rows[4], DiffRow::Ctx { old_no: 3, new_no: 3, text } if text == "c"));
    }

    #[test]
    fn inline_rows_full_coverage() {
        let old_lines: Vec<String> = (1..=10).map(|i| format!("line{}", i)).collect();
        let mut new_lines = old_lines.clone();
        new_lines[4] = "changed5".into();
        let diff = "@@ -5,1 +5,1 @@\n-line5\n+changed5";
        let rows = build_inline_rows(&old_lines.join("\n"), &new_lines.join("\n"), diff).unwrap();
        assert_eq!(rows.len(), 12);
        assert!(matches!(&rows[4], DiffRow::Del { old_no: 5, .. }));
        assert!(matches!(&rows[5], DiffRow::Sep));
        assert!(matches!(&rows[6], DiffRow::Add { new_no: 5, .. }));
        assert!(matches!(&rows[11], DiffRow::Ctx { old_no: 10, new_no: 10, .. }));
    }

    #[test]
    fn inline_rows_pure_add_no_sep() {
        let rows = build_inline_rows("", "n1\nn2", "@@ -0,0 +1,2 @@\n+n1\n+n2").unwrap();
        assert_eq!(rows.iter().filter(|r| matches!(r, DiffRow::Sep)).count(), 0);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn inline_rows_multi_change_group_keeps_monotonic_numbers() {
        // 一个 hunk 内含多个变更组（ctx→del→add→ctx→add→ctx），
        // 旧代码会把第二个 add 组后的上下文整体挪到所有 add 之后，
        // 导致新文件行号乱序（如 …2、4、3、5…）；修复后必须按实际位置穿插输出。
        let old_lines = vec!["line1", "line2", "line3", "line5"];
        let new_lines = vec!["line1", "X", "line3", "Y", "line5"];
        let diff = "@@ -1,4 +1,5 @@\n line1\n-line2\n+X\n line3\n+Y\n line5";
        let rows = build_inline_rows(
            &old_lines.join("\n"),
            &new_lines.join("\n"),
            diff,
        )
        .unwrap();
        assert!(matches!(&rows[0], DiffRow::Ctx { old_no: 1, new_no: 1, .. }));
        assert!(matches!(&rows[1], DiffRow::Del { old_no: 2, .. }));
        assert!(matches!(&rows[2], DiffRow::Sep));
        assert!(matches!(&rows[3], DiffRow::Add { new_no: 2, .. }));
        assert!(matches!(&rows[4], DiffRow::Ctx { old_no: 3, new_no: 3, .. }));
        assert!(matches!(&rows[5], DiffRow::Add { new_no: 4, .. }));
        assert!(matches!(&rows[6], DiffRow::Ctx { old_no: 4, new_no: 5, .. }));
        // 新旧两侧行号必须各自严格单调递增
        let mut last_old = 0u32;
        let mut last_new = 0u32;
        for r in &rows {
            match r {
                DiffRow::Ctx { old_no, new_no, .. } => {
                    assert!(*old_no > last_old);
                    assert!(*new_no > last_new);
                    last_old = *old_no;
                    last_new = *new_no;
                }
                DiffRow::Del { old_no, .. } => {
                    assert!(*old_no > last_old);
                    last_old = *old_no;
                }
                DiffRow::Add { new_no, .. } => {
                    assert!(*new_no > last_new);
                    last_new = *new_no;
                }
                DiffRow::Sep => {}
            }
        }
    }

    #[test]
    fn commit_diff_rows_multi_change_group_keeps_monotonic_numbers() {
        let diff = "@@ -1,4 +1,5 @@\n line1\n-line2\n+X\n line3\n+Y\n line5";
        let rows = build_commit_diff_rows(diff).unwrap();
        assert!(matches!(&rows[0], DiffRow::Ctx { old_no: 1, new_no: 1, .. }));
        assert!(matches!(&rows[1], DiffRow::Del { old_no: 2, .. }));
        assert!(matches!(&rows[2], DiffRow::Sep));
        assert!(matches!(&rows[3], DiffRow::Add { new_no: 2, .. }));
        assert!(matches!(&rows[4], DiffRow::Ctx { old_no: 3, new_no: 3, .. }));
        assert!(matches!(&rows[5], DiffRow::Add { new_no: 4, .. }));
        assert!(matches!(&rows[6], DiffRow::Ctx { old_no: 4, new_no: 5, .. }));
    }

    #[test]
    fn invalid_hunk_counts_errors() {
        assert!(build_inline_rows("a", "x", "@@ -1,99 +1,1 @@\n+x").is_err());
    }

    #[test]
    fn stale_diff_content_errors() {
        // diff 上下文/新增行与当前文件内容不一致（文件在生成 diff 后又改动）→ 报错而非静默重建
        let diff = "@@ -1,3 +1,3 @@\n a\n-b\n+X\n c";
        assert!(build_inline_rows("a\nb\nc", "a\nY\nc", diff).is_err());
        assert!(apply_reverse_unified_diff("a\nY\nc", diff).is_err());
        assert!(apply_reverse_unified_diff("a\nX\nc", diff).is_ok());
    }

    #[test]
    fn resolve_path_rules() {
        // 绝对盘符路径原样返回（反斜杠）
        assert_eq!(resolve_path("C:/a/b.txt", "D:\\root").unwrap(), "C:\\a\\b.txt");
        // UNC 路径原样返回
        assert_eq!(
            resolve_path(r"\\srv\share\f.txt", "D:\\root").unwrap(),
            "\\\\srv\\share\\f.txt"
        );
        // 相对路径按 root 拼接，根目录尾分隔符不产生双斜杠
        assert_eq!(
            resolve_path("src/a.txt", "D:\\root").unwrap(),
            "D:\\root\\src\\a.txt"
        );
        assert_eq!(
            resolve_path("a.txt", "D:\\root\\").unwrap(),
            "D:\\root\\a.txt"
        );
        // drive-relative（C:foo）无法确定基准，直接拒绝
        assert!(resolve_path("C:foo", "D:\\root").is_err());
    }

    #[test]
    fn large_file_ten_thousand_lines() {
        let old_lines: Vec<String> = (1..=10_000).map(|i| format!("line{}", i)).collect();
        let mut new_lines = old_lines.clone();
        new_lines[5000] = "changed".into();
        let diff = "@@ -5001,1 +5001,1 @@\n-line5001\n+changed";
        let rows =
            build_inline_rows(&old_lines.join("\n"), &new_lines.join("\n"), diff).unwrap();
        assert_eq!(rows.len(), 10_002);
        assert!(matches!(&rows[5000], DiffRow::Del { .. }));
        assert!(matches!(&rows[5002], DiffRow::Add { .. }));
    }

    #[test]
    fn build_diff_preview_reads_and_parses() {
        let dir = std::env::temp_dir().join(format!("codexui-diff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "a\nX\nc\n").unwrap();
        let params = DiffPreviewParams {
            path: p.to_string_lossy().into_owned(),
            kind: "update".into(),
            diff: REPLACE_DIFF.to_string(),
            workspace: dir.to_string_lossy().into_owned(),
        };
        let rows = build_diff_preview(params).unwrap();
        assert_eq!(rows.len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_diff_preview_relative_path_resolves() {
        let dir = std::env::temp_dir().join(format!("codexui-diff-rel-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.txt"), "a\nX\nc\n").unwrap();
        let params = DiffPreviewParams {
            path: "src/a.txt".into(),
            kind: "update".into(),
            diff: REPLACE_DIFF.to_string(),
            workspace: dir.to_string_lossy().into_owned(),
        };
        assert_eq!(build_diff_preview(params).unwrap().len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_diff_preview_missing_file_errors() {
        let params = DiffPreviewParams {
            path: "C:\\definitely-missing-diff-preview.txt".into(),
            kind: "update".into(),
            diff: REPLACE_DIFF.to_string(),
            workspace: String::new(),
        };
        assert!(build_diff_preview(params).is_err());
    }

    #[test]
    fn commit_diff_rows_from_git_patch() {
        let diff = "diff --git a/a.txt b/a.txt\n\
index 111..222 100644\n\
--- a/a.txt\n\
+++ b/a.txt\n\
@@ -1,3 +1,3 @@\n\
 a\n\
-b\n\
+B\n\
 c\n\
@@ -10 +10 @@\n\
-p\n\
+q\n";
        let rows = build_commit_diff_rows(diff).unwrap();
        assert!(matches!(&rows[0], DiffRow::Ctx { text, .. } if text == "a"));
        assert!(matches!(&rows[1], DiffRow::Del { old_no: 2, .. }));
        assert!(matches!(&rows[2], DiffRow::Sep));
        assert!(matches!(&rows[3], DiffRow::Add { new_no: 2, .. }));
        assert!(matches!(&rows[4], DiffRow::Ctx { old_no: 3, new_no: 3, .. }));
        assert!(matches!(&rows[5], DiffRow::Del { old_no: 10, .. }));
        assert!(matches!(&rows[6], DiffRow::Sep));
        assert!(matches!(&rows[7], DiffRow::Add { new_no: 10, .. }));
    }

    #[test]
    fn commit_diff_rows_skip_rename_and_empty() {
        let rename_diff = "diff --git a/old.txt b/new.txt\n\
similarity index 100%\n\
rename from old.txt\n\
rename to new.txt\n";
        assert!(build_commit_diff_rows(rename_diff).unwrap().is_empty());
        assert!(build_commit_diff_rows("").unwrap().is_empty());
    }

}
