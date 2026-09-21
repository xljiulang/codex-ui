//! 建库/增量更新任务：扫描 → 抽取 → 切块 → 向量化 → 落库（可取消、按文件 mtime/size 增量）。
//!
//! `data_dir` 为知识库根目录（`--data-dir`，只承载 `kbs/`），`model_dir` 为当前向量模型目录
//! （`--model-dir`，默认与 `codexui-kb.exe` 同目录的 `model/<MODEL_ID>`）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::Serialize;

use super::store::now_secs;
use super::{chunk, embed, extract, store::KbStore};

/// 进度回调载荷
#[derive(Debug, Clone, Serialize)]
pub struct IndexProgress {
    pub cwd: String,
    pub phase: String,
    pub processed: usize,
    pub total: usize,
    pub current: String,
}

/// 建库结果摘要（同时作为 `index_docs` 工具返回值与后台完成事件载荷）
#[derive(Debug, Clone, Serialize, Default)]
pub struct IndexSummary {
    pub cwd: String,
    pub roots: Vec<String>,
    pub total: usize,
    pub added: i64,
    pub updated: i64,
    pub skipped: i64,
    pub removed: i64,
    /// 失败明细（`路径: 原因`，最多保留 10 条）
    pub failed: Vec<String>,
    pub docs: i64,
    pub chunks: i64,
    pub elapsed_ms: u64,
    /// 是否已转后台执行（当次调用只返回启动信息）
    pub background: bool,
    pub cancelled: bool,
    /// 人类可读摘要（由 Rust 生成，供动态工具直接回传给 agent）
    pub text: String,
}

impl IndexSummary {
    /// 后台启动占位：命令层不等待任务完成时返回
    pub fn background_started(cwd: &str, roots: Vec<String>, total: usize) -> Self {
        let mut summary = IndexSummary {
            cwd: cwd.to_string(),
            roots,
            total,
            background: true,
            ..Default::default()
        };
        summary.text = summary.describe();
        summary
    }

    /// 填充 `text` 后返回（命令层统一走这里，避免前端重复拼文案）
    pub fn finalized(mut self) -> Self {
        self.text = self.describe();
        self
    }

    /// 给 agent 的一段人类可读摘要（幂等：无变更时报告现状）
    pub fn describe(&self) -> String {
        if self.background {
            return format!(
                "已在后台开始建库：工作目录 {}，来源 {}，共 {} 个文件。完成后设置页「知识库」列表会更新；\
                 再次调用 index_docs 可查看当前统计。",
                self.cwd,
                self.roots.join("、"),
                self.total
            );
        }
        let mut text = format!(
            "知识库更新{}：新增 {}、更新 {}、跳过 {}、移除失效 {}、失败 {}；当前共 {} 篇文档 / {} 个切块，耗时 {} ms。",
            if self.cancelled { "（已取消，部分完成）" } else { "" },
            self.added,
            self.updated,
            self.skipped,
            self.removed,
            self.failed.len(),
            self.docs,
            self.chunks,
            self.elapsed_ms
        );
        if self.added == 0 && self.updated == 0 && self.removed == 0 && !self.failed.is_empty() {
            text = format!(
                "知识库无变化（内容与上次索引一致）：当前共 {} 篇文档 / {} 个切块；失败 {}。",
                self.docs,
                self.chunks,
                self.failed.len()
            );
        } else if self.added == 0 && self.updated == 0 && self.removed == 0 {
            text = format!(
                "知识库无变化（内容与上次索引一致）：当前共 {} 篇文档 / {} 个切块，来源 {}。",
                self.docs,
                self.chunks,
                self.roots.join("、")
            );
        }
        if !self.failed.is_empty() {
            text.push_str("\n失败明细：\n");
            text.push_str(&self.failed.join("\n"));
        }
        text
    }
}

/// 解析待索引根：缺省用工作目录；相对路径按工作目录展开；不存在的路径直接报错
pub fn resolve_roots(cwd: &str, paths: &[String]) -> Result<Vec<PathBuf>, String> {
    let raw: Vec<String> = if paths.is_empty() {
        vec![cwd.to_string()]
    } else {
        paths.to_vec()
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let candidate = PathBuf::from(&item);
        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            Path::new(cwd).join(candidate)
        };
        if !resolved.exists() {
            return Err(format!("路径不存在：{}", resolved.display()));
        }
        let key = resolved.to_string_lossy().to_lowercase();
        if seen.insert(key) {
            out.push(resolved);
        }
    }
    if out.is_empty() {
        return Err("未指定可索引的路径，且当前会话没有工作目录".into());
    }
    Ok(out)
}

/// 扫描根下支持的文件（去重、排序）
pub fn plan_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for root in roots {
        for file in extract::collect_files(root) {
            let key = file.to_string_lossy().to_lowercase();
            if seen.insert(key) {
                out.push(file);
            }
        }
    }
    out.sort();
    out
}

fn stat(path: &Path) -> Result<(i64, i64), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("读取文件信息失败：{e}"))?;
    let size = meta.len() as i64;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok((size, mtime))
}

