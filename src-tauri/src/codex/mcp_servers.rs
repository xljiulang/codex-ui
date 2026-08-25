//! MCP 服务器配置读写：管理 `config.toml` 的 `[mcp_servers.*]`。
//!
//! 与 model_config 同一套 CODEX_HOME 定位与原子写策略；读取时 config 缺失会
//! 自动创建空文件，无法解析时报错（提示用户先用编辑器修复）。保存为整表同步：
//! 支持 STDIO（command / args / env）与 Streamable HTTP（url / http_headers /
//! bearer_token_env_var）两种传输；upsert 已知字段并保留服务器表内未知字段与
//! 其余 TOML 内容，删除列表外的服务器。传输类型以是否配置 url 判定。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

use super::model_config::{atomic_write, codex_home, config_path_in};

/// 单个 MCP 服务器条目（env 用有序键值对，便于 UI 增删）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    /// [mcp_servers.<name>] 表名标识。
    pub name: String,
    /// STDIO 启动命令（http 服务器为空）。
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<McpEnvEntry>,
    /// Streamable HTTP 地址（stdio 服务器为空）。
    pub url: String,
    /// 静态 HTTP 请求头（http_headers）。
    pub headers: Vec<McpEnvEntry>,
    /// Bearer 令牌来源环境变量名。
    pub bearer_token_env_var: String,
}

/// env 表中的单个键值对。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpEnvEntry {
    pub key: String,
    pub value: String,
}

/// `mcp_servers_read` 的返回结构。
#[derive(Debug, Clone, Serialize)]
pub struct McpServersState {
    pub servers: Vec<McpServerInfo>,
}

/// `mcp_servers_save` 的输入：全部服务器（列表外的将被删除）。
#[derive(Debug, Clone, Deserialize)]
pub struct McpServersEdit {
    pub servers: Vec<McpServerInfo>,
}

/// 读取 MCP 服务器配置（真实 CODEX_HOME）。
pub fn read_state() -> Result<McpServersState, String> {
    read_state_in(&codex_home()?)
}

