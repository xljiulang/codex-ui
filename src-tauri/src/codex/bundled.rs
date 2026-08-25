//! 内置插件市场的启动期解压。
//!
//! 安装器只随包提供 `openai-bundled.tar.gz`（放到 `<应用目录>/marketplaces/`）；
//! `codex-primary-runtime` 不再随包，改由 [`crate::codex::runtime_download`] 在启动后台按需
//! 下载/升级。codex-ui 启动时先在后台把随包归档（`openai-bundled`）解压到 codex 认可的
//! canonical 位置并通知登记逻辑，随后再异步处理运行时，二者互不阻塞。
//!
//! 为避免「标记先落、内容缺」的半成品：物化采用「临时目录完整解压 → 整体改名」，目标目录
//! 只会由一次完整的 `rename` 产生。
//! `openai-bundled` 在目标已存在且非空时，比对磁盘与随包归档的 `.materialization-key`：
//! 磁盘缺失或 `appVersion` 低于归档版时以 `replace=true` 重新物化（仅升级、绝不降级）；
//! 其余情况视为已就位。运行时升级场景始终允许 `replace=true` 覆盖非空目标。

use std::cmp::Ordering;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::codex::app_server::{CodexServer, app_exe_dir};
use crate::codex::model_config;
use crate::codex::runtime_download;

/// 内置市场归档文件名（均为 `.tar.gz`）。
pub(crate) const RUNTIME_ARCHIVE: &str = "codex-primary-runtime.tar.gz";
pub(crate) const BUNDLED_ARCHIVE: &str = "openai-bundled.tar.gz";

/// 单个内置市场的解压参数。
#[derive(Debug, Clone)]
pub(crate) struct BundleSpec {
    /// 归档文件名（位于 `<app>/marketplaces/` 下）。
    pub archive_name: &'static str,
    /// 解压目标根目录（等价于 `tar -C`，归档顶层目录会落到该根下）。
    pub dest: PathBuf,
    /// 归档顶层目录名（解压后的目标目录 = `dest/<top>`）。
    pub top: &'static str,
}

/// `bootstrap_one` 的单市场处理结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BootOutcome {
    /// 目标目录已存在且非空，无需解压。
    AlreadyPresent,
    /// 归档未随包提供（开发/裸跑等），跳过。
    ArchiveMissing,
    /// 已临时解压并整体改名到目标目录。
    Extracted,
    /// 目标存在但磁盘 `.materialization-key` 缺失或 `appVersion` 低于归档版，已按归档重新物化。
    Updated,
    /// 解压失败（目标未落，下次启动重试）。
    Failed(String),
}

/// 应用自身的 `marketplaces` 目录：`<current_exe 目录>/marketplaces`。
pub(crate) fn archive_dir() -> Option<PathBuf> {
    app_exe_dir().map(|d| d.join("marketplaces"))
}

/// 依据进程环境解析指定内置市场的解压参数；无法定位 CODEX_HOME/%USERPROFILE% 时返回 None。
pub(crate) fn bundle_spec(name: &str) -> Option<BundleSpec> {
    let home = model_config::codex_home().ok()?;
    let user = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    bundle_spec_in(&home, &user, name)
}

/// 纯函数：按内置市场名解析解压参数（便于测试）。
/// - openai-bundled -> 归档 `openai-bundled.tar.gz`，解压到 `<CODEX_HOME>/.tmp/bundled-marketplaces`
/// - openai-primary-runtime -> 归档 `codex-primary-runtime.tar.gz`，解压到 `%USERPROFILE%\.cache\codex-runtimes`
pub(crate) fn bundle_spec_in(
    codex_home: &Path,
    userprofile: &Path,
    name: &str,
) -> Option<BundleSpec> {
    match name {
        "openai-bundled" => Some(BundleSpec {
            archive_name: BUNDLED_ARCHIVE,
            dest: codex_home.join(".tmp").join("bundled-marketplaces"),
            top: "openai-bundled",
        }),
        "openai-primary-runtime" => Some(BundleSpec {
            archive_name: RUNTIME_ARCHIVE,
            dest: userprofile.join(".cache").join("codex-runtimes"),
            top: "codex-primary-runtime",
        }),
        _ => None,
    }
}

