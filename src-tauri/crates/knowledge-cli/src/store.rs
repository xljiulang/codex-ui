//! 单个知识库的存储（SQLite：元数据 + 文档 + 切块 + 向量 + FTS5 关键词索引）。
//!
//! 库由**库名**标识：`meta.kb` 存规范化库名（寻址键）、`meta.kb_display` 存展示名；
//! 来源目录记在 `meta.source`/`meta.source_display`（`sources` 表同步写一行以兼容旧读法）。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::paths;

/// 库结构版本：变更表结构时递增（旧库按版本重建，不做增量迁移）
const SCHEMA_VERSION: i64 = 1;

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// 设置页「知识库」列表行
#[derive(Debug, Clone, Serialize)]
pub struct KbSummary {
    /// 库文件名（损坏库删除时回传该值）
    pub file: String,
    /// 库名（展示用大小写；缺元数据时为空）
    pub kb: String,
    /// 索引来源目录（原始路径；缺元数据时为空）
    pub source: String,
    pub docs: i64,
    pub chunks: i64,
    /// 上次更新时间（Unix 秒；0 表示未索引过）
    pub updated_at: i64,
    /// 是否可正常使用（损坏/缺元数据为 false，仍允许删除）
    pub available: bool,
    pub error: Option<String>,
}

/// `create` 的结果
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    /// 规范化库名（寻址键）
    pub kb: String,
    /// 展示用库名
    pub display: String,
    /// 来源目录（原始路径）
    pub source: String,
    /// 库文件名
    pub file: String,
    /// 本次是否新建；false = 已存在且来源一致（幂等）
    pub created: bool,
}

/// 检索命中的切块
#[derive(Debug, Clone, Serialize)]
pub struct ChunkHit {
    pub doc_path: String,
    pub title_path: String,
    pub text: String,
    pub score: f32,
}

pub struct KbStore {
    conn: Connection,
    pub path: PathBuf,
}

impl KbStore {
    /// 新建或打开指定库名的知识库：同名且来源一致时幂等返回 `created=false`；
    /// 同名但来源不同直接报错（不静默改写）；文件槽位冲突时顺延，绝不覆盖他人数据。
    pub fn create(
        data_dir: &Path,
        normalized: &str,
        display: &str,
        source: &str,
    ) -> Result<(Self, bool), String> {
        let dir = paths::kbs_dir(data_dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("创建知识库目录失败 {}: {e}", dir.display()))?;
        let stem = paths::kb_stem(normalized);
        for slot in 1..=8 {
            let name = if slot == 1 {
                format!("{stem}.sqlite")
            } else {
                format!("{stem}-{slot}.sqlite")
            };
            let path = dir.join(&name);
            let existed = path.exists();
            let conn = Connection::open(&path).map_err(|e| format!("打开知识库失败：{e}"))?;
            Self::init(&conn)?;
            let store = KbStore { conn, path };
            match store.meta_get("kb") {
                Some(existing) if existing == normalized => {
                    let current = store.source_display();
                    if !current.is_empty() && paths::normalize_dir(&current) != paths::normalize_dir(source)
                    {
                        return Err(format!(
                            "库「{display}」已存在，来源为 {current}，与本次 {source} 不一致；\
                             请先用 list 查看，或换一个库名（删除旧库：codexui-kb delete \"{display}\"）"
                        ));
                    }
                    return Ok((store, false));
                }
                None if !existed || store.counts() == (0, 0) => {
                    store.meta_set("kb", normalized)?;
                    store.meta_set("kb_display", display)?;
                    store.set_source(source)?;
                    return Ok((store, true));
                }
                // 属于别的库（哈希碰撞）：换下一个槽位，绝不覆盖
                _ => continue,
            }
        }
        Err("知识库文件槽位已用尽（同名库冲突），请清理 knowledge/kbs 后重试".into())
    }

