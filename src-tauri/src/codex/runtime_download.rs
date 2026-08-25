//! codex-primary-runtime 的按需下载/升级，并在关键步骤写日志。
//!
//! 安装包不再捆绑 `codex-primary-runtime`；codex-ui 启动后由后台任务判断是否需要：
//! - `%USERPROFILE%\.cache\codex-runtimes\codex-primary-runtime\runtime.json` 不存在（未安装），或
//! - 存在但其 `bundleVersion` 旧于 [`RUNTIME_DOWNLOAD_VERSION`]（升级），
//! 才会从 CDN 下载 `.tar.gz` 并解压到 canonical 目录。下载支持断点续传（HTTP Range），
//! 失败/中断保留部分文件供下次续传。关键步骤通过 `CodexServer::push_log` 记录到
//! 应用内 `logs` 与日志文件（事件 `server-log`）。

use std::cmp::Ordering;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::codex::app_server::CodexServer;
use crate::codex::bundled::{BundleSpec, materialize};

/// 期望安装的运行时版本（含 poppler/PDF 工具链）。
pub(crate) const RUNTIME_DOWNLOAD_VERSION: &str = "26.819.11345";

/// 本地运行时安装状态，用于日志与下载判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeStatus {
    /// `runtime.json` 不存在，视为未安装。
    Missing,
    /// 已安装但 `bundleVersion` 旧于目标版本（携带当前版本号）。
    Older(String),
    /// `runtime.json` 存在但无法解析 `bundleVersion`，视为旧版。
    Unparsable,
    /// 已安装且版本不低于目标版本（携带当前版本号）。
    Current(String),
}

/// CDN 上该版本运行时的 `.tar.gz` 地址。
pub(crate) fn runtime_url(version: &str) -> String {
    format!(
        "https://persistent.oaistatic.com/codex-primary-runtime/{version}/codex-primary-runtime-win32-x64-{version}.tar.gz"
    )
}

/// 本机缓存/断点续传用的归档文件：`<dest>/<top>.tar.gz`。
/// 对于运行时为 `%USERPROFILE%\.cache\codex-runtimes\codex-primary-runtime.tar.gz`。
pub(crate) fn archive_cache_path(spec: &BundleSpec) -> PathBuf {
    spec.dest.join(format!("{}.tar.gz", spec.top))
}

/// 把版本号按 `.` 拆段逐段比较（数值），如 `26.819.11345`。
fn compare_versions(a: &str, b: &str) -> Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|seg| seg.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (av, bv) = (parse(a), parse(b));
    let len = av.len().max(bv.len());
    for i in 0..len {
        let x = if i < av.len() { av[i] } else { 0 };
        let y = if i < bv.len() { bv[i] } else { 0 };
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// 判断本地运行时的安装状态（纯函数，便于测试）。
pub(crate) fn runtime_status(target: &Path) -> RuntimeStatus {
    let json_path = target.join("runtime.json");
    let text = match std::fs::read_to_string(&json_path) {
        Ok(t) => t,
        Err(_) => return RuntimeStatus::Missing,
    };
    let installed = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("bundleVersion").and_then(|x| x.as_str()).map(str::to_owned));
    match installed {
        Some(ver) if compare_versions(&ver, RUNTIME_DOWNLOAD_VERSION) == Ordering::Less => {
            RuntimeStatus::Older(ver)
        }
        Some(ver) => RuntimeStatus::Current(ver),
        None => RuntimeStatus::Unparsable,
    }
}

/// 若需要下载则返回可读原因（用于日志），否则返回 `None`。
pub(crate) fn runtime_reason(target: &Path) -> Option<String> {
    match runtime_status(target) {
        RuntimeStatus::Missing => Some("runtime.json 不存在（未安装）".to_string()),
        RuntimeStatus::Older(ver) => {
            Some(format!("bundleVersion {ver} 旧于 {RUNTIME_DOWNLOAD_VERSION}"))
        }
        RuntimeStatus::Unparsable => Some("runtime.json 无法解析 bundleVersion".to_string()),
        RuntimeStatus::Current(_) => None,
    }
}

/// 文件是否为 gzip（magic `1F 8B`）。
fn is_gzip(path: &Path) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut magic = [0u8; 2];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    magic == [0x1F, 0x8B]
}

/// 带断点续传地下载到 `dest`，并在关键步骤写日志：
/// - 已有部分文件则带 `Range: bytes=<len>-` 请求；响应 206 追加、200 忽略 Range 则截断重写；
/// - 响应 416 说明本地已 >= 完整长度，视为已完成，不再写；
/// - 中断/失败保留部分文件（不删除），供下次续传；每 ~64 MiB 记录一次进度。
pub(crate) async fn download_with_resume(
    url: &str,
    dest: &Path,
    server: &CodexServer,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let existing = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    let mode = if existing > 0 {
        format!("从 {existing} 字节续传")
    } else {
        "全新下载".to_string()
    };
    server
        .push_log("info", format!("下载 codex-primary-runtime：{mode} · {url} → {}", dest.display()))
        .await;

    let mut req = client.get(url);
    if existing > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={existing}-"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {e}"))?;
    let status = resp.status();

    match status {
        status if status == reqwest::StatusCode::PARTIAL_CONTENT => {
            server.push_log("info", "HTTP 206：断点续传".to_string()).await;
            let mut file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(dest)
                .await
                .map_err(|e| format!("打开缓存文件失败 {}: {e}", dest.display()))?;
            let mut total = existing;
            write_body(resp, &mut file, server, &mut total).await?;
            file.flush().await.map_err(|e| format!("刷新缓存文件失败: {e}"))?;
            server
                .push_log("info", format!("下载完成，共 {total} 字节：{}", dest.display()))
                .await;
        }
        status if status == reqwest::StatusCode::OK => {
            let note = if existing > 0 {
                "服务端忽略 Range，重新下载"
            } else {
                "全新下载"
            };
            server.push_log("info", format!("HTTP 200：{note}")).await;
            let mut file = tokio::fs::File::create(dest)
                .await
                .map_err(|e| format!("创建缓存文件失败 {}: {e}", dest.display()))?;
            let mut total = 0u64;
            write_body(resp, &mut file, server, &mut total).await?;
            file.flush().await.map_err(|e| format!("刷新缓存文件失败: {e}"))?;
            server
                .push_log("info", format!("下载完成，共 {total} 字节：{}", dest.display()))
                .await;
        }
        status if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE => {
            server
                .push_log("info", "HTTP 416：本地已完整，无需下载".to_string())
                .await;
        }
        other => {
            server
                .push_log("warn", format!("下载失败，HTTP 状态码 {other}"))
                .await;
            return Err(format!("下载失败，HTTP 状态码 {other}"));
        }
    }
    Ok(())
}