/// 判断目录是否为空（未创建/不可读按非空处理，避免误删不可读目录）。
fn is_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false)
}

/// 目标目录是否需要（重新）物化：不存在、或存在但为空目录。
/// 已存在且非空（或不是目录）视为已就位。
fn target_needs_materialize(target: &Path) -> bool {
    match std::fs::metadata(target) {
        Ok(m) if m.is_dir() => is_empty_dir(target),
        Ok(_) => false,
        Err(_) => true,
    }
}

/// 单市场解压决策（纯函数，便于测试）：目标缺失/为空则物化；目标已存在且非空时按
/// `.materialization-key` 版本判断——磁盘缺失或 `appVersion` 低于归档版则 `replace=true`
/// 重新物化（仅升级、绝不降级），否则视为已就位；归档缺失时目标缺失返回 `ArchiveMissing`、
/// 目标存在返回 `AlreadyPresent`（无法更新旧副本）。
pub(crate) fn bootstrap_one(spec: &BundleSpec, archive_dir: &Path) -> BootOutcome {
    let target = spec.dest.join(spec.top);
    let archive = archive_dir.join(spec.archive_name);
    if !target_needs_materialize(&target) {
        // 目标已存在且非空：仅当能确认归档有更新的版本时才升级。
        let Some(archive_version) = read_archive_app_version(&archive, spec.top) else {
            return BootOutcome::AlreadyPresent;
        };
        match read_installed_app_version(&target) {
            Some(installed)
                if runtime_download::compare_versions(&installed, &archive_version) != Ordering::Less =>
            {
                return BootOutcome::AlreadyPresent;
            }
            _ => {
                return match materialize(&archive, spec, &target, true) {
                    Ok(()) => BootOutcome::Updated,
                    Err(e) => BootOutcome::Failed(e),
                };
            }
        }
    }
    if !archive.is_file() {
        return BootOutcome::ArchiveMissing;
    }
    match materialize(&archive, spec, &target, false) {
        Ok(()) => BootOutcome::Extracted,
        Err(e) => BootOutcome::Failed(e),
    }
}

/// 读取磁盘上 `<target>/.materialization-key` 的 `appVersion`；缺失/非法 JSON/无该键返回 None。
fn read_installed_app_version(target: &Path) -> Option<String> {
    read_app_version_from_path(&target.join(".materialization-key"))
}

/// 读取单个 `.materialization-key` 文件里的 `appVersion` 字段。
fn read_app_version_from_path(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("appVersion")?
        .as_str()
        .map(str::to_owned)
}

/// 读取随包归档内 `<top>/.materialization-key` 的 `appVersion`（归档为 `.tar.gz` gzip）。
/// 归档缺失/不可读/无该条目/无 `appVersion` 键均返回 None。
fn read_archive_app_version(archive: &Path, top: &str) -> Option<String> {
    let file = std::fs::File::open(archive).ok()?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut entries = archive.entries().ok()?;
    let wanted = format!("{top}/.materialization-key");
    while let Some(Ok(entry)) = entries.next() {
        let is_wanted = match entry.path() {
            Ok(path) => path.as_ref() == Path::new(&wanted),
            Err(_) => false,
        };
        if is_wanted {
            let mut content = String::new();
            let mut entry = entry;
            entry.read_to_string(&mut content).ok()?;
            return serde_json::from_str::<serde_json::Value>(&content)
                .ok()?
                .get("appVersion")?
                .as_str()
                .map(str::to_owned);
        }
    }
    None
}