fn read_state_in(home: &Path) -> Result<McpServersState, String> {
    let config_path = config_path_in(home);
    if !config_path.is_file() {
        // 不存在则创建：保证设置页可直接编辑
        atomic_write(&config_path, "")?;
    }
    let text = fs::read_to_string(&config_path)
        .map_err(|e| format!("读取 config.toml 失败: {e}"))?;
    let doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("config.toml 解析失败，无法读取 MCP 配置（可先用编辑器修复）: {e}"))?;

    let mut servers = Vec::new();
    if let Some(table) = doc.get("mcp_servers").and_then(Item::as_table) {
        for (name, item) in table.iter() {
            let Some(t) = item.as_table() else {
                continue;
            };
            servers.push(McpServerInfo {
                name: name.to_string(),
                command: t
                    .get("command")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
                args: t
                    .get("args")
                    .and_then(Item::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                env: read_kv_table(t.get("env")),
                url: t
                    .get("url")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
                headers: read_kv_table(t.get("http_headers")),
                bearer_token_env_var: t
                    .get("bearer_token_env_var")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Ok(McpServersState { servers })
}

/// 读取 env / http_headers 等键值表（兼容正式表与内联表）。
fn read_kv_table(item: Option<&Item>) -> Vec<McpEnvEntry> {
    let Some(item) = item else {
        return Vec::new();
    };
    let pairs: Vec<(String, String)> = if let Some(table) = item.as_table() {
        table
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("").to_string()))
            .collect()
    } else if let Item::Value(v) = item {
        v.as_inline_table()
            .map(|it| {
                it.iter()
                    .map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    pairs
        .into_iter()
        .map(|(key, value)| McpEnvEntry { key, value })
        .collect()
}

/// 保存 MCP 服务器配置（真实 CODEX_HOME）：整表同步，其余 TOML 内容保留。
pub fn save(edit: &McpServersEdit) -> Result<(), String> {
    save_in(&codex_home()?, edit)
}

fn save_in(home: &Path, edit: &McpServersEdit) -> Result<(), String> {
    let path = config_path_in(home);
    let original = if path.is_file() {
        fs::read_to_string(&path).map_err(|e| format!("读取 config.toml 失败: {e}"))?
    } else {
        String::new()
    };
    let mut doc: DocumentMut = original.parse().map_err(|e| {
        format!("config.toml 解析失败，无法保存 MCP 配置（可先用编辑器修复）: {e}")
    })?;

    // 校验：名称合法且不重复；stdio 必须 command 非空，http 必须 url 以 http(s) 开头；
    // 键值对（env / 请求头）键非空且不重复。
    let mut seen: Vec<String> = Vec::new();
    for s in &edit.servers {
        let name = s.name.trim();
        if name.is_empty() {
            return Err("MCP 服务器名称不能为空".to_string());
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "MCP 服务器名称「{name}」只能包含字母、数字、下划线与连字符"
            ));
        }
        if seen.contains(&name.to_string()) {
            return Err(format!("MCP 服务器名称「{name}」重复"));
        }
        seen.push(name.to_string());
        let is_http = !s.url.trim().is_empty();
        if is_http {
            let url = s.url.trim();
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(format!(
                    "MCP 服务器「{name}」的 url 必须以 http:// 或 https:// 开头"
                ));
            }
        } else if s.command.trim().is_empty() {
            return Err(format!("MCP 服务器「{name}」缺少 command"));
        }
        let kv_label = if is_http { "请求头" } else { "env" };
        let mut kv_keys: Vec<String> = Vec::new();
        for e in (if is_http { &s.headers } else { &s.env }).iter() {
            let key = e.key.trim();
            if key.is_empty() {
                return Err(format!("MCP 服务器「{name}」存在空的 {kv_label} 键"));
            }
            if kv_keys.contains(&key.to_string()) {
                return Err(format!("MCP 服务器「{name}」{kv_label}键「{key}」重复"));
            }
            kv_keys.push(key.to_string());
        }
    }

    // 整表同步：upsert 已知字段（保留未知字段），删除列表外的服务器。
    let servers_table = doc
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()));
    let servers_table = servers_table
        .as_table_mut()
        .ok_or_else(|| "config.toml 中 mcp_servers 必须是表".to_string())?;
    for s in &edit.servers {
        let name = s.name.trim();
        let entry = servers_table
            .entry(name)
            .or_insert(Item::Table(Table::new()));
        let table = entry
            .as_table_mut()
            .ok_or_else(|| format!("mcp_servers.{name} 必须是表"))?;
        let is_http = !s.url.trim().is_empty();
        if is_http {
            set_or_remove(table, "url", s.url.trim());
            set_or_remove(table, "bearer_token_env_var", s.bearer_token_env_var.trim());
            set_kv_table(table, "http_headers", &s.headers);
            // 清理 stdio 侧字段
            table.remove("command");
            table.remove("args");
            table.remove("env");
        } else {
            set_or_remove(table, "command", s.command.trim());
            let args: Vec<String> = s
                .args
                .iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect();
            if args.is_empty() {
                table.remove("args");
            } else {
                table.insert(
                    "args",
                    Item::Value(Value::Array(Array::from_iter(
                        args.iter().map(|a| Value::from(a.as_str())),
                    ))),
                );
            }
            set_kv_table(table, "env", &s.env);
            // 清理 http 侧字段
            table.remove("url");
            table.remove("http_headers");
            table.remove("bearer_token_env_var");
        }
    }
    let existing_names: Vec<String> = servers_table
        .iter()
        .map(|(k, _)| k.to_string())
        .collect();
    for name in existing_names {
        if !seen.contains(&name) {
            servers_table.remove(&name);
        }
    }

    atomic_write(&path, &doc.to_string())
}