    /// 只打开已存在的库（不创建、不改写元数据）；不存在返回 None
    pub fn open_existing(data_dir: &Path, normalized: &str) -> Result<Option<Self>, String> {
        let Some(path) = resolve_existing_path(data_dir, normalized)? else {
            return Ok(None);
        };
        let conn = Connection::open(&path).map_err(|e| format!("打开知识库失败：{e}"))?;
        Self::init(&conn)?;
        Ok(Some(KbStore { conn, path }))
    }

    fn init(conn: &Connection) -> Result<(), String> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("设置 WAL 失败：{e}"))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| format!("开启外键失败：{e}"))?;
        conn.busy_timeout(std::time::Duration::from_millis(3000))
            .map_err(|e| format!("设置 busy_timeout 失败：{e}"))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS sources (
              path TEXT PRIMARY KEY,
              added_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS documents (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              root TEXT NOT NULL,
              path TEXT NOT NULL UNIQUE,
              size INTEGER NOT NULL,
              mtime INTEGER NOT NULL,
              status TEXT NOT NULL,
              error TEXT,
              updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS chunks (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              doc_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
              ord INTEGER NOT NULL,
              title_path TEXT NOT NULL,
              text TEXT NOT NULL,
              embed BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS chunks_doc ON chunks(doc_id);
            CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
              text,
              title_path,
              content='chunks',
              content_rowid='id',
              tokenize='trigram'
            );
            CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
              INSERT INTO chunks_fts(rowid, text, title_path)
              VALUES (new.id, new.text, new.title_path);
            END;
            CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
              INSERT INTO chunks_fts(chunks_fts, rowid, text, title_path)
              VALUES ('delete', old.id, old.text, old.title_path);
            END;
            CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON chunks BEGIN
              INSERT INTO chunks_fts(chunks_fts, rowid, text, title_path)
              VALUES ('delete', old.id, old.text, old.title_path);
              INSERT INTO chunks_fts(rowid, text, title_path)
              VALUES (new.id, new.text, new.title_path);
            END;
            "#,
        )
        .map_err(|e| format!("初始化知识库表失败：{e}"))?;
        let version = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(|e| format!("读取库版本失败：{e}"))?;
        if version.as_deref() != Some(&SCHEMA_VERSION.to_string()) {
            conn.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version', ?1)",
                params![SCHEMA_VERSION.to_string()],
            )
            .map_err(|e| format!("写入库版本失败：{e}"))?;
        }
        Ok(())
    }

    pub fn meta_get(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .ok()
            .flatten()
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES(?1, ?2)",
                params![key, value],
            )
            .map_err(|e| format!("写入知识库元数据失败：{e}"))?;
        Ok(())
    }

    /// 展示用库名：优先原始大小写，缺失时回落到规范化名
    pub fn kb_display(&self) -> String {
        self.meta_get("kb_display")
            .or_else(|| self.meta_get("kb"))
            .unwrap_or_default()
    }

    /// 展示用来源目录
    pub fn source_display(&self) -> String {
        self.meta_get("source_display")
            .or_else(|| self.meta_get("source"))
            .unwrap_or_default()
    }

    /// 写入来源目录（`meta.source` + `sources` 表，兼容旧读法）
    pub fn set_source(&self, source: &str) -> Result<(), String> {
        self.meta_set("source", &paths::normalize_dir(source))?;
        self.meta_set("source_display", source.trim())?;
        self.add_source(source.trim())
    }

    /// 内容修订号：每次索引写入递增
    pub fn revision(&self) -> String {
        self.meta_get("revision").unwrap_or_else(|| "0".to_string())
    }

    fn bump_revision(&self) -> Result<(), String> {
        let next = self
            .revision()
            .parse::<u64>()
            .unwrap_or(0)
            .wrapping_add(1);
        self.meta_set("revision", &next.to_string())
    }

    pub fn touch_updated(&self) -> Result<(), String> {
        self.meta_set("updated_at", &now_secs().to_string())?;
        self.bump_revision()
    }

    pub fn updated_at(&self) -> i64 {
        self.meta_get("updated_at")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    pub fn counts(&self) -> (i64, i64) {
        let docs = self
            .conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .unwrap_or(0);
        let chunks = self
            .conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
            .unwrap_or(0);
        (docs, chunks)
    }

    fn add_source(&self, root: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO sources(path, added_at) VALUES(?1, ?2)",
                params![root, now_secs()],
            )
            .map_err(|e| format!("记录来源失败：{e}"))?;
        Ok(())
    }

    /// 已知来源目录（新库恒为一个）
    pub fn sources(&self) -> Vec<String> {
        let mut stmt = match self.conn.prepare("SELECT path FROM sources ORDER BY path") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |r| r.get::<_, String>(0));
        match rows {
            Ok(it) => it.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 已入库文档的状态：`(size, mtime, status, error)`；不存在返回 None
    pub fn document_state(&self, path: &str) -> Option<(i64, i64, String, Option<String>)> {
        self.conn
            .query_row(
                "SELECT size, mtime, status, error FROM documents WHERE path = ?1",
                params![path],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn document_id(&self, path: &str) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT id FROM documents WHERE path = ?1",
                params![path],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn chunk_count_for(&self, doc_id: i64) -> i64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE doc_id = ?1",
                params![doc_id],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    /// 写入文档行（存在则更新基础信息并保留 id）
    pub fn upsert_document(
        &self,
        root: &str,
        path: &str,
        size: i64,
        mtime: i64,
    ) -> Result<i64, String> {
        self.conn
            .execute(
                r#"INSERT INTO documents(root, path, size, mtime, status, error, updated_at)
                   VALUES(?1, ?2, ?3, ?4, 'pending', NULL, ?5)
                   ON CONFLICT(path) DO UPDATE SET
                     root = excluded.root,
                     size = excluded.size,
                     mtime = excluded.mtime,
                     updated_at = excluded.updated_at"#,
                params![root, path, size, mtime, now_secs()],
            )
            .map_err(|e| format!("写入文档记录失败：{e}"))?;
        self.document_id(path)
            .ok_or_else(|| "写入文档记录后取不到 id".to_string())
    }

    pub fn set_document_status(
        &self,
        doc_id: i64,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE documents SET status = ?1, error = ?2, updated_at = ?3 WHERE id = ?4",
                params![status, error, now_secs(), doc_id],
            )
            .map_err(|e| format!("更新文档状态失败：{e}"))?;
        Ok(())
    }

    /// 用新的切块替换该文档的全部切块（含向量）
    pub fn replace_chunks(
        &self,
        doc_id: i64,
        chunks: &[(String, String, Vec<f32>)],
    ) -> Result<(), String> {
        // 单文档一个事务：中途失败/被杀进程时不会留下"旧切块已删、新切块只写了一半"的半成品
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("开启写入事务失败：{e}"))?;
        let outcome = self.replace_chunks_inner(doc_id, chunks);
        match &outcome {
            Ok(()) => self
                .conn
                .execute_batch("COMMIT")
                .map_err(|e| format!("提交写入事务失败：{e}"))?,
            Err(_) => {
                let _ = self.conn.execute_batch("ROLLBACK");
            }
        }
        outcome
    }

    fn replace_chunks_inner(
        &self,
        doc_id: i64,
        chunks: &[(String, String, Vec<f32>)],
    ) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM chunks WHERE doc_id = ?1", params![doc_id])
            .map_err(|e| format!("清理旧切块失败：{e}"))?;
        for (ord, (title_path, text, embed)) in chunks.iter().enumerate() {
            let blob = encode_vector(embed);
            self.conn
                .execute(
                    "INSERT INTO chunks(doc_id, ord, title_path, text, embed) VALUES(?1, ?2, ?3, ?4, ?5)",
                    params![doc_id, ord as i64, title_path, text, blob],
                )
                .map_err(|e| format!("写入切块失败：{e}"))?;
        }
        Ok(())
    }

    /// 删除路径已不存在的文档（限定在指定来源根下），返回删除条数
    pub fn prune_missing_under(&self, root: &str) -> Result<i64, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM documents WHERE path LIKE ?1 || '%'")
            .map_err(|e| format!("查询文档失败：{e}"))?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![root], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| format!("查询文档失败：{e}"))?
            .flatten()
            .collect();
        drop(stmt);
        let mut removed = 0;
        for (id, path) in rows {
            if !Path::new(&path).exists() {
                self.conn
                    .execute("DELETE FROM documents WHERE id = ?1", params![id])
                    .map_err(|e| format!("删除失效文档失败：{e}"))?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// 全量读取向量（供检索缓存）：返回 `(chunk_ids, 扁平向量, 维度)`
    pub fn all_embeddings(&self) -> Result<(Vec<i64>, Vec<f32>, usize), String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, embed FROM chunks ORDER BY id")
            .map_err(|e| format!("读取向量失败：{e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .map_err(|e| format!("读取向量失败：{e}"))?;
        let mut ids = Vec::new();
        let mut flat = Vec::new();
        let mut dim = 0usize;
        for row in rows.flatten() {
            let (id, blob) = row;
            let vec = decode_vector(&blob);
            if dim == 0 {
                dim = vec.len();
            }
            if vec.len() != dim || dim == 0 {
                continue;
            }
            ids.push(id);
            flat.extend_from_slice(&vec);
        }
        Ok((ids, flat, dim))
    }

    /// FTS5 关键词检索：`match_expr` 已是 MATCH 表达式（由 `search::build_match_expr` 构造），
    /// 返回 `(chunk_id, bm25)`，bm25 越小越相关
    pub fn fts_search(&self, match_expr: &str, limit: usize) -> Result<Vec<(i64, f64)>, String> {
        if match_expr.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT rowid, bm25(chunks_fts) AS rank FROM chunks_fts
                 WHERE chunks_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            )
            .map_err(|e| format!("关键词检索失败：{e}"))?;
        let rows = stmt
            .query_map(params![match_expr, limit as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
            })
            .map_err(|e| format!("关键词检索失败：{e}"))?;
        Ok(rows.flatten().collect())
    }

    /// 短查询兜底：LIKE 子串匹配（按出现位置粗排）
    pub fn like_search(&self, query: &str, limit: usize) -> Result<Vec<i64>, String> {
        let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id FROM chunks WHERE text LIKE ?1 ESCAPE '\\' OR title_path LIKE ?1 ESCAPE '\\'
                 ORDER BY id LIMIT ?2",
            )
            .map_err(|e| format!("关键词兜底检索失败：{e}"))?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], |r| r.get::<_, i64>(0))
            .map_err(|e| format!("关键词兜底检索失败：{e}"))?;
        Ok(rows.flatten().collect())
    }

    pub fn chunks_by_ids(&self, ids: &[i64]) -> Result<Vec<ChunkHit>, String> {
        let mut out = Vec::new();
        for id in ids {
            let row = self
                .conn
                .query_row(
                    "SELECT c.title_path, c.text, d.path FROM chunks c
                     JOIN documents d ON d.id = c.doc_id WHERE c.id = ?1",
                    params![id],
                    |r| {
                        Ok(ChunkHit {
                            doc_path: r.get(2)?,
                            title_path: r.get(0)?,
                            text: r.get(1)?,
                            score: 0.0,
                        })
                    },
                )
                .optional()
                .map_err(|e| format!("读取命中切块失败：{e}"))?;
            if let Some(hit) = row {
                out.push(hit);
            }
        }
        Ok(out)
    }
}

