//! 定时任务：SQLite 持久化 + cron 调度 + 原会话自动执行。
//!
//! - 任务绑定已有会话（仅存 thread_id），到点后端直接 `thread/resume` + `turn/start`
//!   在原会话中发送任务 prompt（链路照抄微信桥 `run_turn_on_thread`）；
//! - 权限/模型/effort 执行时按会话 id 动态解析（sessions.json），缺省回退会话创建同款默认；
//! - 触发时目标线程有进行中回合 → 按任务级 busy_policy 顺延（defer）或跳过（skip）；
//! - 应用未运行期间错过的触发点记「已错过」，不补跑；
//! - 单次任务（cron 带年份）触发点已过且无下一次 → `done=1` 归档；
//! - 删除路径全部级联：删会话删任务、删任务删执行记录。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
use crate::codex::app_server::CodexServer;
use crate::codex::notifications;
use crate::codex::session_state::SessionStateStore;
use crate::codex::settings;
use crate::codex::util::cached_default_model;
use chrono::{DateTime, Local, TimeZone};
use cron::Schedule as CronSchedule;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

/// 前端事件名：任务每次变更全量推送快照。
pub const SCHEDULED_TASKS_EVENT: &str = "scheduled-tasks/event";

/// 调度 tick 间隔（秒）。
const TICK_SECS: u64 = 20;
/// 单个任务回合约最长等待（秒）；超时按失败落记录，避免任务永久停留在进行中。
const TURN_TIMEOUT_SECS: u64 = 1800;
/// cron 相邻两次触发的最小间隔（秒）：一次 agent 回合通常耗时数十秒以上，
/// 更小的颗粒度没有实际意义（配合顺延/跳过机制也不会堆积），创建时强制拒绝。
pub const MIN_INTERVAL_SECS: i64 = 60;
/// 执行结果摘要截断长度（按字符计，UTF-8 安全）。
const RESULT_MAX_CHARS: usize = 4000;

/// 忙碌策略：顺延（等待会话空闲后执行）。
pub const BUSY_POLICY_DEFER: &str = "defer";
/// 忙碌策略：跳过本次（等下一个触发点）。
pub const BUSY_POLICY_SKIP: &str = "skip";

/// 归一化忙碌策略：非法值回退 skip（忙时跳过，创建默认值）。
pub fn normalize_busy_policy(v: &str) -> &'static str {
    if v.eq_ignore_ascii_case(BUSY_POLICY_DEFER) {
        BUSY_POLICY_DEFER
    } else {
        BUSY_POLICY_SKIP
    }
}

// ---------------------------------------------------------------------------
// 纯函数（单元测试覆盖）
// ---------------------------------------------------------------------------

/// 解析 cron 表达式（6 字段「秒 分 时 日 月 周」或 7 字段末尾年份）。
fn parse_cron(expr: &str) -> Option<CronSchedule> {
    CronSchedule::from_str(expr.trim()).ok()
}

/// 计算下一次触发时刻（本地时区语义，时间戳与时区无关）。
/// 解析失败或无后续触发（如单次任务年份已过）返回 None。
pub fn next_run_at<TZ: TimeZone>(expr: &str, from: &DateTime<TZ>) -> Option<i64>
where
    TZ::Offset: Copy,
{
    let sched = parse_cron(expr)?;
    sched.after(from).next().map(|dt| dt.timestamp())
}

/// 表达式是否合法（解析通过即合法；颗粒度由 validate_cron 另行校验）。
pub fn is_valid_cron(expr: &str) -> bool {
    parse_cron(expr).is_some()
}

/// 是否为单次表达式（7 字段带年份：触发点唯一）。
pub fn is_once_cron(expr: &str) -> bool {
    expr.trim().split_whitespace().count() >= 7
}

/// 创建校验：表达式合法且相邻两次触发间隔不小于 [`MIN_INTERVAL_SECS`]。
/// 间隔取同一迭代器的连续两次触发之差，天然覆盖每秒/每 N 秒类表达式；
/// 仅有一个触发点（单次表达式）不存在重复触发风险，直接放行。
pub fn validate_cron(expr: &str) -> Result<(), String> {
    let sched = parse_cron(expr).ok_or_else(|| {
        format!("无效的 cron 表达式：{expr}（应为 6 字段「秒 分 时 日 月 周」或 7 字段末尾年份）")
    })?;
    let mut it = sched.after(&Local::now());
    let first = it
        .next()
        .ok_or_else(|| "该表达式此后不再触发（单次时间已过）".to_string())?;
    if let Some(second) = it.next() {
        if second.timestamp() - first.timestamp() < MIN_INTERVAL_SECS {
            return Err(format!(
                "定时颗粒度过小，最小间隔为 {} 分钟（建议不低于 5 分钟）",
                MIN_INTERVAL_SECS / 60
            ));
        }
    }
    Ok(())
}

/// 复刻前端 lib/permissions.ts 的 toApprovalPolicy：无人值守回合的审批策略。
pub fn to_approval_policy(mode: &str) -> &'static str {
    match mode {
        "ask-for-approval" | "help-me-approve" => "on-request",
        _ => "never",
    }
}

/// 复刻前端 lib/permissions.ts 的 toSandbox 语义 → turn/start 的 sandboxPolicy.type
/// （camelCase，形态与微信桥 build_turn_params 一致）。
pub fn to_sandbox_policy(mode: &str) -> &'static str {
    match mode {
        "read-only" => "readOnly",
        "ask-for-approval" | "help-me-approve" => "workspaceWrite",
        _ => "dangerFullAccess",
    }
}