/// 重建键值表（env / http_headers）：先移除编辑外的键，再 upsert；空表整体移除。
fn set_kv_table(table: &mut Table, key: &str, entries: &[McpEnvEntry]) {
    let item = table.entry(key).or_insert(Item::Table(Table::new()));
    if item.as_table_mut().is_none() {
        *item = Item::Table(Table::new());
    }
    let kv = item
        .as_table_mut()
        .expect("已保证为正式表");
    let wanted: Vec<String> = entries
        .iter()
        .map(|e| e.key.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    let existing: Vec<String> = kv.iter().map(|(k, _)| k.to_string()).collect();
    for k in existing {
        if !wanted.contains(&k) {
            kv.remove(&k);
        }
    }
    for e in entries {
        let k = e.key.trim();
        if !k.is_empty() {
            kv.insert(k, Item::Value(Value::from(e.value.as_str())));
        }
    }
    if kv.is_empty() {
        table.remove(key);
    }
}

/// 写入字符串值；空串时移除该键。
fn set_or_remove(table: &mut Table, key: &str, val: &str) {
    if val.is_empty() {
        table.remove(key);
    } else {
        table.insert(key, value(val));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_stdio(name: &str, command: &str) -> McpServerInfo {
        McpServerInfo {
            name: name.into(),
            command: command.into(),
            args: vec![],
            env: vec![],
            url: String::new(),
            headers: vec![],
            bearer_token_env_var: String::new(),
        }
    }

    fn make_http(name: &str, url: &str) -> McpServerInfo {
        McpServerInfo {
            name: name.into(),
            command: String::new(),
            args: vec![],
            env: vec![],
            url: url.into(),
            headers: vec![],
            bearer_token_env_var: String::new(),
        }
    }

    #[test]
    fn read_parses_command_args_and_env() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"[mcp_servers.filesystem]
command = "npx"
args = ["-y", "mcp-server"]
env = { "API_KEY" = "abc", "BASE" = "https://x" }
"#,
        )
        .unwrap();

        let state = read_state_in(dir.path()).unwrap();
        assert_eq!(state.servers.len(), 1);
        let s = &state.servers[0];
        assert_eq!(s.name, "filesystem");
        assert_eq!(s.command, "npx");
        assert_eq!(s.args, vec!["-y", "mcp-server"]);
        assert_eq!(s.env.len(), 2);
        assert_eq!(s.env[0].key, "API_KEY");
        assert_eq!(s.env[0].value, "abc");
        assert_eq!(s.env[1].key, "BASE");
        assert_eq!(s.env[1].value, "https://x");
    }

    #[test]
    fn read_creates_missing_config_with_empty_servers() {
        let dir = TempDir::new().unwrap();
        let state = read_state_in(dir.path()).unwrap();
        assert!(state.servers.is_empty());
        assert!(config_path_in(dir.path()).is_file());
        assert_eq!(fs::read_to_string(config_path_in(dir.path())).unwrap(), "");
    }

    #[test]
    fn read_rejects_unparseable_config() {
        let dir = TempDir::new().unwrap();
        fs::write(config_path_in(dir.path()), "not valid toml = = [").unwrap();
        assert!(read_state_in(dir.path()).is_err());
    }

    #[test]
    fn save_upserts_preserving_unknown_fields_and_removes_deleted() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"instructions = "保留"
[mcp_servers.a]
command = "old"
timeout = 30
env = { "K1" = "v1", "K2" = "v2" }
[mcp_servers.b]
command = "b"
"#,
        )
        .unwrap();

        let edit = McpServersEdit {
            servers: vec![McpServerInfo {
                name: "a".into(),
                command: "npx".into(),
                args: vec!["-y".into(), "server".into()],
                env: vec![
                    McpEnvEntry {
                        key: "K1".into(),
                        value: "new".into(),
                    },
                    McpEnvEntry {
                        key: "K3".into(),
                        value: "v3".into(),
                    },
                ],
                url: String::new(),
                headers: vec![],
                bearer_token_env_var: String::new(),
            }],
        };
        save_in(dir.path(), &edit).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(
            doc.get("instructions").unwrap().as_str(),
            Some("保留")
        );
        let mcp = doc.get("mcp_servers").unwrap().as_table().unwrap();
        assert!(mcp.get("b").is_none());
        let a = mcp.get("a").unwrap().as_table().unwrap();
        assert_eq!(a.get("command").unwrap().as_str(), Some("npx"));
        // 未知字段保留
        assert_eq!(a.get("timeout").unwrap().as_integer(), Some(30));
        let args = a.get("args").unwrap().as_array().unwrap();
        assert_eq!(args.len(), 2);
        let env = a.get("env").unwrap().as_table().unwrap();
        assert_eq!(env.get("K1").unwrap().as_str(), Some("new"));
        assert!(env.get("K2").is_none());
        assert_eq!(env.get("K3").unwrap().as_str(), Some("v3"));
    }

    #[test]
    fn save_removes_env_table_when_empty() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"[mcp_servers.a]
command = "npx"
env = { "K1" = "v1" }
"#,
        )
        .unwrap();

        let edit = McpServersEdit {
            servers: vec![make_stdio("a", "npx")],
        };
        save_in(dir.path(), &edit).unwrap();
        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        let a = doc
            .get("mcp_servers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("a")
            .unwrap()
            .as_table()
            .unwrap();
        assert!(a.get("env").is_none());
    }

    #[test]
    fn save_rejects_invalid_inputs() {
        let dir = TempDir::new().unwrap();
        fs::write(config_path_in(dir.path()), "").unwrap();

        let base = make_stdio("a", "npx");
        let mut bad_name = base.clone();
        bad_name.name = "a b".into();
        assert!(save_in(dir.path(), &McpServersEdit { servers: vec![bad_name] }).is_err());

        let mut empty_name = base.clone();
        empty_name.name = "".into();
        assert!(save_in(dir.path(), &McpServersEdit { servers: vec![empty_name] }).is_err());

        let mut empty_cmd = base.clone();
        empty_cmd.command = "".into();
        assert!(save_in(dir.path(), &McpServersEdit { servers: vec![empty_cmd] }).is_err());

        let mut dup_env = base.clone();
        dup_env.env = vec![
            McpEnvEntry { key: "K".into(), value: "1".into() },
            McpEnvEntry { key: "K".into(), value: "2".into() },
        ];
        assert!(save_in(dir.path(), &McpServersEdit { servers: vec![dup_env] }).is_err());

        let duplicate = McpServersEdit {
            servers: vec![base.clone(), base],
        };
        assert!(save_in(dir.path(), &duplicate).is_err());
    }

    #[test]
    fn read_parses_http_server_fields() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"[mcp_servers.remote]