/// 解析该库名已存在的库文件路径（按槽位顺序，校验 `meta.kb` 归属）
pub fn resolve_existing_path(data_dir: &Path, normalized: &str) -> Result<Option<PathBuf>, String> {
    let dir = paths::kbs_dir(data_dir);
    let stem = paths::kb_stem(normalized);
    for slot in 1..=8 {
        let name = if slot == 1 {
            format!("{stem}.sqlite")
        } else {
            format!("{stem}-{slot}.sqlite")
        };
        let path = dir.join(&name);
        if !path.is_file() {
            continue;
        }
        let conn = Connection::open(&path).map_err(|e| format!("打开知识库失败：{e}"))?;
        let existing: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'kb'", [], |r| r.get(0))
            .optional()
            .unwrap_or(None);
        if existing.as_deref() == Some(normalized) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// 解析库文件路径（不存在则报错），供 `index`/`delete` 使用
pub fn require_existing_path(data_dir: &Path, normalized: &str, display: &str) -> Result<PathBuf, String> {
    resolve_existing_path(data_dir, normalized)?.ok_or_else(|| {
        format!(
            "库「{display}」尚未建立：请先执行 codexui-kb create \"{display}\" <目录>；\
             可用 codexui-kb list 查看已有库"
        )
    })
}

/// 扫描 `kbs/` 下全部知识库，按更新时间倒序返回（损坏/缺元数据的标记为不可用但仍可删）
pub fn list_all(data_dir: &Path) -> Vec<KbSummary> {
    let dir = paths::kbs_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sqlite") {
            continue;
        }
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        match Connection::open(&path) {
            Ok(conn) => {
                let read_meta = |key: &str| -> Option<String> {
                    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                        r.get::<_, String>(0)
                    })
                    .optional()
                    .ok()
                    .flatten()
                };
                let kb = read_meta("kb_display").or_else(|| read_meta("kb"));
                let source = read_meta("source_display").or_else(|| read_meta("source"));
                let docs = conn
                    .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get::<_, i64>(0))
                    .unwrap_or(0);
                let chunks = conn
                    .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get::<_, i64>(0))
                    .unwrap_or(0);
                let updated_at = read_meta("updated_at")
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0);
                let available = kb.is_some();
                out.push(KbSummary {
                    file,
                    kb: kb.unwrap_or_default(),
                    source: source.unwrap_or_default(),
                    docs,
                    chunks,
                    updated_at,
                    available,
                    error: if available {
                        None
                    } else {
                        Some(
                            "旧版或损坏的知识库（缺少库名元数据），请删除后用 create 重建"
                                .to_string(),
                        )
                    },
                });
            }
            Err(e) => out.push(KbSummary {
                file,
                kb: String::new(),
                source: String::new(),
                docs: 0,
                chunks: 0,
                updated_at: 0,
                available: false,
                error: Some(format!("打开失败：{e}")),
            }),
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.file.cmp(&b.file)));
    out
}