/// 原子物化：先完整解压到 `dest/.codex-ui-extract-<top>`，校验后整体改名到目标目录。
/// `replace=true` 时允许覆盖非空目标（运行时升级）；否则目标非空即放弃。
/// 任一步失败只清理临时目录，不落目标，交由下次启动重试。
pub(crate) fn materialize(
    archive: &Path,
    spec: &BundleSpec,
    target: &Path,
    replace: bool,
) -> Result<(), String> {
    std::fs::create_dir_all(&spec.dest)
        .map_err(|e| format!("创建目标根失败 {}: {e}", spec.dest.display()))?;
    let staging = spec.dest.join(format!(".codex-ui-extract-{}", spec.top));
    // 清理上次失败可能留下的临时目录；`NotFound`（首次/已无残留）视为成功。
    match std::fs::remove_dir_all(&staging) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("清理暂存目录失败 {}: {e}", staging.display())),
    }
    if let Err(e) = extract_tar(archive, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let staged_top = staging.join(spec.top);
    if !staged_top.is_dir() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("归档缺少顶层目录: {}", spec.top));
    }
    // 提交：目标为空（占位）时先移除空目录，再整体改名（同卷，原子）。
    if target.exists() {
        if is_empty_dir(target) {
            std::fs::remove_dir(target).map_err(|e| {
                let _ = std::fs::remove_dir_all(&staging);
                format!("移除空目标目录失败 {}: {e}", target.display())
            })?;
        } else if replace {
            // 升级：替换旧运行时（codex 托管数据，删除后重物化）。
            std::fs::remove_dir_all(target).map_err(|e| {
                let _ = std::fs::remove_dir_all(&staging);
                format!("移除旧目标目录失败 {}: {e}", target.display())
            })?;
        } else {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!(
                "目标目录已被写入内容，放弃覆盖: {}",
                target.display()
            ));
        }
    }
    std::fs::rename(&staged_top, target).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging);
        format!("移动解压结果失败 {}: {e}", target.display())
    })?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// 后台启动编排：
/// 1) 先物化 `openai-bundled`（快，来自随包归档）并 `bundled_mark_done()`；
/// 2) 再以独立后台任务处理 `codex-primary-runtime`（按需下载/升级），完成或失败后
///    `runtime_mark_done()`，不阻塞 `openai-bundled` 的注册与使用。
pub(crate) async fn bootstrap(server: Arc<CodexServer>) {
    if let Some(dir) = archive_dir() {
        if dir.is_dir() {
            if let Some(spec) = bundle_spec("openai-bundled") {
                let archive = dir.join(spec.archive_name);
                let target = spec.dest.join(spec.top);
                server
                    .push_log(
                        "info",
                        format!(
                            "检查 openai-bundled：归档 {} → 目标 {}",
                            archive.display(),
                            target.display()
                        ),
                    )
                    .await;
                match bootstrap_one(&spec, &dir) {
                    BootOutcome::AlreadyPresent => {
                        server
                            .push_log(
                                "info",
                                format!("openai-bundled 已就位，跳过安装：{}", target.display()),
                            )
                            .await;
                    }
                    BootOutcome::ArchiveMissing => {
                        server
                            .push_log(
                                "info",
                                format!(
                                    "openai-bundled 归档未随包提供，跳过安装：{}",
                                    archive.display()
                                ),
                            )
                            .await;
                    }
                    BootOutcome::Extracted => {
                        server
                            .push_log(
                                "info",
                                format!("openai-bundled 已物化到 {}", target.display()),
                            )
                            .await;
                    }
                    BootOutcome::Updated => {
                        server
                            .push_log(
                                "info",
                                format!("openai-bundled 已按归档更新到新版本：{}", target.display()),
                            )
                            .await;
                    }
                    BootOutcome::Failed(e) => {
                        server
                            .push_log("warn", format!("openai-bundled 物化失败：{e}"))
                            .await;
                    }
                }
            }
        }
    }
    server.bundled_mark_done();

    // 运行时（慢 / 可能断网）：后台非阻塞处理。
    let server2 = server.clone();
    tauri::async_runtime::spawn(async move {
        match runtime_boot(server2.as_ref()).await {
            Ok(msg) => server2.push_log("info", msg).await,
            Err(e) => server2
                .push_log("warn", format!("codex-primary-runtime 处理失败：{e}"))
                .await,
        }
        server2.runtime_mark_done();
    });
}