/// 从 `thread/turns/list(itemsView=full)` 响应里取指定回合的最后一条非空
/// `agentMessage` 文本（与微信桥 extract_turn_agent_text 同款逻辑）。
pub fn extract_result_text(resp: &Value, turn_id: &str) -> Option<String> {
    let turns = resp.pointer("/data").and_then(|d| d.as_array())?;
    let turn = turns
        .iter()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(turn_id))?;
    let items = turn.get("items")?.as_array()?;
    for item in items.iter().rev() {
        if item.get("type").and_then(|v| v.as_str()) == Some("agentMessage") {
            let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// 超长文本截断（UTF-8 安全按字符计），尾部追加省略号。
pub fn truncate_chars(text: &str, max: usize) -> String {
    if max == 0 || text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// 拼装任务终态通知的标题与正文（纯函数，便于单测）：成功带耗时与结果摘要，失败带错误。
pub fn task_finish_notice(
    name: &str,
    status: &str,
    duration_ms: Option<i64>,
    result: Option<&str>,
    error: Option<&str>,
) -> (String, String) {
    if status == "success" {
        let dur = duration_ms
            .map(|ms| format!("{} 秒", ms / 1000))
            .unwrap_or_else(|| "—".to_string());
        let mut body = format!("「{}」执行完成，耗时 {}", name, dur);
        if let Some(r) = result {
            let r = truncate_chars(r, 60);
            if !r.is_empty() {
                body.push_str(&format!("\n{r}"));
            }
        }
        ("定时任务执行完成".to_string(), body)
    } else {
        let msg = error.unwrap_or("未知错误");
        (
            "定时任务执行失败".to_string(),
            format!("「{}」执行失败：{}", name, msg),
        )
    }
}

/// 单次触发决策：线程空闲即执行；忙时按任务策略顺延或跳过。
#[derive(Debug, PartialEq, Eq)]
pub enum FireAction {
    Execute,
    Defer,
    Skip,
}

/// 触发决策：线程空闲即执行；忙时按任务策略顺延或跳过。
pub fn decide_fire(thread_busy: bool, policy: &str) -> FireAction {
    if !thread_busy {
        return FireAction::Execute;
    }
    if normalize_busy_policy(policy) == BUSY_POLICY_SKIP {
        FireAction::Skip
    } else {
        FireAction::Defer
    }
}

/// 启动首轮对单个任务的处置结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstTickAction {
    /// 已过期且启用：记「已错过」并推进 next_run（错过不补跑）。
    MissAndAdvance,
    /// 未到期 / 停用 / 已归档，无需处理。
    Idle,
}

/// 启动首轮处置决策：应用未运行期间错过的触发点一律记「已错过」不补跑。
/// 此刻线程必为空闲，若复用 [`decide_fire`] 会误判为 `Execute` 立即执行，
/// 故首轮对到期任务固定返回 [`FirstTickAction::MissAndAdvance`]，
/// 与任务忙时策略（skip/defer）无关。
pub fn decide_first_tick(now: i64, task: &ScheduledTask) -> FirstTickAction {
    let due = task.next_run.map(|nr| nr <= now).unwrap_or(false);
    if task.enabled && !task.done && due {
        FirstTickAction::MissAndAdvance
    } else {
        FirstTickAction::Idle
    }
}

/// 「thread not found」判定：codex 删除线程后 resume/turn 的稳定错误文案。
fn is_thread_not_found(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("thread not found") || e.contains("thread does not exist")
}

// ---------------------------------------------------------------------------
// 数据结构与存储
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub cron: String,
    pub thread_id: String,
    /// defer（顺延，默认）| skip（跳过），任务级。
    pub busy_policy: String,
    pub enabled: bool,
    /// 单次任务终态标记（触发点已过且无下一次）。
    pub done: bool,
    pub created_at: i64,
    pub next_run: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunRecord {
    pub id: i64,
    pub task_id: String,
    pub started_at: i64,
    /// running | success | failed | skipped | missed
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS scheduled_tasks (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, prompt TEXT NOT NULL,
  cron TEXT NOT NULL, thread_id TEXT NOT NULL,
  busy_policy TEXT NOT NULL DEFAULT 'skip',
  enabled INTEGER NOT NULL DEFAULT 1,
  done INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, next_run INTEGER);
CREATE TABLE IF NOT EXISTS task_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT NOT NULL,
  started_at INTEGER NOT NULL, status TEXT NOT NULL,
  duration_ms INTEGER, turn_id TEXT, result TEXT, error TEXT);
CREATE INDEX IF NOT EXISTS idx_task_runs ON task_runs(task_id, started_at DESC);
";

fn scheduled_db_path(app_dir: &Path) -> PathBuf {
    app_dir.join("codexui.sqlite")
}

fn next_task_id(counter: &AtomicU64) -> String {
    format!(
        "task-{}-{}",
        Local::now().timestamp_millis(),
        counter.fetch_add(1, Ordering::Relaxed)
    )
}

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<ScheduledTask> {
    Ok(ScheduledTask {
        id: row.get("id")?,
        name: row.get("name")?,
        prompt: row.get("prompt")?,
        cron: row.get("cron")?,
        thread_id: row.get("thread_id")?,
        busy_policy: row.get("busy_policy")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        done: row.get::<_, i64>("done")? != 0,
        created_at: row.get("created_at")?,
        next_run: row.get("next_run")?,
    })
}

fn row_to_run(row: &rusqlite::Row) -> rusqlite::Result<TaskRunRecord> {
    Ok(TaskRunRecord {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        started_at: row.get("started_at")?,
        status: row.get("status")?,
        duration_ms: row.get("duration_ms")?,
        turn_id: row.get("turn_id")?,
        result: row.get("result")?,
        error: row.get("error")?,
    })
}

/// 定时任务存储：单连接 + Mutex 串行访问（低频小数据，无需连接池）。
pub struct ScheduledTaskStore {
    conn: Mutex<Connection>,
    counter: AtomicU64,
}

impl ScheduledTaskStore {
    /// 打开（不存在则创建）应用自有数据库并幂等建表；随后清理僵尸 running 记录。
    pub fn open(app_dir: &Path) -> Result<Self, String> {
        let conn = Connection::open(scheduled_db_path(app_dir))
            .map_err(|e| format!("打开定时任务库失败：{e}"))?;
        conn.busy_timeout(Duration::from_secs(3))
            .map_err(|e| format!("设置定时任务库忙等待失败：{e}"))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("设置 WAL 失败：{e}"))?;
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| format!("初始化定时任务表失败：{e}"))?;
        // 上次运行中应用退出/崩溃遗留的 running 记录收敛为失败，避免永久悬挂。
        conn.execute(
            "UPDATE task_runs SET status='failed', error='应用中断' WHERE status='running'",
            [],
        )
        .map_err(|e| format!("清理遗留执行记录失败：{e}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
            counter: AtomicU64::new(1),
        })
    }

    pub fn list(&self) -> Vec<ScheduledTask> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = match conn.prepare("SELECT * FROM scheduled_tasks ORDER BY created_at, id") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], row_to_task)
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<ScheduledTask> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT * FROM scheduled_tasks WHERE id = ?1",
            [id],
            row_to_task,
        )
        .ok()
    }

    /// 创建任务：校验 cron（合法 + 最小颗粒度），归一化忙碌策略，计算初始 next_run。
    pub fn add(
        &self,
        name: &str,
        prompt: &str,
        cron: &str,
        thread_id: &str,
        busy_policy: &str,
    ) -> Result<ScheduledTask, String> {
        let name = name.trim();
        let prompt = prompt.trim();
        if name.is_empty() || prompt.is_empty() || thread_id.trim().is_empty() {
            return Err("任务名、prompt 与绑定会话不能为空".to_string());
        }
        validate_cron(cron)?;
        let policy = normalize_busy_policy(busy_policy);
        let now = Local::now().timestamp();
        let task = ScheduledTask {
            id: next_task_id(&self.counter),
            name: name.to_string(),
            prompt: prompt.to_string(),
            cron: cron.trim().to_string(),
            thread_id: thread_id.trim().to_string(),
            busy_policy: policy.to_string(),
            enabled: true,
            done: false,
            created_at: now,
            next_run: next_run_at(cron, &Local::now()),
        };
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO scheduled_tasks (id, name, prompt, cron, thread_id, busy_policy, enabled, done, created_at, next_run)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, ?7, ?8)",
            rusqlite::params![
                task.id,
                task.name,
                task.prompt,
                task.cron,
                task.thread_id,
                task.busy_policy,
                task.created_at,
                task.next_run,
            ],
        )
        .map_err(|e| format!("写入定时任务失败：{e}"))?;
        Ok(task)
    }

    /// 删除单个任务（事务内先删其全部执行记录，再删任务）。返回任务是否存在过。
    pub fn remove(&self, id: &str) -> Result<bool, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM task_runs WHERE task_id = ?1", [id])
            .map_err(|e| e.to_string())?;
        let n = tx
            .execute("DELETE FROM scheduled_tasks WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    /// 删除绑定某线程的全部任务及其执行记录（会话删除级联）。返回被删任务 id 列表。
    pub fn remove_by_thread(&self, thread_id: &str) -> Result<Vec<String>, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let ids: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT id FROM scheduled_tasks WHERE thread_id = ?1")
                .map_err(|e| e.to_string())?;
            stmt.query_map([thread_id], |r| r.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .map_err(|e| e.to_string())?
        };
        tx.execute(
            "DELETE FROM task_runs WHERE task_id IN (SELECT id FROM scheduled_tasks WHERE thread_id = ?1)",
            [thread_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM scheduled_tasks WHERE thread_id = ?1",
            [thread_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(ids)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE scheduled_tasks SET enabled = ?1 WHERE id = ?2",
            rusqlite::params![enabled as i64, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_busy_policy(&self, id: &str, policy: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE scheduled_tasks SET busy_policy = ?1 WHERE id = ?2",
            rusqlite::params![normalize_busy_policy(policy), id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 编辑任务：更新标题、提示词与忙时策略（cron/绑定会话/启用/完成态不变）。
    /// 返回更新后的完整任务；任务不存在返回错误。
    pub fn update(
        &self,
        id: &str,
        name: &str,
        prompt: &str,
        busy_policy: &str,
    ) -> Result<ScheduledTask, String> {
        let name = name.trim();
        let prompt = prompt.trim();
        if name.is_empty() || prompt.is_empty() {
            return Err("任务名与 prompt 不能为空".to_string());
        }
        let policy = normalize_busy_policy(busy_policy);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n = conn
            .execute(
                "UPDATE scheduled_tasks SET name = ?1, prompt = ?2, busy_policy = ?3 WHERE id = ?4",
                rusqlite::params![name, prompt, policy, id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("定时任务不存在".to_string());
        }
        conn.query_row(
            "SELECT * FROM scheduled_tasks WHERE id = ?1",
            [id],
            row_to_task,
        )
        .map_err(|e| e.to_string())
    }

    pub fn set_next_run(&self, id: &str, next_run: Option<i64>) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE scheduled_tasks SET next_run = ?1 WHERE id = ?2",
            rusqlite::params![next_run, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 单次任务终态：置 done 并停用。
    pub fn mark_done(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE scheduled_tasks SET done = 1, enabled = 0 WHERE id = ?1",
            [id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 任务存在时新增一条执行记录（running），返回记录行 id；任务已被删除则返回 None。
    pub fn start_run(
        &self,
        task_id: &str,
        started_at: i64,
        turn_id: Option<&str>,
    ) -> Result<Option<i64>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if conn
            .query_row(
                "SELECT 1 FROM scheduled_tasks WHERE id = ?1",
                [task_id],
                |_| Ok(()),
            )
            .is_err()
        {
            return Ok(None);
        }
        conn.execute(
            "INSERT INTO task_runs (task_id, started_at, status, turn_id) VALUES (?1, ?2, 'running', ?3)",
            rusqlite::params![task_id, started_at, turn_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(Some(conn.last_insert_rowid()))
    }

    /// 收敛一条执行记录的终态（记录所在任务已删除时更新 0 行，无害）。
    pub fn finish_run(
        &self,
        run_id: i64,
        status: &str,
        duration_ms: Option<i64>,
        result: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE task_runs SET status = ?1, duration_ms = ?2, result = ?3, error = ?4 WHERE id = ?5",
            rusqlite::params![status, duration_ms, result, error, run_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 无回合的记录（已跳过 / 已错过），一步插入终态；任务已删除则跳过。
    pub fn record_event_run(
        &self,
        task_id: &str,
        started_at: i64,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if conn
            .query_row(
                "SELECT 1 FROM scheduled_tasks WHERE id = ?1",
                [task_id],
                |_| Ok(()),
            )
            .is_err()
        {
            return Ok(());
        }
        conn.execute(
            "INSERT INTO task_runs (task_id, started_at, status, error) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![task_id, started_at, status, error],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 按任务倒序分页查询执行记录。
    pub fn list_runs(
        &self,
        task_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskRunRecord>, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare(
                "SELECT * FROM task_runs WHERE task_id = ?1 ORDER BY started_at DESC, id DESC LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![task_id, limit, offset], row_to_run)
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

// ---------------------------------------------------------------------------
// 调度器
// ---------------------------------------------------------------------------

struct RunningTurn {
    thread_id: String,
    turn_id: String,
    run_id: i64,
    started_at: i64,
    deadline: Instant,
}

#[derive(Default)]
struct SchedulerInner {
    /// 有进行中回合的线程集合（通知驱动 + 执行前预占，双重防并发）。
    active_threads: HashSet<String>,
    /// 因会话忙而顺延中的任务 id。
    deferred: HashSet<String>,
    /// 任务 id → 进行中回合（用于完成匹配与超时兜底）。
    running: HashMap<String, RunningTurn>,
    /// 首轮 tick 标记：把「应用未运行期间已过期的触发点」判定为「已错过」。
    /// 默认 `false`（`#[derive(Default)]`），由 [`TaskScheduler::start`] 在启动
    /// 第一个 tick 前同步置为 `true`；tick 首轮消费后随即复位，仅生效一次。
    first_tick: bool,
    /// 默认模型缓存：None 未解析过；成功结果永久命中，失败结果按
    /// `util::DEFAULT_MODEL_FAILURE_TTL_SECS` 过期重试（避免瞬时故障被永久固化）。
    default_model: Option<(Result<String, String>, Instant)>,
}

/// 定时任务调度器：tick 循环 + 通知订阅泵 + 回合执行。
pub struct TaskScheduler {
    app: AppHandle,
    server: Arc<CodexServer>,
    store: Arc<ScheduledTaskStore>,
    session_store: Arc<SessionStateStore>,
    app_dir: PathBuf,
    inner: Mutex<SchedulerInner>,
}

impl TaskScheduler {
    pub fn new(
        app: AppHandle,
        server: Arc<CodexServer>,
        store: Arc<ScheduledTaskStore>,
        session_store: Arc<SessionStateStore>,
        app_dir: PathBuf,
    ) -> Self {
        Self {
            app,
            server,
            store,
            session_store,
            app_dir,
            inner: Mutex::new(SchedulerInner::default()),
        }
    }

    /// 启动调度循环与通知订阅（仅在 setup 调用一次）。
    pub fn start(self: &Arc<Self>) {
        // 启动首轮标记：必须在 spawn tick 循环前同步置位（同一把 inner 锁，
        // 与 tick 消费 `mem::replace(.., false)` 互斥），保证首个 tick 生效一次。
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .first_tick = true;
        let tick = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
                tick.tick().await;
            }
        });
        let pump = self.clone();
        tauri::async_runtime::spawn(async move { pump.notification_pump().await });
    }

    /// 快照推送到前端（tasks 全量 + 本次变更的任务 id，供前端定向刷新展开的记录）。
    fn emit_tasks(&self, changed_task_id: Option<&str>) {
        let _ = self.app.emit(
            SCHEDULED_TASKS_EVENT,
            json!({
                "tasks": self.store.list(),
                "changedTaskId": changed_task_id,
            }),
        );
    }

    /// 供命令层在变更后主动推送快照。
    pub fn notify_changed(&self, changed_task_id: Option<&str>) {
        self.emit_tasks(changed_task_id);
    }

    /// 发送 Windows 原生通知（系统 toast/操作中心，非托盘 tooltip）。
    /// 实现在共用模块 [`notifications`]：在带消息泵的主线程显示（保证 `Activated`
    /// 事件可被投递），并附「打开会话」按钮；点击按钮携带 thread_id，经 Tauri 事件
    /// 通知前端聚焦窗口并打开绑定会话。
    fn notify_task(&self, title: &str, body: &str, thread_id: &str) {
        let app = self.app.clone();
        let server = self.server.clone();
        let title = title.to_string();
        let body = body.to_string();
        let thread_id = thread_id.to_string();
        #[cfg(windows)]
        {
            let _ = app.clone().run_on_main_thread(move || {
                let _ = notifications::show_winrt_toast(
                    &app,
                    &server,
                    notifications::notification_app_id(),
                    &title,
                    &body,
                    &thread_id,
                    "scheduled-task",
                );
            });
        }
        #[cfg(not(windows))]
        {
            let _ = (app, server, title, body, thread_id);
        }
    }

    /// 任务执行终态通知（成功/失败）；未命中任务（已被删除）则不发。
    fn notify_finished(
        &self,
        task_id: &str,
        status: &str,
        duration_ms: Option<i64>,
        result: Option<&str>,
        error: Option<&str>,
    ) {
        let Some(task) = self.store.get(task_id) else {
            return;
        };
        let (title, body) = task_finish_notice(&task.name, status, duration_ms, result, error);
        self.notify_task(&title, &body, &task.thread_id);
    }

    /// 默认模型解析（进程内缓存）：isDefault 优先，其次首个非 hidden（微信桥同款）。
    /// 命中判定见 [`cached_default_model`]：成功永久缓存，失败仅缓存 60 秒。
    async fn resolve_default_model(&self) -> Result<String, String> {
        let cached = {
            let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            cached_default_model(&g.default_model, Instant::now())
        };
        if let Some(res) = cached {
            return res;
        }
        let res = async {
            let resp = self
                .server
                .request("model/list", json!({}), Some(Duration::from_secs(30)))
                .await?;
            let list = resp
                .get("data")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default();
            let pick = list.iter().find(|m| {
                m.get("isDefault")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false)
                    && !m.get("hidden").and_then(|x| x.as_bool()).unwrap_or(false)
            });
            let pick = pick.or_else(|| {
                list.iter()
                    .find(|m| !m.get("hidden").and_then(|x| x.as_bool()).unwrap_or(true))
            });
            pick.and_then(|m| m.get("model").and_then(|x| x.as_str()))
                .map(str::to_string)
                .ok_or_else(|| "模型列表为空".to_string())
        }
        .await;
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .default_model = Some((res.clone(), Instant::now()));
        res
    }

    /// 调度 tick：错过判定 → 顺延/跳过/执行 → 超时兜底 → 内存清理。
    async fn tick(self: &Arc<Self>) {
        let now = Local::now().timestamp();
        let tasks = self.store.list();

        let first_tick = {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut g.first_tick, false)
        };

        // 内存 reconcile：deferred 只保留启用且仍在顺延语义内（到期未跑）的任务
        {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.deferred.retain(|id| {
                tasks.iter().any(|t| {
                    &t.id == id && t.enabled && t.next_run.map(|nr| nr <= now).unwrap_or(false)
                })
            });
        }

        for task in tasks.iter().filter(|t| t.enabled && !t.done) {
            // 启动首轮：应用未运行期间错过的触发点一律记「已错过」，不补跑。
            // 决策不依赖线程忙闲（此刻必空闲）与任务忙时策略，到期即
            // MissAndAdvance，未到期回退到下方正常调度路径。
            if first_tick && decide_first_tick(now, task) == FirstTickAction::MissAndAdvance {
                let _ = self.store.record_event_run(&task.id, now, "missed", None);
                self.advance(task, now);
                self.emit_tasks(Some(&task.id));
                continue;
            }

            let due = task.next_run.map(|nr| nr <= now).unwrap_or(false);
            if !due {
                continue;
            }
            let busy = self
                .inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .active_threads
                .contains(&task.thread_id);

            match decide_fire(busy, &task.busy_policy) {
                FireAction::Execute => {
                    {
                        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                        g.deferred.remove(&task.id);
                        // 执行前预占线程：同 tick 多任务同线程不会并发回合
                        g.active_threads.insert(task.thread_id.clone());
                    }
                    self.advance(task, now);
                    self.emit_tasks(Some(&task.id));
                    let sched = self.clone();
                    let task = task.clone();
                    tauri::async_runtime::spawn(async move { sched.execute(task).await });
                }
                FireAction::Defer => {
                    let newly = self
                        .inner
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .deferred
                        .insert(task.id.clone());
                    if newly {
                        self.emit_tasks(Some(&task.id));
                    }
                }
                FireAction::Skip => {
                    let _ = self.store.record_event_run(
                        &task.id,
                        now,
                        "skipped",
                        Some("会话正忙，按任务策略跳过"),
                    );
                    self.advance(task, now);
                    self.emit_tasks(Some(&task.id));
                }
            }
        }

        // 超时兜底：进行中回合超过 TURN_TIMEOUT_SECS 记失败并停止等待；
        // 线程保留在 active 集合中（回合可能仍在服务端跑，完成通知到达后自然释放）。
        let expired: Vec<(String, i64)> = {
            let now_inst = Instant::now();
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let expired: Vec<(String, i64)> = g
                .running
                .iter()
                .filter(|(_, r)| now_inst > r.deadline)
                .map(|(id, r)| (id.clone(), r.run_id))
                .collect();
            for (id, _) in &expired {
                g.running.remove(id);
            }
            expired
        };
        for (task_id, run_id) in expired {
            let _ =
                self.store
                    .finish_run(run_id, "failed", None, None, Some("执行超时（30 分钟）"));
            self.notify_finished(&task_id, "failed", None, None, Some("执行超时（30 分钟）"));
            self.emit_tasks(Some(&task_id));
        }
    }

    /// 推进 next_run 到 now 之后的下一个触发点；单次任务（无下一次）置 done 并停用。
    fn advance(&self, task: &ScheduledTask, now: i64) {
        let next = Local
            .timestamp_opt(now, 0)
            .single()
            .and_then(|from| next_run_at(&task.cron, &from));
        let _ = self.store.set_next_run(&task.id, next);
        if next.is_none() {
            let _ = self.store.mark_done(&task.id);
        }
    }

    /// 执行任务：解析权限/模型 → resume → turn/start → 落 running，完成由通知泵收敛。
    async fn execute(&self, task: ScheduledTask) {
        let started_at = Local::now().timestamp();
        self.notify_task(
            "定时任务开始执行",
            &format!("「{}」已开始执行", task.name),
            &task.thread_id,
        );
        let session = self.session_store.get(&task.thread_id);
        let permission = session
            .as_ref()
            .and_then(|s| s.permission_mode.clone())
            .unwrap_or_else(|| settings::load(&self.app_dir).default_permission);
        let effort = session.as_ref().and_then(|s| s.effort.clone());

        let model = match session.as_ref().and_then(|s| s.model.clone()) {
            Some(m) => m,
            None => match self.resolve_default_model().await {
                Ok(m) => m,
                Err(e) => {
                    self.release_thread(&task.thread_id);
                    if let Some(run_id) = self
                        .store
                        .start_run(&task.id, started_at, None)
                        .ok()
                        .flatten()
                    {
                        let _ = self.store.finish_run(
                            run_id,
                            "failed",
                            None,
                            None,
                            Some(&format!("无法解析默认模型：{e}")),
                        );
                        self.notify_finished(
                            &task.id,
                            "failed",
                            None,
                            None,
                            Some(&format!("无法解析默认模型：{e}")),
                        );
                    }
                    self.emit_tasks(Some(&task.id));
                    return;
                }
            },
        };

        // 先 resume：codex 的 turn/start 只对已 resume 的线程可寻址（微信桥同款注释）。
        if let Err(e) = self
            .server
            .request(
                "thread/resume",
                json!({ "threadId": task.thread_id }),
                Some(Duration::from_secs(30)),
            )
            .await
        {
            self.release_thread(&task.thread_id);
            let not_found = is_thread_not_found(&e);
            if not_found {
                let _ = self.store.set_enabled(&task.id, false);
                if is_once_cron(&task.cron) {
                    let _ = self.store.mark_done(&task.id);
                }
            }
            if let Some(run_id) = self
                .store
                .start_run(&task.id, started_at, None)
                .ok()
                .flatten()
            {
                let err_text = if not_found {
                    "绑定的会话已删除".to_string()
                } else {
                    format!("恢复会话失败：{e}")
                };
                let _ = self
                    .store
                    .finish_run(run_id, "failed", None, None, Some(&err_text));
                self.notify_finished(&task.id, "failed", None, None, Some(&err_text));
            }
            self.emit_tasks(Some(&task.id));
            return;
        }

        let params = json!({
            "threadId": task.thread_id,
            "input": [{ "type": "text", "text": task.prompt }],
            "clientUserMessageId": format!("sched-{}-{}", task.id, started_at),
            "approvalPolicy": to_approval_policy(&permission),
            "sandboxPolicy": { "type": to_sandbox_policy(&permission) },
            "model": model,
            "effort": effort,
            "collaborationMode": {
                "mode": "default",
                "settings": { "model": model, "reasoning_effort": effort, "developer_instructions": null },
            },
        });
        match self
            .server
            .request("turn/start", params, Some(Duration::from_secs(60)))
            .await
        {
            Ok(resp) => {
                let turn_id = resp
                    .pointer("/turn/id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let run_id = self
                    .store
                    .start_run(&task.id, started_at, Some(&turn_id))
                    .ok()
                    .flatten();
                if let Some(run_id) = run_id {
                    self.inner
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .running
                        .insert(
                            task.id.clone(),
                            RunningTurn {
                                thread_id: task.thread_id.clone(),
                                turn_id: turn_id.clone(),
                                run_id,
                                started_at,
                                deadline: Instant::now() + Duration::from_secs(TURN_TIMEOUT_SECS),
                            },
                        );
                } else {
                    // 任务在执行启动期间被删除：尽力中断，避免无人看管的回合继续跑。
                    let _ = self
                        .server
                        .request(
                            "turn/interrupt",
                            json!({ "threadId": task.thread_id, "turnId": turn_id }),
                            None,
                        )
                        .await;
                }
                self.emit_tasks(Some(&task.id));
            }
            Err(e) => {
                self.release_thread(&task.thread_id);
                if let Some(run_id) = self
                    .store
                    .start_run(&task.id, started_at, None)
                    .ok()
                    .flatten()
                {
                    let _ = self.store.finish_run(
                        run_id,
                        "failed",
                        None,
                        None,
                        Some(&format!("启动回合失败：{e}")),
                    );
                    self.notify_finished(
                        &task.id,
                        "failed",
                        None,
                        None,
                        Some(&format!("启动回合失败：{e}")),
                    );
                }
                self.emit_tasks(Some(&task.id));
            }
        }
    }

    fn release_thread(&self, thread_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active_threads
            .remove(thread_id);
    }

    /// 通知订阅泵：维护进行中线程集合，收敛定时任务的回合终态与执行结果。
    async fn notification_pump(&self) {
        let mut rx = self.server.subscribe_notifications();
        loop {
            match rx.recv().await {
                Ok((method, params)) => self.on_notification(&method, &params).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    }

    async fn on_notification(&self, method: &str, params: &Value) {
        let thread_id = params
            .get("threadId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match method {
            "turn/started" => {
                if !thread_id.is_empty() {
                    self.inner
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .active_threads
                        .insert(thread_id.to_string());
                }
            }
            "turn/completed" => {
                if !thread_id.is_empty() {
                    self.inner
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .active_threads
                        .remove(thread_id);
                }
                let turn_id = params
                    .pointer("/turn/id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let status = params
                    .pointer("/turn/status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                // 匹配本调度器发起的回合：线程必须一致，turn id 双方非空时精确匹配
                let matched = {
                    let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                    g.running.iter().find_map(|(id, r)| {
                        (r.thread_id == thread_id
                            && (turn_id.is_empty() || r.turn_id.is_empty() || r.turn_id == turn_id))
                            .then(|| id.clone())
                    })
                };
                let Some(task_id) = matched else { return };
                let Some(run) = self
                    .inner
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .running
                    .remove(&task_id)
                else {
                    return;
                };
                let duration_ms = (Local::now().timestamp() - run.started_at).max(0) * 1000;
                // completed → 成功；failed / interrupted（含用户手动中断）→ 失败
                let (status, error) = if status == "completed" || status.is_empty() {
                    ("success", None)
                } else {
                    ("failed", Some(format!("回合结束状态：{status}")))
                };
                let result = if status == "success" {
                    self.fetch_result(thread_id, &run.turn_id)
                        .await
                        .map(|t| truncate_chars(&t, RESULT_MAX_CHARS))
                } else {
                    None
                };
                let _ = self.store.finish_run(
                    run.run_id,
                    status,
                    Some(duration_ms),
                    result.as_deref(),
                    error.as_deref(),
                );
                self.notify_finished(
                    &task_id,
                    status,
                    Some(duration_ms),
                    result.as_deref(),
                    error.as_deref(),
                );
                self.emit_tasks(Some(&task_id));
            }
            _ => {}
        }
    }

    /// 回查执行结果：取该回合最后一条非空 agentMessage（best-effort，失败返回 None）。
    async fn fetch_result(&self, thread_id: &str, turn_id: &str) -> Option<String> {
        for attempt in 0..2 {
            let resp = self
                .server
                .request(
                    "thread/turns/list",
                    json!({
                        "threadId": thread_id,
                        "limit": 5,
                        "sortDirection": "desc",
                        "itemsView": "full",
                    }),
                    Some(Duration::from_secs(5)),
                )
                .await
                .ok()?;
            if let Some(text) = extract_result_text(&resp, turn_id) {
                return Some(text);
            }
            if attempt == 0 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        None
    }

    /// 删除任务后清理调度器内存状态。进行中回合真实存在（通知完成后释放），
    /// 因此仅摘除跟踪记录，不从 active_threads 移除线程。
    pub fn purge_task(&self, task_id: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.deferred.remove(task_id);
        g.running.remove(task_id);
    }

    /// 立即执行（不判断忙碌）；同任务已在进行中则拒绝。
    pub fn run_now(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let task = self.store.get(id).ok_or_else(|| "任务不存在".to_string())?;
        let busy_self = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running
            .contains_key(id);
        if busy_self {
            return Err("任务正在执行".to_string());
        }
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active_threads
            .insert(task.thread_id.clone());
        let sched = self.clone();
        tauri::async_runtime::spawn(async move { sched.execute(task).await });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn tz() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    #[test]
    fn next_run_at_daily_and_interval() {
        // 2026-01-15 10:00:00 → 每天 9 点的下一次是次日 9 点
        let from = tz().with_ymd_and_hms(2026, 1, 15, 10, 0, 0).unwrap();
        let next = next_run_at("0 0 9 * * *", &from).unwrap();
        let expected = tz().with_ymd_and_hms(2026, 1, 16, 9, 0, 0).unwrap();
        assert_eq!(next, expected.timestamp());

        // 每 30 分钟：10:00 → 10:30
        let next = next_run_at("0 */30 * * * *", &from).unwrap();
        let expected = tz().with_ymd_and_hms(2026, 1, 15, 10, 30, 0).unwrap();
        assert_eq!(next, expected.timestamp());
    }

    #[test]
    fn next_run_at_once_and_invalid() {
        // 单次（带年份）：未来时间可触发
        let from = tz().with_ymd_and_hms(2026, 1, 15, 10, 0, 0).unwrap();
        assert!(next_run_at("0 0 9 20 1 * 2026", &from).is_some());
        // 年份已过 → 无下一次
        assert!(next_run_at("0 0 9 20 1 * 2025", &from).is_none());
        // 非法表达式
        assert!(next_run_at("not-a-cron", &from).is_none());
    }

    #[test]
    fn validate_cron_rejects_sub_minute_granularity() {
        // 每秒 / 每 30 秒 → 拒绝
        assert!(validate_cron("* * * * * *").is_err());
        assert!(validate_cron("*/30 * * * * *").is_err());
        assert!(validate_cron("0/15 * * * * *").is_err());
        // 每分钟 / 每 5 分钟 / 每天 → 通过
        assert!(validate_cron("0 * * * * *").is_ok());
        assert!(validate_cron("0 */5 * * * *").is_ok());
        assert!(validate_cron("0 0 9 * * *").is_ok());
        // 单次（未来时间）→ 通过（仅一个触发点，无重复风险）
        assert!(validate_cron("0 0 9 20 1 * 2099").is_ok());
        // 非法表达式 → 拒绝
        assert!(validate_cron("bad").is_err());
    }

    #[test]
    fn is_once_cron_detects_year_field() {
        assert!(is_once_cron("0 0 9 20 1 * 2026"));
        assert!(!is_once_cron("0 0 9 * * *"));
    }

    #[test]
    fn busy_policy_normalization() {
        assert_eq!(normalize_busy_policy("defer"), "defer");
        assert_eq!(normalize_busy_policy("skip"), "skip");
        assert_eq!(normalize_busy_policy("SKIP"), "skip");
        // 创建默认/非法值回退为跳过
        assert_eq!(normalize_busy_policy("whatever"), "skip");
        assert_eq!(normalize_busy_policy(""), "skip");
    }

    #[test]
    fn decision_matrix_aligns_with_permissions_ts() {
        // 与前端 toApprovalPolicy / toSandbox 对齐
        assert_eq!(to_approval_policy("ask-for-approval"), "on-request");
        assert_eq!(to_approval_policy("help-me-approve"), "on-request");
        assert_eq!(to_approval_policy("read-only"), "never");
        assert_eq!(to_approval_policy("full-access"), "never");
        assert_eq!(to_sandbox_policy("read-only"), "readOnly");
        assert_eq!(to_sandbox_policy("ask-for-approval"), "workspaceWrite");
        assert_eq!(to_sandbox_policy("help-me-approve"), "workspaceWrite");
        assert_eq!(to_sandbox_policy("full-access"), "dangerFullAccess");
        assert_eq!(to_sandbox_policy("unknown"), "dangerFullAccess");
    }

    #[test]
    fn decide_fire_matrix() {
        // 空闲即执行
        assert_eq!(decide_fire(false, "defer"), FireAction::Execute);
        assert_eq!(decide_fire(false, "skip"), FireAction::Execute);
        // 忙时按任务策略：defer 顺延 / skip 跳过
        assert_eq!(decide_fire(true, "defer"), FireAction::Defer);
        assert_eq!(decide_fire(true, "skip"), FireAction::Skip);
        // 非法策略回退 skip（创建默认）
        assert_eq!(decide_fire(true, "bad"), FireAction::Skip);
    }

    #[test]
    fn decide_first_tick_marks_due_tasks_missed_and_ignores_rest() {
        let due = 1_000i64;
        let task = |busy_policy: &str| ScheduledTask {
            id: "t-1".into(),
            name: "n".into(),
            prompt: "p".into(),
            cron: "0 0 9 * * *".into(),
            thread_id: "th".into(),
            busy_policy: busy_policy.into(),
            enabled: true,
            done: false,
            created_at: 0,
            next_run: Some(due),
        };

        // 到期且启用：无论忙时策略（defer/skip）一律记「已错过」
        assert_eq!(
            decide_first_tick(due + 1, &task("defer")),
            FirstTickAction::MissAndAdvance
        );
        assert_eq!(
            decide_first_tick(due + 1, &task("skip")),
            FirstTickAction::MissAndAdvance
        );
        // 临界：now == next_run 也算到期
        assert_eq!(
            decide_first_tick(due, &task("skip")),
            FirstTickAction::MissAndAdvance
        );

        // 未到期 → Idle
        let not_due = ScheduledTask {
            next_run: Some(due + 100),
            ..task("skip")
        };
        assert_eq!(decide_first_tick(due, &not_due), FirstTickAction::Idle);
        // 停用 / 已归档 → Idle
        let disabled = ScheduledTask {
            enabled: false,
            ..task("defer")
        };
        assert_eq!(decide_first_tick(due + 1, &disabled), FirstTickAction::Idle);
        let done = ScheduledTask {
            done: true,
            ..task("skip")
        };
        assert_eq!(decide_first_tick(due + 1, &done), FirstTickAction::Idle);
        // next_run 缺失（无可触发时刻）→ Idle
        let no_next = ScheduledTask {
            next_run: None,
            ..task("skip")
        };
        assert_eq!(decide_first_tick(due, &no_next), FirstTickAction::Idle);
    }

    #[test]
    fn extract_result_text_picks_last_nonempty_agent_message() {
        let resp = json!({
            "data": [{
                "id": "turn-a",
                "items": [
                    { "type": "userMessage", "text": "问题" },
                    { "type": "agentMessage", "text": "  " },
                    { "type": "agentMessage", "text": "最终回复" },
                ],
            }]
        });
        assert_eq!(
            extract_result_text(&resp, "turn-a").as_deref(),
            Some("最终回复")
        );
        // 指定回合不存在 / 全空消息
        assert_eq!(extract_result_text(&resp, "turn-b"), None);
        let empty = json!({ "data": [{ "id": "turn-c", "items": [] }] });
        assert_eq!(extract_result_text(&empty, "turn-c"), None);
    }

    #[test]
    fn truncate_chars_is_utf8_safe() {
        assert_eq!(truncate_chars("abc", 10), "abc");
        assert_eq!(truncate_chars("你好世界", 2), "你好…");
        assert_eq!(truncate_chars("", 5), "");
    }

    #[test]
    fn task_finish_notice_builds_success_with_duration_and_result() {
        let (title, body) =
            task_finish_notice("测试任务", "success", Some(1500), Some("已完成总结"), None);
        assert_eq!(title, "定时任务执行完成");
        assert!(body.contains("「测试任务」执行完成，耗时 1 秒"));
        assert!(body.contains("已完成总结"));
    }

    #[test]
    fn task_finish_notice_builds_failure_with_error() {
        let (title, body) =
            task_finish_notice("测试任务", "failed", None, None, Some("启动回合失败：boom"));
        assert_eq!(title, "定时任务执行失败");
        assert!(body.contains("「测试任务」执行失败：启动回合失败：boom"));
    }

    #[test]
    fn task_finish_notice_falls_back_to_unknown_error() {
        let (title, body) = task_finish_notice("测试任务", "failed", None, None, None);
        assert_eq!(title, "定时任务执行失败");
        assert!(body.contains("「测试任务」执行失败：未知错误"));
    }

    #[test]
    fn store_roundtrip_and_cascades() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = ScheduledTaskStore::open(dir.path()).unwrap();

        assert!(store.add("", "p", "0 0 9 * * *", "t1", "defer").is_err());
        assert!(store.add("n", "", "0 0 9 * * *", "t1", "defer").is_err());
        assert!(store.add("n", "p", "* * * * * *", "t1", "defer").is_err());
        let t1 = store
            .add("每日总结", "总结提交", "0 0 9 * * *", "t1", "defer")
            .unwrap();
        let t2 = store
            .add("临时", "做点事", "0 0 12 * * *", "t2", "skip")
            .unwrap();
        assert_eq!(t1.busy_policy, "defer");
        assert_eq!(t2.busy_policy, "skip");
        assert!(t1.enabled && !t1.done);
        assert!(t1.next_run.is_some());
        assert_eq!(store.list().len(), 2);
        assert_eq!(store.get(&t1.id).unwrap().name, "每日总结");

        // 执行记录：running → success；任务删除后 start_run 返回 None
        let run1 = store
            .start_run(&t1.id, 100, Some("turn-1"))
            .unwrap()
            .unwrap();
        assert!(store.start_run("task-gone", 100, None).unwrap().is_none());
        store
            .finish_run(run1, "success", Some(1500), Some("已完成总结"), None)
            .unwrap();
        store
            .record_event_run(&t1.id, 200, "skipped", Some("会话正忙"))
            .unwrap();
        let runs = store.list_runs(&t1.id, 20, 0).unwrap();
        assert_eq!(runs.len(), 2);
        // 倒序：后插入的在前
        assert_eq!(runs[0].status, "skipped");
        assert_eq!(runs[1].result.as_deref(), Some("已完成总结"));
        // 分页
        assert_eq!(store.list_runs(&t1.id, 1, 0).unwrap().len(), 1);
        assert_eq!(store.list_runs(&t1.id, 20, 2).unwrap().len(), 0);

        // 删任务级联删记录
        assert!(store.remove(&t1.id).unwrap());
        assert!(store.list_runs(&t1.id, 20, 0).unwrap().is_empty());

        // 删会话级联删任务与记录
        let run2 = store.start_run(&t2.id, 300, None).unwrap().unwrap();
        store
            .finish_run(run2, "failed", None, None, Some("出错"))
            .unwrap();
        let removed = store.remove_by_thread("t2").unwrap();
        assert_eq!(removed, vec![t2.id.clone()]);
        assert!(store.list().is_empty());
        assert!(store.list_runs(&t2.id, 20, 0).unwrap().is_empty());
        assert!(!store.remove(&t2.id).unwrap());
    }

    #[test]
    fn reopen_clears_zombie_running_records() {
        let dir = tempfile::TempDir::new().unwrap();
        let task_id = {
            let store = ScheduledTaskStore::open(dir.path()).unwrap();
            let t = store.add("n", "p", "0 0 9 * * *", "t1", "defer").unwrap();
            store
                .start_run(&t.id, 100, Some("turn-x"))
                .unwrap()
                .unwrap();
            assert_eq!(store.list_runs(&t.id, 20, 0).unwrap()[0].status, "running");
            t.id
        };
        // 重新打开（模拟应用重启）：running → failed（应用中断）
        let store = ScheduledTaskStore::open(dir.path()).unwrap();
        let runs = store.list_runs(&task_id, 20, 0).unwrap();
        assert_eq!(runs[0].status, "failed");
        assert_eq!(runs[0].error.as_deref(), Some("应用中断"));
    }
}