/// 把响应体逐块写入文件（断点续传时已正确指向目标位置），并每 ~64 MiB 记一次进度。
async fn write_body(
    resp: reqwest::Response,
    file: &mut tokio::fs::File,
    server: &CodexServer,
    total: &mut u64,
) -> Result<(), String> {
    const PROGRESS_INTERVAL: u64 = 64 * 1024 * 1024;
    let mut last_logged: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载流读取失败: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("写入缓存文件失败: {e}"))?;
        *total += chunk.len() as u64;
        if *total - last_logged >= PROGRESS_INTERVAL {
            last_logged = *total;
            server
                .push_log("info", format!("下载进度：已下载 {total} 字节"))
                .await;
        }
    }
    Ok(())
}

/// 确保下载并安装运行时到 `spec.dest/<top>`；返回前完成目标目录落地。
/// 关键步骤（下载、校验、解压、清理、失败）均写日志。
pub(crate) async fn ensure(spec: &BundleSpec, server: &CodexServer) -> Result<(), String> {
    let url = runtime_url(RUNTIME_DOWNLOAD_VERSION);
    let cache = archive_cache_path(spec);
    download_with_resume(&url, &cache, server).await?;

    if !is_gzip(&cache) {
        let _ = std::fs::remove_file(&cache);
        server
            .push_log("warn", "下载的运行时归档 gzip 校验失败，已删除缓存，下次重下".to_string())
            .await;
        return Err("下载的运行时归档不是有效的 gzip 文件".into());
    }
    server.push_log("info", "运行时归档 gzip 校验通过".to_string()).await;

    let target = spec.dest.join(spec.top);
    server
        .push_log("info", format!("开始解压/安装 codex-primary-runtime 到 {}", target.display()))
        .await;
    if let Err(e) = materialize(&cache, spec, &target, true) {
        let _ = std::fs::remove_file(&cache);
        server
            .push_log("warn", format!("解压运行时归档失败，已删除缓存：{e}"))
            .await;
        return Err(format!("解压运行时归档失败: {e}"));
    }
    let _ = std::fs::remove_file(&cache);
    server
        .push_log("info", format!("codex-primary-runtime 已安装到 {}，缓存已清理", target.display()))
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn write_runtime_json(dir: &Path, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("runtime.json"), body).unwrap();
    }

    #[test]
    fn status_missing_when_no_runtime_json() {
        let dir = TempDir::new().unwrap();
        assert_eq!(runtime_status(dir.path()), RuntimeStatus::Missing);
        assert!(runtime_reason(dir.path()).is_some());
        assert_eq!(
            runtime_reason(dir.path()).unwrap(),
            "runtime.json 不存在（未安装）"
        );
    }

    #[test]
    fn status_current_when_version_is_target() {
        let dir = TempDir::new().unwrap();
        write_runtime_json(dir.path(), r#"{"bundleVersion":"26.819.11345"}"#);
        assert_eq!(
            runtime_status(dir.path()),
            RuntimeStatus::Current("26.819.11345".to_string())
        );
        assert!(runtime_reason(dir.path()).is_none());
    }

    #[test]
    fn status_older_when_version_below_target() {
        let dir = TempDir::new().unwrap();
        write_runtime_json(dir.path(), r#"{"bundleVersion":"26.426.12240"}"#);
        assert_eq!(
            runtime_status(dir.path()),
            RuntimeStatus::Older("26.426.12240".to_string())
        );
        assert!(runtime_reason(dir.path()).is_some());
        assert!(runtime_reason(dir.path()).unwrap().contains("26.426.12240"));
    }

    #[test]
    fn status_unparsable_when_json_invalid_or_missing_key() {
        let dir = TempDir::new().unwrap();
        write_runtime_json(dir.path(), "not json");
        assert_eq!(runtime_status(dir.path()), RuntimeStatus::Unparsable);
        assert!(runtime_reason(dir.path()).is_some());

        let dir2 = TempDir::new().unwrap();
        write_runtime_json(dir2.path(), r#"{"nodeVersion":"v24.19.0"}"#);
        assert_eq!(runtime_status(dir2.path()), RuntimeStatus::Unparsable);
        assert!(runtime_reason(dir2.path()).is_some());
    }

    #[test]
    fn compare_versions_numeric_segments() {
        assert_eq!(compare_versions("26.819.11345", "26.819.11345"), Ordering::Equal);
        assert_eq!(compare_versions("26.426.12240", "26.819.11345"), Ordering::Less);
        assert_eq!(compare_versions("26.819.11346", "26.819.11345"), Ordering::Greater);
        assert_eq!(compare_versions("26.819", "26.819.0"), Ordering::Equal);
    }
}