url = "https://example.com/mcp"
http_headers = { "Authorization" = "Bearer x", "X-Custom" = "1" }
bearer_token_env_var = "MY_MCP_TOKEN"
"#,
        )
        .unwrap();

        let state = read_state_in(dir.path()).unwrap();
        let s = &state.servers[0];
        assert_eq!(s.name, "remote");
        assert_eq!(s.url, "https://example.com/mcp");
        assert_eq!(s.headers.len(), 2);
        assert_eq!(s.headers[0].key, "Authorization");
        assert_eq!(s.headers[0].value, "Bearer x");
        assert_eq!(s.bearer_token_env_var, "MY_MCP_TOKEN");
        assert_eq!(s.command, "");
    }

    #[test]
    fn save_http_writes_fields_and_cleans_stdio_side() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"[mcp_servers.a]
command = "old"
args = ["-y"]
env = { "K" = "v" }
url = "http://old.example.com/mcp"
"#,
        )
        .unwrap();

        let mut s = make_http("a", "https://example.com/mcp");
        s.headers = vec![McpEnvEntry {
            key: "Authorization".into(),
            value: "Bearer t".into(),
        }];
        s.bearer_token_env_var = "TOKEN".into();
        save_in(dir.path(), &McpServersEdit { servers: vec![s] }).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        let a = doc
            .get("mcp_servers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("a")
            .unwrap()
            .as_table()
            .unwrap();
        assert_eq!(
            a.get("url").unwrap().as_str(),
            Some("https://example.com/mcp")
        );
        assert_eq!(
            a.get("bearer_token_env_var").unwrap().as_str(),
            Some("TOKEN")
        );
        let headers = a.get("http_headers").unwrap().as_table().unwrap();
        assert_eq!(
            headers.get("Authorization").unwrap().as_str(),
            Some("Bearer t")
        );
        assert!(a.get("command").is_none());
        assert!(a.get("args").is_none());
        assert!(a.get("env").is_none());
    }

    #[test]
    fn save_stdio_switches_back_and_cleans_http_side() {
        let dir = TempDir::new().unwrap();
        fs::write(
            config_path_in(dir.path()),
            r#"[mcp_servers.a]
url = "https://example.com/mcp"
http_headers = { "Authorization" = "Bearer t" }
bearer_token_env_var = "TOKEN"
"#,
        )
        .unwrap();

        let mut s = make_stdio("a", "npx");
        s.args = vec!["-y".into(), "server".into()];
        save_in(dir.path(), &McpServersEdit { servers: vec![s] }).unwrap();

        let text = fs::read_to_string(config_path_in(dir.path())).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        let a = doc
            .get("mcp_servers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("a")
            .unwrap()
            .as_table()
            .unwrap();
        assert_eq!(a.get("command").unwrap().as_str(), Some("npx"));
        assert!(a.get("url").is_none());
        assert!(a.get("http_headers").is_none());
        assert!(a.get("bearer_token_env_var").is_none());
    }

    #[test]
    fn save_rejects_invalid_http_inputs() {
        let dir = TempDir::new().unwrap();
        fs::write(config_path_in(dir.path()), "").unwrap();

        let mut no_url = make_http("a", "");
        no_url.command = "npx".into(); // 有 command 但无 url → 视为 stdio，需 command 非空，应通过
        assert!(save_in(dir.path(), &McpServersEdit { servers: vec![no_url] }).is_ok());

        let bad_scheme = make_http("a", "ftp://example.com/mcp");
        assert!(save_in(dir.path(), &McpServersEdit { servers: vec![bad_scheme] }).is_err());

        let mut empty_header_key = make_http("a", "https://example.com/mcp");
        empty_header_key.headers = vec![McpEnvEntry {
            key: "".into(),
            value: "v".into(),
        }];
        assert!(save_in(dir.path(), &McpServersEdit { servers: vec![empty_header_key] }).is_err());

        let mut dup_header = make_http("a", "https://example.com/mcp");
        dup_header.headers = vec![
            McpEnvEntry { key: "H".into(), value: "1".into() },
            McpEnvEntry { key: "H".into(), value: "2".into() },
        ];
        assert!(save_in(dir.path(), &McpServersEdit { servers: vec![dup_header] }).is_err());
    }

    #[test]
    fn save_rejects_unparseable_config() {
        let dir = TempDir::new().unwrap();
        fs::write(config_path_in(dir.path()), "not valid toml = = [").unwrap();
        let edit = McpServersEdit { servers: vec![] };
        assert!(save_in(dir.path(), &edit).is_err());
    }
}