/// 运行时按需下载/升级的处理（纯逻辑，便于推理）：
/// 需要下载时先尝试随包归档（兼容/离线），否则从 CDN 下载。
async fn runtime_boot(server: &CodexServer) -> Result<String, String> {
    let Some(spec) = bundle_spec("openai-primary-runtime") else {
        return Err("无法解析 codex-primary-runtime 的目录".to_string());
    };
    let target = spec.dest.join(spec.top);
    match runtime_download::runtime_status(&target) {
        runtime_download::RuntimeStatus::Current(ver) => {
            server
                .push_log(
                    "info",
                    format!("codex 运行时已是最新（bundleVersion {ver}），无需下载"),
                )
                .await;
            return Ok("codex 运行时已是最新，无需下载".to_string());
        }
        _ => {
            let reason = runtime_download::runtime_reason(&target)
                .unwrap_or_else(|| "未知".to_string());
            server
                .push_log("info", format!("需要下载/升级 codex-primary-runtime：{reason}"))
                .await;
        }
    }
    if let Some(dir) = archive_dir() {
        let archive = dir.join(spec.archive_name);
        if archive.is_file() {
            server
                .push_log(
                    "info",
                    format!(
                        "使用随包归档物化运行时：{} → {}",
                        archive.display(),
                        target.display()
                    ),
                )
                .await;
            return match materialize(&archive, &spec, &target, true) {
                Ok(()) => {
                    server
                        .push_log(
                            "info",
                            format!("codex-primary-runtime 已物化到 {}", target.display()),
                        )
                        .await;
                    Ok("已从随包归档物化 codex-primary-runtime".to_string())
                }
                Err(e) => {
                    server
                        .push_log("warn", format!("随包运行时归档解压失败：{e}"))
                        .await;
                    Err(format!("随包运行时归档解压失败：{e}"))
                }
            };
        }
    }
    runtime_download::ensure(&spec, server)
        .await
        .map(|_| "已下载并安装 codex-primary-runtime".to_string())
}

