//! 知识库 Tauri 命令层：列表/删除/建库/取消/检索。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use super::index::{self, IndexProgress, IndexSummary};
use super::search::{self, VectorCache};
use super::store::{self, ChunkHit, KbSummary};

/// 进度事件名
pub const EVENT_PROGRESS: &str = "knowledge/index-progress";
/// 完成事件名（成功带 summary、失败带 error）
pub const EVENT_DONE: &str = "knowledge/index-done";
/// 超过该文件数直接转后台，避免长时间挂住对话回合
const BACKGROUND_FILE_THRESHOLD: usize = 300;
/// 同步等待上限：超过即转后台（任务继续跑）
const SYNC_WAIT: Duration = Duration::from_secs(180);

/// 知识库共享状态：向量缓存 + 进行中的建库任务（按规范化工作目录索引）
pub struct KnowledgeState {
    pub cache: Arc<VectorCache>,
    pub jobs: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl Default for KnowledgeState {
    fn default() -> Self {
        Self {
            cache: Arc::new(VectorCache::default()),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn app_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录：{e}"))
}

/// 当前工作目录的知识库列表（扫 `knowledge/kbs/`，按更新时间倒序）
#[tauri::command]
pub async fn knowledge_list(app: AppHandle) -> Result<Vec<KbSummary>, String> {
    let dir = app_dir(&app)?;
    Ok(store::list_all(&dir))
}

/// 删除指定知识库文件（只删索引，不动原始文档与模型）
#[tauri::command]
pub async fn knowledge_delete(app: AppHandle, file: String) -> Result<(), String> {
    let dir = app_dir(&app)?;
    store::delete_file(&dir, &file)
}

/// 检索当前会话工作目录对应的知识库（向量 + 关键词混合）
#[tauri::command]
pub async fn knowledge_search(
    app: AppHandle,
    state: State<'_, KnowledgeState>,
    cwd: String,
    query: String,
    top_k: Option<usize>,
) -> Result<Vec<ChunkHit>, String> {
    let dir = app_dir(&app)?;
    let cache = state.cache.clone();
    let top = top_k.unwrap_or(search::DEFAULT_TOP_K);
    tauri::async_runtime::spawn_blocking(move || {
        let Some(store) = store::KbStore::open_existing(&dir, &cwd)? else {
            return Ok(Vec::new());
        };
        search::search(&dir, &store, &cache, &query, top)
    })
    .await
    .map_err(|e| format!("检索任务异常结束：{e}"))?
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
    let dir = app_dir(&app)?;
    let cwd = cwd.trim().to_string();
    if cwd.is_empty() {
        return Err("当前会话没有工作目录，无法确定知识库归属（请先选择会话目录）".into());
    }
    let roots = index::resolve_roots(&cwd, paths.as_deref().unwrap_or(&[]))?;
    let files = index::plan_files(&roots);
    let total = files.len();
    let root_labels: Vec<String> = roots.iter().map(|p| p.to_string_lossy().to_string()).collect();
    let key = super::paths::normalize_workspace(&cwd);
    if let Ok(jobs) = state.jobs.lock() {
        if jobs.contains_key(&key) {
            return Err("该工作目录已有一个建库任务在进行中".into());
        }
    }
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut jobs) = state.jobs.lock() {
        jobs.insert(key.clone(), cancel.clone());
    }
    let jobs = state.jobs.clone();
    let cwd_task = cwd.clone();
    let app_task = app.clone();
    let handle = tauri::async_runtime::spawn_blocking(move || {
        let emit_progress = |p: IndexProgress| {
            let _ = app_task.emit(EVENT_PROGRESS, p);
        };
        let mut on_progress = emit_progress;
        let result = index::run_index(
            &dir,
            &cwd_task,
            &roots,
            full.unwrap_or(false),
            &cancel,
            &mut on_progress,
        );
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

    if total > BACKGROUND_FILE_THRESHOLD {
        return Ok(IndexSummary::background_started(&cwd, root_labels, total));
    }
    match tokio::time::timeout(SYNC_WAIT, handle).await {
        Ok(Ok(Ok(summary))) => Ok(summary.finalized()),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(e)) => Err(format!("建库任务异常结束：{e}")),
        // 超时不打断任务：转后台继续，完成后仍会推送事件
        Err(_) => Ok(IndexSummary::background_started(&cwd, root_labels, total)),
    }
}

/// 取消进行中的建库任务
#[tauri::command]
pub async fn knowledge_index_cancel(
    state: State<'_, KnowledgeState>,
    cwd: String,
) -> Result<(), String> {
    let key = super::paths::normalize_workspace(&cwd);
    let flag = state
        .jobs
        .lock()
        .ok()
        .and_then(|map| map.get(&key).cloned());
    match flag {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            Ok(())
        }
        None => Err("该工作目录没有进行中的建库任务".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::knowledge::paths::normalize_workspace;

    #[test]
    fn default_state_starts_empty() {
        let state = KnowledgeState::default();
        assert!(state.cache.is_empty());
        assert!(state.jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn workspace_keys_are_normalized() {
        assert_eq!(
            normalize_workspace("D:\\Work\\售后\\"),
            normalize_workspace("d:/work/售后")
        );
    }
}
