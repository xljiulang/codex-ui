//! 知识库 Tauri 命令层：列表/删除/建库/取消/检索。
//!
//! 所有实际工作（抽取、切块、向量化、SQLite 混合检索）都在 `codexui-kb.exe` 里完成，
//! 数据目录与模型也由子进程自行解析；本层只做编排：先 `create <库名> <目录>` 登记来源，
//! 再 `index <库名>` 建库，并把 NDJSON 进度翻译成事件。
//!
//! 库名单一来源是调用方传入的 `kb`（缺省由前端按会话工作目录名派生）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, State};

use super::cli::{self, ChunkHit, IndexProgress, IndexSummary, JobHandle, KbSummary};

/// 进度事件名
pub const EVENT_PROGRESS: &str = "knowledge/index-progress";
/// 完成事件名（成功带 summary、失败带 error）
pub const EVENT_DONE: &str = "knowledge/index-done";
/// 超过该文件数直接转后台，避免长时间挂住调用方
const BACKGROUND_FILE_THRESHOLD: usize = 300;
/// 同步等待上限：超过即转后台（任务继续跑）
const SYNC_WAIT: Duration = Duration::from_secs(180);
/// 等待首个扫描进度（拿到文件总数）的上限；超时则按"未达阈值"继续同步等待
const SCAN_WAIT: Duration = Duration::from_secs(30);

/// 知识库共享状态：进行中的建库任务（按库名索引，不同库可并行）
#[derive(Default)]
pub struct KnowledgeState {
    pub jobs: Arc<Mutex<HashMap<String, Arc<JobHandle>>>>,
}

/// 任务状态键：规范化库名（去首尾空白、折叠内部空白、小写），与 CLI 同口径
fn kb_key(kb: &str) -> String {
    kb.trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// 各知识库列表（由子进程扫数据目录；损坏/旧版库标记为不可用但仍可删）
#[tauri::command]
pub async fn knowledge_list() -> Result<Vec<KbSummary>, String> {
    cli::list().await
}

/// 删除知识库：优先按库名；库名不可用（损坏/旧版库）时按库文件名兜底
#[tauri::command]
pub async fn knowledge_delete(kb: Option<String>, file: Option<String>) -> Result<(), String> {
    let kb = kb.unwrap_or_default();
    if !kb.trim().is_empty() {
        return cli::delete_kb(kb.trim()).await;
    }
    let file = file.unwrap_or_default();
    if file.trim().is_empty() {
        return Err("删除知识库需要库名或库文件名".to_string());
    }
    cli::delete_file(file.trim()).await
}

/// 检索指定库（库不存在返回空数组）
#[tauri::command]
pub async fn knowledge_search(
    kb: String,
    query: String,
    top_k: Option<usize>,
) -> Result<Vec<ChunkHit>, String> {
    let kb = kb.trim();
    if kb.is_empty() {
        return Err("缺少库名，无法确定检索哪个知识库".to_string());
    }
    let top = top_k.unwrap_or(cli::DEFAULT_TOP_K);
    cli::search(kb, &query, top).await
}

/// 建库/增量更新：先 create（幂等登记来源）再 index；文件数过多或超时转后台
#[tauri::command]
pub async fn knowledge_index_start(
    app: AppHandle,
    state: State<'_, KnowledgeState>,
    kb: String,
    source: String,
    full: Option<bool>,
) -> Result<IndexSummary, String> {
    let kb = kb.trim().to_string();
    let source = source.trim().to_string();
    if kb.is_empty() {
        return Err("当前会话没有可用库名，无法确定知识库归属".into());
    }
    if source.is_empty() {
        return Err("当前会话没有工作目录，无法确定索引来源（请先选择会话目录）".into());
    }
    let key = kb_key(&kb);
    if let Ok(jobs) = state.jobs.lock() {
        if jobs.contains_key(&key) {
            return Err(format!("知识库「{kb}」已有一个建库任务在进行中"));
        }
    }
    // 登记库与来源目录（幂等；同名不同来源由 CLI 报错并原样透出）
    cli::create(&kb, &source).await?;

    let job = Arc::new(JobHandle::default());
    if let Ok(mut jobs) = state.jobs.lock() {
        jobs.insert(key.clone(), job.clone());
    }
    let jobs = state.jobs.clone();
    let kb_task = kb.clone();
    let source_task = source.clone();
    let app_task = app.clone();
    let (total_tx, mut total_rx) = tokio::sync::watch::channel(0usize);
    let mut handle = tauri::async_runtime::spawn(async move {
        let mut on_progress = |p: IndexProgress| {
            // 扫描进度的 total 用于判断是否转后台
            let _ = total_tx.send(p.total);
            let _ = app_task.emit(EVENT_PROGRESS, &p);
        };
        let result = cli::run_index(&kb_task, full.unwrap_or(false), &job, &mut on_progress).await;
        let payload = match &result {
            Ok(summary) => serde_json::json!({ "kb": kb_task, "summary": summary }),
            Err(e) => serde_json::json!({ "kb": kb_task, "error": e }),
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
        return Ok(IndexSummary::background_started(&kb, &source_task, total));
    }
    match tokio::time::timeout(SYNC_WAIT, handle).await {
        Ok(Ok(summary)) => summary,
        Ok(Err(e)) => Err(format!("建库任务异常结束：{e}")),
        // 超时不打断任务：转后台继续，完成后仍会推送事件
        Err(_) => Ok(IndexSummary::background_started(&kb, &source_task, total)),
    }
}

/// 取消进行中的建库任务（终止子进程；SQLite WAL 保证库仍可用）
#[tauri::command]
pub async fn knowledge_index_cancel(
    state: State<'_, KnowledgeState>,
    kb: String,
) -> Result<(), String> {
    let key = kb_key(&kb);
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
        None => Err(format!("知识库「{kb}」没有进行中的建库任务")),
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
    fn kb_keys_are_normalized() {
        assert_eq!(kb_key("  售后   手册 "), kb_key("售后 手册"));
        assert_eq!(kb_key("ShouHou"), kb_key("shouhou"));
    }
}
