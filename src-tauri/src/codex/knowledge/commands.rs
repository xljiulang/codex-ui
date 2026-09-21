//! 知识库 Tauri 命令层：列表/删除/建库/取消/检索。
//!
//! 所有实际工作（抽取、切块、向量化、SQLite 混合检索）都在 `codexui-kb.exe` 里完成；
//! 本层负责起子进程、把 NDJSON 进度翻译成事件、维护"同一工作目录只有一个建库任务"的约束。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use super::cli::{self, ChunkHit, IndexProgress, IndexSummary, JobHandle, KbSummary};

/// 进度事件名
pub const EVENT_PROGRESS: &str = "knowledge/index-progress";
/// 完成事件名（成功带 summary、失败带 error）
pub const EVENT_DONE: &str = "knowledge/index-done";
/// 超过该文件数直接转后台，避免长时间挂住对话回合
const BACKGROUND_FILE_THRESHOLD: usize = 300;
/// 同步等待上限：超过即转后台（任务继续跑）
const SYNC_WAIT: Duration = Duration::from_secs(180);
/// 等待首个扫描进度（拿到文件总数）的上限；超时则按"未达阈值"继续同步等待
const SCAN_WAIT: Duration = Duration::from_secs(30);

/// 知识库共享状态：进行中的建库任务（按规范化工作目录索引）
#[derive(Default)]
pub struct KnowledgeState {
    pub jobs: Arc<Mutex<HashMap<String, Arc<JobHandle>>>>,
}

fn app_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录：{e}"))
}

/// 知识库数据根目录（传给子进程的 `--data-dir`）
fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(super::paths::knowledge_dir(&app_dir(app)?))
}

/// 建库任务状态键：规范化工作目录（统一斜杠/大小写，与子进程派生库文件名同口径）
fn workspace_key(cwd: &str) -> String {
    crate::codex::path_util::norm_key(std::path::Path::new(cwd.trim()))
}

/// 各工作目录的知识库列表（由子进程扫 `knowledge/kbs/`，按更新时间倒序）
#[tauri::command]
pub async fn knowledge_list(app: AppHandle) -> Result<Vec<KbSummary>, String> {
    let dir = data_dir(&app)?;
    cli::list(&dir).await
}

/// 删除指定知识库文件（只删索引，不动原始文档与模型）
#[tauri::command]
pub async fn knowledge_delete(app: AppHandle, file: String) -> Result<(), String> {
    let dir = data_dir(&app)?;
    cli::delete(&dir, &file).await
}

/// 检索当前会话工作目录对应的知识库（向量 + 关键词混合）
#[tauri::command]
pub async fn knowledge_search(
    app: AppHandle,
    cwd: String,
    query: String,
    top_k: Option<usize>,
) -> Result<Vec<ChunkHit>, String> {
    let dir = data_dir(&app)?;
    let top = top_k.unwrap_or(cli::DEFAULT_TOP_K);
    cli::search(&dir, &cwd, &query, top).await
}

/// 建库/增量更新（文件数过多或超过同步等待上限时自动转后台，结果经事件回传）
#[tauri::command]
pub async fn knowledge_index_start(
    app: AppHandle,
    state: State<'_, KnowledgeState>,
    cwd: String,
    paths: Option<Vec<String>>,
    full: Option<bool>,
) -> Result<IndexSummary, String> {
    let dir = data_dir(&app)?;
    let cwd = cwd.trim().to_string();
    if cwd.is_empty() {
        return Err("当前会话没有工作目录，无法确定知识库归属（请先选择会话目录）".into());
    }
    let roots: Vec<String> = paths
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let root_labels: Vec<String> = if roots.is_empty() {
        vec![cwd.clone()]
    } else {
        roots.clone()
    };
    let key = workspace_key(&cwd);
    if let Ok(jobs) = state.jobs.lock() {
        if jobs.contains_key(&key) {
            return Err("该工作目录已有一个建库任务在进行中".into());
        }
    }
    let job = Arc::new(JobHandle::default());
    if let Ok(mut jobs) = state.jobs.lock() {
        jobs.insert(key.clone(), job.clone());
    }
    let jobs = state.jobs.clone();
    let cwd_task = cwd.clone();
    let app_task = app.clone();
    let (total_tx, mut total_rx) = tokio::sync::watch::channel(0usize);
    let mut handle = tauri::async_runtime::spawn(async move {
        let mut on_progress = |p: IndexProgress| {
            // 首个扫描进度的 total 用于判定是否转后台
            let _ = total_tx.send(p.total);
            let _ = app_task.emit(EVENT_PROGRESS, &p);
        };
        let result = cli::run_index(
            &dir,
            &cwd_task,
            &roots,
            full.unwrap_or(false),
            &job,
            &mut on_progress,
        )
        .await;
        let payload = match &result {
            Ok(summary) => serde_json::json!({ "cwd": cwd_task, "summary": summary }),
            Err(e) => serde_json::json!({ "cwd": cwd_task, "error": e }),
        };
        let _ = app_task.emit(EVENT_DONE, payload);
        if let Ok(mut map) = jobs.lock() {
            map.remove(&key);
        }
        result
    });

    // 先等首个扫描进度拿到文件总数（子进程在建库前会先报一次总数）
    let mut finished: Option<Result<IndexSummary, String>> = None;
    let mut total = 0usize;
    let deadline = tokio::time::Instant::now() + SCAN_WAIT;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            changed = total_rx.changed() => {
                // 子进程提前结束（无进度）时 sender 被丢弃：直接走结果分支
                if changed.is_err() {
                    break;
                }
                let reported = *total_rx.borrow();
                if reported > 0 {
                    total = reported;
                    break;
                }
            }
            result = &mut handle => {
                finished = Some(result.map_err(|e| format!("建库任务异常结束：{e}"))?);
                break;
            }
        }
    }
    if let Some(result) = finished {
        return result;
    }
    if total > BACKGROUND_FILE_THRESHOLD {
        return Ok(IndexSummary::background_started(&cwd, root_labels, total));
    }
    match tokio::time::timeout(SYNC_WAIT, handle).await {
        Ok(Ok(summary)) => summary,
        Ok(Err(e)) => Err(format!("建库任务异常结束：{e}")),
        // 超时不打断任务：转后台继续，完成后仍会推送事件
        Err(_) => Ok(IndexSummary::background_started(&cwd, root_labels, total)),
    }
}

/// 取消进行中的建库任务（终止子进程；SQLite WAL 保证库仍可用）
#[tauri::command]
pub async fn knowledge_index_cancel(
    state: State<'_, KnowledgeState>,
    cwd: String,
) -> Result<(), String> {
    let key = workspace_key(&cwd);
    let job = state
        .jobs
        .lock()
        .ok()
        .and_then(|map| map.get(&key).cloned());
    match job {
        Some(job) => {
            job.cancel().await;
            Ok(())
        }
        None => Err("该工作目录没有进行中的建库任务".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_starts_empty() {
        let state = KnowledgeState::default();
        assert!(state.jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn workspace_keys_are_normalized() {
        assert_eq!(
            workspace_key("D:\\Work\\售后\\"),
            workspace_key("d:/work/售后")
        );
    }
}