/// 执行一次建库（调用方负责放到后台线程；`cancel` 置位后尽快收尾）
pub fn run_index(
    data_dir: &Path,
    model_dir: &Path,
    cwd: &str,
    roots: &[PathBuf],
    full: bool,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(IndexProgress),
) -> Result<IndexSummary, String> {
    let started = Instant::now();
    let store = KbStore::open(data_dir, cwd)?;
    let files = plan_files(roots);
    let total = files.len();
    let mut summary = IndexSummary {
        cwd: cwd.to_string(),
        roots: roots
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        total,
        ..Default::default()
    };
    for root in roots {
        if root.is_dir() {
            store.add_source(&root.to_string_lossy())?;
        }
    }
    on_progress(IndexProgress {
        cwd: cwd.to_string(),
        phase: "scan".into(),
        processed: 0,
        total,
        current: String::new(),
    });

    for (i, file) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        let path_str = file.to_string_lossy().to_string();
        on_progress(IndexProgress {
            cwd: cwd.to_string(),
            phase: "index".into(),
            processed: i,
            total,
            current: path_str.clone(),
        });
        let root_str = roots
            .iter()
            .find(|r| r.is_dir() && file.starts_with(r))
            .map(|r| r.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone());
        let (size, mtime) = match stat(file) {
            Ok(v) => v,
            Err(e) => {
                summary.failed.push(format!("{path_str}: {e}"));
                continue;
            }
        };
        let existing = store.document_state(&path_str);
        let unchanged = matches!(&existing, Some((s, m, status, _))
            if *s == size && *m == mtime && status == "ok")
            && store
                .document_id(&path_str)
                .map(|id| store.chunk_count_for(id) > 0)
                .unwrap_or(false);
        if !full && unchanged {
            summary.skipped += 1;
            continue;
        }

        let doc_id = match store.upsert_document(&root_str, &path_str, size, mtime) {
            Ok(id) => id,
            Err(e) => {
                summary.failed.push(format!("{path_str}: {e}"));
                continue;
            }
        };
        let result = (|| -> Result<usize, String> {
            let text = extract::extract_file(file)?;
            let chunks = chunk::chunk_text(&text);
            if chunks.is_empty() {
                store.replace_chunks(doc_id, &[])?;
                return Ok(0);
            }
            let mut payload: Vec<(String, String, Vec<f32>)> = Vec::with_capacity(chunks.len());
            for group in chunks.chunks(embed::EMBED_BATCH) {
                let texts: Vec<String> = group.iter().map(|c| c.text.clone()).collect();
                let vectors = embed::embed_texts(model_dir, &texts)?;
                for (c, v) in group.iter().zip(vectors.into_iter()) {
                    payload.push((c.title_path.clone(), c.text.clone(), v));
                }
            }
            store.replace_chunks(doc_id, &payload)?;
            Ok(payload.len())
        })();
        match result {
            Ok(_) => {
                store.set_document_status(doc_id, "ok", None)?;
                if existing.is_some() {
                    summary.updated += 1;
                } else {
                    summary.added += 1;
                }
            }
            Err(e) => {
                let _ = store.set_document_status(doc_id, "error", Some(&e));
                summary.failed.push(format!("{path_str}: {e}"));
            }
        }
    }

    for root in roots {
        if root.is_dir() {
            summary.removed += store.prune_missing_under(&root.to_string_lossy())?;
        }
    }
    store.touch_updated()?;
    let (docs, chunks) = store.counts();
    summary.docs = docs;
    summary.chunks = chunks;
    summary.elapsed_ms = started.elapsed().as_millis() as u64;
    summary.failed.truncate(10);
    on_progress(IndexProgress {
        cwd: cwd.to_string(),
        phase: "done".into(),
        processed: total,
        total,
        current: String::new(),
    });
    let _ = now_secs();
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn resolve_roots_defaults_to_cwd_and_expands_relative() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("docs");
        std::fs::create_dir_all(&sub).unwrap();
        let cwd = dir.path().to_string_lossy().to_string();

        let default = resolve_roots(&cwd, &[]).unwrap();
        assert_eq!(default.len(), 1);
        assert!(default[0].to_string_lossy().contains("docs") || default[0].exists());

        let relative = resolve_roots(&cwd, &["docs".to_string()]).unwrap();
        assert_eq!(relative[0], sub);
        assert!(resolve_roots(&cwd, &["missing".to_string()]).is_err());
    }

    #[test]
    fn plan_files_dedupes_and_sorts() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("b.md"), "b").unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("c.png"), "c").unwrap();
        let roots = vec![dir.path().to_path_buf(), dir.path().to_path_buf()];
        let files = plan_files(&roots);
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with("a.txt"));
    }

    #[test]
    fn summary_describes_idempotent_state() {
        let summary = IndexSummary {
            cwd: "D:\\kb".into(),
            roots: vec!["D:\\kb".into()],
            docs: 3,
            chunks: 12,
            ..Default::default()
        };
        let text = summary.describe();
        assert!(text.contains("无变化"));
        assert!(text.contains("3 篇"));
        let background = IndexSummary::background_started("D:\\kb", vec!["D:\\kb".into()], 500);
        assert!(background.describe().contains("后台"));
    }
}
