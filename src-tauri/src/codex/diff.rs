use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::Manager;

/// diff 预览参数（由主窗口传入新窗口）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffPreviewParams {
    pub path: String,
    pub kind: String,
    pub diff: String,
    pub workspace_root: String,
}

/// 新窗口取走参数的共享状态
pub struct DiffParamsState(pub Mutex<Option<DiffPreviewParams>>);

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

        let mut o = h.old_start;
        let mut n = h.new_start;
        let mut saw_change = false;
        let mut before_ctx: Vec<DiffRow> = Vec::new();
        let mut after_ctx: Vec<DiffRow> = Vec::new();
        let mut dels: Vec<(u32, String)> = Vec::new();
        let mut adds: Vec<(u32, String)> = Vec::new();
        for (kind, text) in &h.lines {
            match kind {
                DiffLineKind::Ctx => {
                    let row = DiffRow::Ctx {
                        old_no: o as u32,
                        new_no: n as u32,
                        text: text.clone(),
                    };
                    o += 1;
                    n += 1;
                    if saw_change {
                        after_ctx.push(row);
                    } else {
                        before_ctx.push(row);
                    }
                }
                DiffLineKind::Del => {
                    dels.push((o as u32, text.clone()));
                    o += 1;
                    saw_change = true;
                }
                DiffLineKind::Add => {
                    adds.push((n as u32, text.clone()));
                    n += 1;
                    saw_change = true;
                }
            }
        }
        let has_del = !dels.is_empty();
        let has_add = !adds.is_empty();
        rows.extend(before_ctx);
        rows.extend(
            dels
                .into_iter()
                .map(|(old_no, text)| DiffRow::Del { old_no, text }),
        );
        if has_del && has_add {
            rows.push(DiffRow::Sep);
        }
        rows.extend(
            adds
                .into_iter()
                .map(|(new_no, text)| DiffRow::Add { new_no, text }),
        );
        rows.extend(after_ctx);
        old_idx += h.old_count;
        new_idx += h.new_count;
    }
    while old_idx < old_lines.len() || new_idx < new_lines.len() {
        emit_ctx_pair!();
    }
    Ok(rows)
}

fn is_abs_path(h: &str) -> bool {
    let b = h.as_bytes();
    b.len() >= 3 && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')
}

/// Windows 路径解析：绝对盘符路径直接返回，相对路径按工作目录拼接
fn resolve_path(p: &str, root: &str) -> String {
    let h = p.replace('\\', "/");
    if is_abs_path(&h) {
        return h.replace('/', "\\");
    }
    let trimmed = h.trim_start_matches('/');
    if trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':' {
        return trimmed.replace('/', "\\");
    }
    let base = root.replace('\\', "/").trim_end_matches('/').to_string();
    format!("{}/{}", base, trimmed).replace('/', "\\")
}

/// 一次 IPC 完成：路径解析 + 读文件 + 反向重建 + 内联行生成
#[tauri::command]
pub fn build_diff_preview(params: DiffPreviewParams) -> Result<Vec<DiffRow>, String> {
    let abs = resolve_path(&params.path, &params.workspace_root);
    let new_content = if params.kind == "delete" {
        String::new()
    } else {
        std::fs::read_to_string(&abs)
            .map_err(|e| format!("无法读取文件 {}: {}", abs, e))?
    };
    let old_content = apply_reverse_unified_diff(&new_content, &params.diff)?;
    build_inline_rows(&old_content, &new_content, &params.diff)
}

pub fn take_params(state: &DiffParamsState) -> Option<DiffPreviewParams> {
    state.0.lock().unwrap().take()
}

#[tauri::command]
pub fn take_diff_params(state: tauri::State<'_, DiffParamsState>) -> Option<DiffPreviewParams> {
    take_params(state.inner())
}

/// 打开独立 diff 窗口；参数存入共享状态，由新窗口一次性取走
#[tauri::command]
pub async fn open_diff_window(
    app: tauri::AppHandle,
    params: DiffPreviewParams,
) -> Result<(), String> {
    if let Some(state) = app.try_state::<DiffParamsState>() {
        *state.0.lock().unwrap() = Some(params);
    }
    // 窗口创建必须发生在主线程消息泵上：同步创建 WebView2 窗口会阻塞主线程导致死锁
    // （新窗口停在 about:blank、invoke 永不返回），因此用 run_on_main_thread 异步创建。
    let app2 = app.clone();
    app.run_on_main_thread(move || {
        if let Some(win) = app2.get_webview_window("diff-preview") {
            // 复用已有窗口：更新参数后重新加载，避免 close+新建同 label 窗口的竞态
            let _ = win.eval("location.reload()");
            return;
        }
        if let Err(e) = tauri::WebviewWindowBuilder::new(
            &app2,
            "diff-preview",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("文件差异预览")
        .inner_size(1200.0, 800.0)
        .min_inner_size(600.0, 400.0)
        .center()
        .resizable(true)
        .maximized(true)
        .build()
        {
            eprintln!("open diff window failed: {e}");
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(())
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
    fn invalid_hunk_counts_errors() {
        assert!(build_inline_rows("a", "x", "@@ -1,99 +1,1 @@\n+x").is_err());
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
            workspace_root: dir.to_string_lossy().into_owned(),
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
            workspace_root: dir.to_string_lossy().into_owned(),
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
            workspace_root: String::new(),
        };
        assert!(build_diff_preview(params).is_err());
    }

    #[test]
    fn take_params_consumes_once() {
        let state = DiffParamsState(Mutex::new(Some(DiffPreviewParams {
            path: "x".into(),
            kind: "update".into(),
            diff: "".into(),
            workspace_root: "".into(),
        })));
        assert!(take_params(&state).is_some());
        assert!(take_params(&state).is_none());
    }
}
