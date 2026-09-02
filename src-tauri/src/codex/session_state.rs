//! 会话状态统一持久化：按 threadId 保存会话的「外在状态」——微信绑定、权限模式、模型、推理强度。
//!
//! 单一数据文件 `<app_dir>/sessions.json`，键 = threadId，值为记录对象。
//! 首次加载时若旧 `wechat/bindings.json` 存在且 `sessions.json` 不存在，自动迁移并删除旧文件。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 持久化的微信绑定子字段（threadId 为记录键，故不入内）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WechatBinding {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_at: Option<u64>,
}

/// 单个会话的持久化状态；字段缺省（或显式 null）即表示未设置/使用默认。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct SessionState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wechat: Option<WechatBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl SessionState {
    /// 记录是否没有有效内容（可安全从字典移除）。
    pub fn is_empty(&self) -> bool {
        self.wechat.is_none()
            && self.permission_mode.is_none()
            && self.model.is_none()
            && self.effort.is_none()
    }
}

/// 会话状态存储：内存映射 + 磁盘原子写。
pub struct SessionStateStore {
    path: PathBuf,
    data: Mutex<HashMap<String, SessionState>>,
}

fn sessions_path(app_dir: &Path) -> PathBuf {
    app_dir.join("sessions.json")
}

fn legacy_bindings_path(app_dir: &Path) -> PathBuf {
    app_dir.join("wechat").join("bindings.json")
}

impl SessionStateStore {
    /// 创建存储并加载磁盘；必要时迁移旧 wechat/bindings.json。
    pub fn new(app_dir: &Path) -> Result<Self, String> {
        let path = sessions_path(app_dir);
        let data = Self::read_disk(&path);
        let store = Self {
            path,
            data: Mutex::new(data),
        };
        // 每次发现遗留 bindings.json 都合并（覆盖降级再升级场景：旧版产生的绑定不丢失）。
        if legacy_bindings_path(app_dir).exists() {
            store.migrate_legacy(app_dir)?;
        }
        Ok(store)
    }

    fn read_disk(path: &Path) -> HashMap<String, SessionState> {
        let Ok(text) = fs::read_to_string(path) else {
            return HashMap::new();
        };
        serde_json::from_str(&text).unwrap_or_else(|_| HashMap::new())
    }