/// 路径越界防护：仅接受纯 `Normal` 组件（拒绝绝对路径、`.`、`..`）。
pub(crate) fn path_is_safe(path: &Path) -> bool {
    if path.is_absolute() {
        return false;
    }
    if path.as_os_str().is_empty() {
        return false;
    }
    path.components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// 用 Rust 原生 tar 解压 `.tar.gz` / `.tar.xz` 到 `dest`（按魔数自动识别压缩格式）。
/// - 创建 `dest`（等价 `tar -C` 要求目录存在）；
/// - 路径越界/符号链接条目直接跳过（归档为自有、可信数据，此处双保险）；
/// - 以归档内容为准覆盖写出（暂存目录可销毁，无需保留已存在文件）。
pub(crate) fn extract_tar(archive: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest)
        .map_err(|e| format!("创建解压目录失败 {}: {e}", dest.display()))?;
    let file = std::fs::File::open(archive)
        .map_err(|e| format!("打开归档失败 {}: {e}", archive.display()))?;
    let mut br = BufReader::new(file);
    let is_gzip = {
        let buf = br
            .fill_buf()
            .map_err(|e| format!("读取归档魔数失败: {e}"))?;
        buf.len() >= 2 && buf[0] == 0x1F && buf[1] == 0x8B
    };
    let reader: Box<dyn Read> = if is_gzip {
        Box::new(flate2::read::GzDecoder::new(br))
    } else {
        Box::new(xz2::read::XzDecoder::new(br))
    };
    let mut ar = tar::Archive::new(reader);
    let entries = ar
        .entries()
        .map_err(|e| format!("读取归档条目失败: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("读取归档条目失败: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("读取条目路径失败: {e}"))?
            .into_owned();
        // 双击防护：越界路径 / 符号链接 直接跳过（拒绝写入额外位置）
        if !path_is_safe(&path) {
            return Err(format!("归档包含越界路径: {}", path.display()));
        }
        if matches!(
            entry.header().entry_type(),
            tar::EntryType::Symlink | tar::EntryType::Link
        ) {
            continue;
        }
        entry
            .unpack_in(dest)
            .map_err(|e| format!("解压条目失败 {}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;

    /// 构造一个最小 `.tar.gz`：顶层目录 `<top>/`，内含若干 `<rel>` 文件。
    fn build_tar_gz(dir: &Path, top: &str, files: &[(&str, &str)]) -> PathBuf {
        let src = dir.join("_src");
        for (rel, content) in files {
            let p = src.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, content).unwrap();
        }
        let archive = dir.join(format!("{top}.tar.gz"));
        let file = fs::File::create(&archive).unwrap();
        let mut enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        {
            let mut b = tar::Builder::new(&mut enc);
            b.append_dir_all(top, &src).unwrap();
            b.finish().unwrap();
        }
        enc.finish().unwrap();
        archive
    }

    /// 构造一个最小 `.tar.xz`（用于验证 `extract_tar` 的 xz 自动识别）。
    fn build_tar_xz(dir: &Path, top: &str, files: &[(&str, &str)]) -> PathBuf {
        let src = dir.join("_src_xz");
        for (rel, content) in files {
            let p = src.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, content).unwrap();
        }
        let archive = dir.join(format!("{top}.tar.xz"));
        let file = fs::File::create(&archive).unwrap();
        let mut enc = xz2::write::XzEncoder::new(file, 6);
        {
            let mut b = tar::Builder::new(&mut enc);
            b.append_dir_all(top, &src).unwrap();
            b.finish().unwrap();
        }
        enc.finish().unwrap();
        archive
    }

    fn spec_for(dest: &Path) -> BundleSpec {
        BundleSpec {
            archive_name: BUNDLED_ARCHIVE,
            dest: dest.to_path_buf(),
            top: "openai-bundled",
        }
    }

    #[test]
    fn bundle_spec_in_maps_dest_top_and_archive() {
        let home = Path::new("C:\\home\\codex");
        let user = Path::new("C:\\Users\\u");
        let spec = bundle_spec_in(home, user, "openai-bundled").unwrap();
        assert_eq!(
            spec.dest,
            Path::new("C:\\home\\codex\\.tmp\\bundled-marketplaces")
        );
        assert_eq!(spec.top, "openai-bundled");
        assert_eq!(spec.archive_name, BUNDLED_ARCHIVE);

        let spec = bundle_spec_in(home, user, "openai-primary-runtime").unwrap();
        assert_eq!(spec.dest, Path::new("C:\\Users\\u\\.cache\\codex-runtimes"));
        assert_eq!(spec.top, "codex-primary-runtime");
        assert_eq!(spec.archive_name, RUNTIME_ARCHIVE);

        assert!(bundle_spec_in(home, user, "other").is_none());
    }

    #[test]
    fn path_is_safe_rejects_traversal_and_absolute() {
        assert!(path_is_safe(Path::new("a/b/c")));
        assert!(path_is_safe(Path::new("openai-bundled/.materialization-key")));
        assert!(!path_is_safe(Path::new("../evil")));
        assert!(!path_is_safe(Path::new("a/../../b")));
        assert!(!path_is_safe(Path::new("/absolute")));
        assert!(!path_is_safe(Path::new("")));
    }

    #[test]
    fn extract_tar_unpacks_and_creates_marker() {
        let dir = TempDir::new().unwrap();
        let archive = build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1")],
        );
        let dest = dir.path().join("dest");
        extract_tar(&archive, &dest).unwrap();
        let marker = dest.join("openai-bundled/.materialization-key");
        assert!(marker.is_file());
        assert_eq!(fs::read_to_string(&marker).unwrap(), "v1");
    }

    #[test]
    fn extract_tar_overwrites_existing_files() {
        let dir = TempDir::new().unwrap();
        let archive = build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1"), ("plugins/a.txt", "hello")],
        );
        let dest = dir.path().join("dest");
        let marker = dest.join("openai-bundled/.materialization-key");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, "keep").unwrap();
        extract_tar(&archive, &dest).unwrap();
        // 以归档为准覆盖写出（不再跳过已存在文件）
        assert_eq!(fs::read_to_string(&marker).unwrap(), "v1");
        assert!(dest.join("openai-bundled/plugins/a.txt").is_file());
    }

    #[test]
    fn extract_tar_autodetects_xz() {
        let dir = TempDir::new().unwrap();
        let archive = build_tar_xz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1")],
        );
        let dest = dir.path().join("dest");
        extract_tar(&archive, &dest).unwrap();
        let marker = dest.join("openai-bundled/.materialization-key");
        assert!(marker.is_file());
        assert_eq!(fs::read_to_string(&marker).unwrap(), "v1");
    }

    #[test]
    fn path_is_safe_guard_rejects_traversal_for_archive_entry() {
        assert!(path_is_safe(Path::new("codex-primary-runtime/runtime.json")));
        assert!(!path_is_safe(Path::new("codex-primary-runtime/../../evil")));
    }

    #[test]
    fn target_needs_materialize_true_when_absent_or_empty() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("openai-bundled");
        // 不存在 → true
        assert!(target_needs_materialize(&target));
        // 存在但为空 → true
        fs::create_dir_all(&target).unwrap();
        assert!(target_needs_materialize(&target));
    }

    #[test]
    fn target_needs_materialize_false_when_nonempty() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("openai-bundled");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("a.txt"), "x").unwrap();
        assert!(!target_needs_materialize(&target));
    }

    #[test]
    fn bootstrap_one_extracts_when_target_absent() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1"), ("plugins/a.txt", "hello")],
        );
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::Extracted);
        assert!(dest.join("openai-bundled/.materialization-key").is_file());
        assert!(dest.join("openai-bundled/plugins/a.txt").is_file());
        assert!(!dest.join(".codex-ui-extract-openai-bundled").exists());
    }

    #[test]
    fn bootstrap_one_rebuilds_empty_target_dir() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir_all(dest.join("openai-bundled")).unwrap();
        build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1")],
        );
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::Extracted);
        assert!(dest.join("openai-bundled/.materialization-key").is_file());
    }

    #[test]
    fn bootstrap_one_cleans_stale_staging() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        // 半路关闭留下的脏暂存目录（固定名 `.codex-ui-extract-<top>`），
        // 以及最终目标不存在。
        let stale = dest.join(".codex-ui-extract-openai-bundled");
        fs::create_dir_all(stale.join("openai-bundled")).unwrap();
        fs::write(stale.join("openai-bundled/stale.txt"), "partial").unwrap();
        build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1")],
        );
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::Extracted);
        // 脏暂存被清空重建，目标目录完整，无暂存残留
        assert!(!stale.exists());
        assert!(dest.join("openai-bundled/.materialization-key").is_file());
        assert!(!dest.join(".codex-ui-extract-openai-bundled").exists());
    }

    #[test]
    fn bootstrap_one_skips_nonempty_target_when_versions_equal() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir_all(dest.join("openai-bundled")).unwrap();
        fs::write(
            dest.join("openai-bundled/.materialization-key"),
            r#"{"appVersion":"26.730.61309"}"#,
        )
        .unwrap();
        build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", r#"{"appVersion":"26.730.61309"}"#)],
        );
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::AlreadyPresent);
        // 版本一致时不覆盖
        assert_eq!(
            fs::read_to_string(dest.join("openai-bundled/.materialization-key")).unwrap(),
            r#"{"appVersion":"26.730.61309"}"#
        );
    }

    #[test]
    fn read_installed_app_version_handles_missing_invalid_and_valid() {
        let dir = TempDir::new().unwrap();
        // 文件不存在 → None
        assert_eq!(read_installed_app_version(dir.path()), None);
        // 非法 JSON → None
        fs::write(dir.path().join(".materialization-key"), "not json").unwrap();
        assert_eq!(read_installed_app_version(dir.path()), None);
        // 合法 JSON 但没有 appVersion 键 → None
        fs::write(dir.path().join(".materialization-key"), r#"{"version":1}"#).unwrap();
        assert_eq!(read_installed_app_version(dir.path()), None);
        // 有 appVersion → Some
        fs::write(
            dir.path().join(".materialization-key"),
            r#"{"appVersion":"26.730.61309"}"#,
        )
        .unwrap();
        assert_eq!(
            read_installed_app_version(dir.path()),
            Some("26.730.61309".to_string())
        );
    }

    #[test]
    fn read_archive_app_version_reads_marker_from_gzip() {
        let dir = TempDir::new().unwrap();
        let archive = build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", r#"{"appVersion":"26.730.61309"}"#)],
        );
        assert_eq!(
            read_archive_app_version(&archive, "openai-bundled"),
            Some("26.730.61309".to_string())
        );
        // 顶层目录名不匹配 → None
        assert_eq!(read_archive_app_version(&archive, "other"), None);
    }

    #[test]
    fn bootstrap_one_updates_when_installed_version_below_archive() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir_all(dest.join("openai-bundled")).unwrap();
        fs::write(
            dest.join("openai-bundled/.materialization-key"),
            r#"{"appVersion":"26.730.61000"}"#,
        )
        .unwrap();
        build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", r#"{"appVersion":"26.730.61309"}"#)],
        );
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::Updated);
        assert_eq!(
            fs::read_to_string(dest.join("openai-bundled/.materialization-key")).unwrap(),
            r#"{"appVersion":"26.730.61309"}"#
        );
    }

    #[test]
    fn bootstrap_one_updates_when_installed_key_missing() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        // 目标非空但缺少 .materialization-key
        fs::create_dir_all(dest.join("openai-bundled")).unwrap();
        fs::write(dest.join("openai-bundled/keep.txt"), "x").unwrap();
        build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", r#"{"appVersion":"26.730.61309"}"#)],
        );
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::Updated);
        assert_eq!(
            fs::read_to_string(dest.join("openai-bundled/.materialization-key")).unwrap(),
            r#"{"appVersion":"26.730.61309"}"#
        );
    }

    #[test]
    fn bootstrap_one_skips_newer_target_without_downgrade() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir_all(dest.join("openai-bundled")).unwrap();
        fs::write(
            dest.join("openai-bundled/.materialization-key"),
            r#"{"appVersion":"26.730.61639"}"#,
        )
        .unwrap();
        build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", r#"{"appVersion":"26.730.61309"}"#)],
        );
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::AlreadyPresent);
        // 磁盘版本更高，不被降级覆盖
        assert_eq!(
            fs::read_to_string(dest.join("openai-bundled/.materialization-key")).unwrap(),
            r#"{"appVersion":"26.730.61639"}"#
        );
    }

    #[test]
    fn bootstrap_one_archive_missing_keeps_present_target() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir_all(dest.join("openai-bundled")).unwrap();
        fs::write(
            dest.join("openai-bundled/.materialization-key"),
            r#"{"appVersion":"26.730.61000"}"#,
        )
        .unwrap();
        // 归档缺失：目标存在 → AlreadyPresent，不动目标
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::AlreadyPresent);
        assert!(dest.join("openai-bundled/.materialization-key").is_file());
    }

    #[test]
    fn bootstrap_one_archive_missing_when_target_absent() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::ArchiveMissing);
        assert!(!dest.join("openai-bundled").exists());
    }

    #[test]
    fn bootstrap_one_failed_leaves_target_untouched() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        fs::write(dir.path().join("openai-bundled.tar.gz"), b"not an archive").unwrap();
        let spec = spec_for(&dest);
        assert!(matches!(bootstrap_one(&spec, dir.path()), BootOutcome::Failed(_)));
        assert!(!dest.join("openai-bundled").exists());
    }

    #[test]
    fn materialize_replace_overwrites_nonempty_target() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        let target = dest.join("openai-bundled");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join(".materialization-key"), "old").unwrap();
        let archive = build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1")],
        );
        let spec = spec_for(&dest);
        materialize(&archive, &spec, &target, true).unwrap();
        assert_eq!(
            fs::read_to_string(target.join(".materialization-key")).unwrap(),
            "v1"
        );
    }

    #[test]
    fn materialize_refuses_overwrite_when_replace_false() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        let target = dest.join("openai-bundled");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join(".materialization-key"), "old").unwrap();
        let archive = build_tar_gz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1")],
        );
        let spec = spec_for(&dest);
        assert!(materialize(&archive, &spec, &target, false).is_err());
        assert_eq!(
            fs::read_to_string(target.join(".materialization-key")).unwrap(),
            "old"
        );
    }
}