/// 删除 `kbs/` 下的库文件（含 WAL/SHM 残留）；只接受纯文件名，拒绝路径穿越
pub fn delete_file(data_dir: &Path, file: &str) -> Result<(), String> {
    let name = Path::new(file);
    if name.file_name().map(|f| f.to_string_lossy().to_string()) != Some(file.to_string())
        || !file.ends_with(".sqlite")
    {
        return Err("非法的知识库文件名".into());
    }
    let path = paths::kbs_dir(data_dir).join(file);
    if !path.is_file() {
        return Err("知识库文件不存在".into());
    }
    for suffix in ["", "-wal", "-shm"] {
        let target = PathBuf::from(format!("{}{suffix}", path.display()));
        if target.exists() {
            std::fs::remove_file(&target)
                .map_err(|e| format!("删除知识库失败 {}: {e}", target.display()))?;
        }
    }
    Ok(())
}

/// `Vec<f32>` → 小端字节（写入 BLOB）
pub fn encode_vector(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// 小端字节 → `Vec<f32>`（读取 BLOB）
pub fn decode_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_kb(dir: &TempDir, name: &str, source: &str) -> KbStore {
        let (norm, display) = paths::normalize_kb_name(name).unwrap();
        KbStore::create(dir.path(), &norm, &display, source).unwrap().0
    }

    #[test]
    fn kb_file_name_is_stable_and_normalized() {
        let dir = TempDir::new().unwrap();
        let (norm, _) = paths::normalize_kb_name("售后手册").unwrap();
        let a = paths::kb_path(dir.path(), &norm);
        let (norm2, _) = paths::normalize_kb_name(" 售后手册 ").unwrap();
        let b = paths::kb_path(dir.path(), &norm2);
        assert_eq!(a, b);
        let name = a.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.ends_with(".sqlite"));
        assert!(name.contains("售后手册"));
    }

    #[test]
    fn create_is_idempotent_and_rejects_different_source() {
        let dir = TempDir::new().unwrap();
        let (store, created) =
            KbStore::create(dir.path(), "手册", "手册", "D:\\售后\\手册").unwrap();
        assert!(created);
        assert_eq!(store.source_display(), "D:\\售后\\手册");
        drop(store);

        // 同名同来源（大小写/斜杠差异）→ 幂等
        let (_, created) =
            KbStore::create(dir.path(), "手册", "手册", "d:/售后/手册/").unwrap();
        assert!(!created);

        // 同名不同来源 → 报错且不改写
        let err = KbStore::create(dir.path(), "手册", "手册", "E:\\别的目录")
            .map(|_| ())
            .unwrap_err();
        assert!(err.contains("来源"), "{err}");
        let (store, _) = KbStore::create(dir.path(), "手册", "手册", "D:\\售后\\手册").unwrap();
        assert_eq!(store.source_display(), "D:\\售后\\手册");
    }

    #[test]
    fn hash_collision_slots_do_not_share_data() {
        let dir = TempDir::new().unwrap();
        let first = create_kb(&dir, "a", "D:\\a");
        first.meta_set("kb", "a").unwrap();
        drop(first);
        // 手工把首选槽位伪造成别的库，验证会顺延到 -2 而不是共用一个库
        let path = paths::kb_path(dir.path(), "a");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES('kb', 'other')",
                [],
            )
            .unwrap();
        }
        let (second, _) = KbStore::create(dir.path(), "a", "a", "D:\\a").unwrap();
        assert_ne!(second.path, path);
        assert_eq!(second.meta_get("kb").as_deref(), Some("a"));
    }

    #[test]
    fn open_existing_requires_matching_kb_name() {
        let dir = TempDir::new().unwrap();
        assert!(KbStore::open_existing(dir.path(), "手册").unwrap().is_none());
        let _kb = create_kb(&dir, "手册", "D:\\kb");
        assert!(KbStore::open_existing(dir.path(), "手册").unwrap().is_some());
        assert!(KbStore::open_existing(dir.path(), "别的").unwrap().is_none());
        let err = require_existing_path(dir.path(), "别的", "别的").unwrap_err();
        assert!(err.contains("create"), "{err}");
    }

    #[test]
    fn documents_chunks_and_keyword_search_roundtrip() {
        let dir = TempDir::new().unwrap();
        let kb = create_kb(&dir, "kb", "D:\\kb");
        let doc_id = kb
            .upsert_document("D:\\kb", "D:\\kb\\手册.md", 10, 20)
            .unwrap();
        kb.replace_chunks(
            doc_id,
            &[
                (
                    "手册 › 故障".to_string(),
                    "错误码 E-1043 表示电源适配器异常".to_string(),
                    vec![1.0, 0.0, 0.0],
                ),
                (
                    "手册 › 安装".to_string(),
                    "安装前请断开电源".to_string(),
                    vec![0.0, 1.0, 0.0],
                ),
            ],
        )
        .unwrap();
        kb.set_document_status(doc_id, "ok", None).unwrap();
        assert_eq!(kb.counts(), (1, 2));

        let hits = kb.fts_search("\"错误码 E-1043\"", 10).unwrap();
        assert_eq!(hits.len(), 1, "trigram 应命中含错误码的切块");
        let rows = kb.chunks_by_ids(&[hits[0].0]).unwrap();
        assert!(rows[0].text.contains("E-1043"));

        // 短查询不构造 MATCH 表达式，由 LIKE 兜底
        assert!(kb.fts_search("", 10).unwrap().is_empty());
        assert_eq!(kb.like_search("E-", 10).unwrap().len(), 1);
    }

    #[test]
    fn replacing_chunks_keeps_fts_in_sync() {
        let dir = TempDir::new().unwrap();
        let kb = create_kb(&dir, "kb", "D:\\kb");
        let doc_id = kb.upsert_document("D:\\kb", "D:\\kb\\a.md", 1, 1).unwrap();
        kb.replace_chunks(
            doc_id,
            &[("t".into(), "旧内容 ABCDEF".into(), vec![1.0, 0.0])],
        )
        .unwrap();
        kb.replace_chunks(
            doc_id,
            &[("t".into(), "新内容 UVWXYZ".into(), vec![0.0, 1.0])],
        )
        .unwrap();
        assert!(kb.fts_search("\"ABCDEF\"", 10).unwrap().is_empty());
        assert_eq!(kb.fts_search("\"UVWXYZ\"", 10).unwrap().len(), 1);
    }

    #[test]
    fn delete_document_cascades_chunks() {
        let dir = TempDir::new().unwrap();
        let kb = create_kb(&dir, "kb", "D:\\kb");
        let doc_id = kb.upsert_document("D:\\kb", "D:\\kb\\gone.md", 1, 1).unwrap();
        kb.replace_chunks(doc_id, &[("t".into(), "内容".into(), vec![1.0])])
            .unwrap();
        kb.prune_missing_under("D:\\kb").unwrap();
        assert_eq!(kb.counts(), (0, 0));
    }

    #[test]
    fn list_and_delete_cover_stray_files() {
        let dir = TempDir::new().unwrap();
        let kb = create_kb(&dir, "手册", "D:\\kb");
        kb.upsert_document("D:\\kb", "D:\\kb\\a.md", 1, 1).unwrap();
        kb.touch_updated().unwrap();
        let file = kb.path.file_name().unwrap().to_string_lossy().to_string();
        drop(kb);

        // 伪造一个损坏库：能被列表识别为不可用且可删除
        let broken = paths::kbs_dir(dir.path()).join("broken-00000000.sqlite");
        std::fs::write(&broken, b"not a database").unwrap();

        let list = list_all(dir.path());
        assert_eq!(list.len(), 2);
        let row = list.iter().find(|r| r.file == file).unwrap();
        assert_eq!(row.kb, "手册");
        assert_eq!(row.source, "D:\\kb");
        assert!(row.available && row.docs == 1);
        let broken_row = list.iter().find(|r| r.file == "broken-00000000.sqlite").unwrap();
        assert!(!broken_row.available);
        assert!(broken_row.error.is_some());

        delete_file(dir.path(), "broken-00000000.sqlite").unwrap();
        assert!(!broken.exists());
        assert!(delete_file(dir.path(), "../escape.sqlite").is_err());
        delete_file(dir.path(), &file).unwrap();
        assert!(list_all(dir.path()).is_empty());
    }

    #[test]
    fn vector_blob_roundtrip() {
        let v = vec![0.5f32, -1.25, 3.0];
        let blob = encode_vector(&v);
        assert_eq!(decode_vector(&blob), v);
        assert!(decode_vector(&[]).is_empty());
    }
}