    fn persist(&self) -> Result<(), String> {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let text = serde_json::to_string_pretty(&data)
            .map_err(|e| format!("序列化会话状态失败: {e}"))?;
        let p = &self.path;
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        let tmp = p.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("写入会话状态失败: {e}"))?;
        if p.exists() {
            fs::remove_file(p).map_err(|e| format!("替换会话状态失败: {e}"))?;
        }
        fs::rename(&tmp, p).map_err(|e| format!("落盘会话状态失败: {e}"))
    }

    /// 读取某线程的持久化状态。
    pub fn get(&self, thread_id: &str) -> Option<SessionState> {
        self.data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(thread_id)
            .cloned()
    }

    /// 前端写入权限/模型/推理强度（全量覆盖这三个字段，保留 wechat）；
    /// None 表示「恢复默认」，落盘为缺省。记录变空则移除。
    pub fn set_settings(
        &self,
        thread_id: &str,
        permission_mode: Option<String>,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<(), String> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let mut rec = data.get(thread_id).cloned().unwrap_or_default();
        rec.permission_mode = permission_mode;
        rec.model = model;
        rec.effort = effort;
        if rec.is_empty() {
            data.remove(thread_id);
        } else {
            data.insert(thread_id.to_string(), rec);
        }
        drop(data);
        self.persist()
    }

    /// 桥写入/清除某线程的微信绑定；记录被清空则移除。
    pub fn set_wechat(
        &self,
        thread_id: &str,
        binding: Option<WechatBinding>,
    ) -> Result<(), String> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let mut rec = data.get(thread_id).cloned().unwrap_or_default();
        rec.wechat = binding;
        if rec.is_empty() {
            data.remove(thread_id);
        } else {
            data.insert(thread_id.to_string(), rec);
        }
        drop(data);
        self.persist()
    }

    /// 清空某线程记录（删除会话时调用）。
    pub fn remove(&self, thread_id: &str) -> Result<(), String> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        data.remove(thread_id);
        drop(data);
        self.persist()
    }

    /// 导出全部带微信绑定的记录为桥可读的绑定列表（含 threadId 字段）。
    pub fn wechat_bindings(&self) -> Vec<Value> {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<(String, Value)> = data
            .iter()
            .filter_map(|(thread_id, rec)| {
                let w = rec.wechat.as_ref()?;
                Some((
                    thread_id.clone(),
                    json!({
                        "threadId": thread_id,
                        "accountId": w.account_id,
                        "userId": w.user_id,
                        "name": w.name,
                        "boundAt": w.bound_at,
                    }),
                ))
            })
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list.into_iter().map(|(_tid, v)| v).collect()
    }

    /// 迁移：把旧 wechat/bindings.json 数组按 threadId 合并进统一记录并删除旧文件
    /// （幂等：重复迁移同一 threadId 只覆盖 wechat 子字段，保留其它字段）。
    fn migrate_legacy(&self, app_dir: &Path) -> Result<(), String> {
        let old = legacy_bindings_path(app_dir);
        let text = fs::read_to_string(&old).map_err(|e| format!("读旧绑定失败: {e}"))?;
        let entries: Vec<Value> = serde_json::from_str(&text).unwrap_or_default();
        {
            let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
            for b in entries {
                let tid = b.get("threadId").and_then(|v| v.as_str()).unwrap_or("");
                if tid.is_empty() {
                    continue;
                }
                let binding = WechatBinding {
                    account_id: b.get("accountId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    user_id: b.get("userId").and_then(|v| v.as_str()).map(str::to_string),
                    name: b.get("name").and_then(|v| v.as_str()).map(str::to_string),
                    bound_at: b.get("boundAt").and_then(|v| v.as_u64()),
                };
                let mut rec = data.get(tid).cloned().unwrap_or_default();
                rec.wechat = Some(binding);
                if !rec.is_empty() {
                    data.insert(tid.to_string(), rec);
                }
            }
        }
        self.persist()?;
        let _ = fs::remove_file(&old);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn set_settings_merges_and_preserves_wechat() {
        let dir = TempDir::new().unwrap();
        let store = SessionStateStore::new(dir.path()).unwrap();
        store.set_wechat("t1", Some(WechatBinding { account_id: "bot-1".into(), ..Default::default() })).unwrap();
        store.set_settings("t1", Some("full-access".into()), Some("deepseek".into()), Some("high".into())).unwrap();
        let rec = store.get("t1").unwrap();
        assert_eq!(rec.permission_mode.as_deref(), Some("full-access"));
        assert_eq!(rec.model.as_deref(), Some("deepseek"));
        assert_eq!(rec.effort.as_deref(), Some("high"));
        assert!(rec.wechat.is_some());
    }

    #[test]
    fn null_model_effort_clears_to_default() {
        let dir = TempDir::new().unwrap();
        let store = SessionStateStore::new(dir.path()).unwrap();
        store.set_settings("t1", Some("ask-for-approval".into()), Some("m".into()), Some("low".into())).unwrap();
        store.set_settings("t1", Some("read-only".into()), None, None).unwrap();
        let rec = store.get("t1").unwrap();
        assert_eq!(rec.permission_mode.as_deref(), Some("read-only"));
        assert!(rec.model.is_none());
        assert!(rec.effort.is_none());
    }

    #[test]
    fn empty_record_is_removed() {
        let dir = TempDir::new().unwrap();
        let store = SessionStateStore::new(dir.path()).unwrap();
        store.set_settings("t1", Some("full-access".into()), None, None).unwrap();
        store.set_settings("t1", None, None, None).unwrap();
        assert!(store.get("t1").is_none());
    }

    #[test]
    fn migrate_from_legacy_bindings() {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path();
        let wechat = app_dir.join("wechat");
        fs::create_dir_all(&wechat).unwrap();
        fs::write(
            wechat.join("bindings.json"),
            r#"[{"threadId":"t1","accountId":"bot-1","userId":"u1","name":null,"boundAt":123}]"#,
        )
        .unwrap();
        let store = SessionStateStore::new(app_dir).unwrap();
        let rec = store.get("t1").unwrap();
        assert_eq!(rec.wechat.as_ref().unwrap().account_id, "bot-1");
        assert_eq!(rec.wechat.as_ref().unwrap().user_id.as_deref(), Some("u1"));
        assert!(!legacy_bindings_path(app_dir).exists());
    }

    #[test]
    fn migrate_again_merges_into_existing_sessions() {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path();
        let wechat = app_dir.join("wechat");
        fs::create_dir_all(&wechat).unwrap();
        // 首次迁移：旧绑定 t1 -> bot-1
        fs::write(
            wechat.join("bindings.json"),
            r#"[{"threadId":"t1","accountId":"bot-1","userId":"u1","name":null,"boundAt":1}]"#,
        )
        .unwrap();
        {
            let store = SessionStateStore::new(app_dir).unwrap();
            assert_eq!(
                store.get("t1").unwrap().wechat.as_ref().unwrap().account_id,
                "bot-1"
            );
            // 记录补充会话设置，验证二次迁移保留
            store
                .set_settings("t1", Some("full-access".into()), Some("m1".into()), None)
                .unwrap();
        }
        assert!(!legacy_bindings_path(app_dir).exists());
        // 模拟降级回旧版：旧版重建 bindings.json（同 t1 改绑 bot-2 + 新线程 t2）
        fs::write(
            wechat.join("bindings.json"),
            r#"[
                {"threadId":"t1","accountId":"bot-2","userId":"u2","name":null,"boundAt":2},
                {"threadId":"t2","accountId":"bot-3","userId":"u3","name":null,"boundAt":3}
            ]"#,
        )
        .unwrap();
        // 再装新版：sessions.json 已存在，仍应合并旧绑定
        let store2 = SessionStateStore::new(app_dir).unwrap();
        let rec1 = store2.get("t1").unwrap();
        assert_eq!(rec1.wechat.as_ref().unwrap().account_id, "bot-2");
        assert_eq!(rec1.permission_mode.as_deref(), Some("full-access"));
        assert_eq!(rec1.model.as_deref(), Some("m1"));
        assert_eq!(
            store2.get("t2").unwrap().wechat.as_ref().unwrap().account_id,
            "bot-3"
        );
        assert!(!legacy_bindings_path(app_dir).exists());
    }

    #[test]
    fn roundtrip_disk() {
        let dir = TempDir::new().unwrap();
        {
            let store = SessionStateStore::new(dir.path()).unwrap();
            store.set_settings("t1", Some("help-me-approve".into()), None, Some("max".into())).unwrap();
        }
        let store = SessionStateStore::new(dir.path()).unwrap();
        let rec = store.get("t1").unwrap();
        assert_eq!(rec.permission_mode.as_deref(), Some("help-me-approve"));
        assert_eq!(rec.effort.as_deref(), Some("max"));
    }
}
