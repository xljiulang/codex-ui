//! 内置插件市场的启动期解压。
//!
//! 安装器把各市场的 `.tar.xz` 归档放到 `<应用目录>/marketplaces/` 下；codex-ui 启动时在后台
//! 把它们解压到 codex 认可的 canonical 位置，随后由 `app_server` 的「内置市场登记」注册。
//!
//! 为避免「标记先落、内容缺」的半成品：仅当目标目录 `dest/<top>` 不存在或为空时才物化，
//! 且物化采用「临时目录完整解压 → 整体改名」——目标目录只会由一次完整的 `rename` 产生，
//! 因此本应用创建的目录必然完整。已存在且非空的目标目录一律当作已就位、跳过，不修复、不重建。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::codex::app_server::{BUNDLED_MARKETPLACES, CodexServer, app_exe_dir};
use crate::codex::model_config;

/// 内置市场归档文件名。
pub(crate) const RUNTIME_ARCHIVE: &str = "codex-primary-runtime.tar.xz";
pub(crate) const BUNDLED_ARCHIVE: &str = "openai-bundled.tar.xz";

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
/// - openai-bundled -> 归档 `openai-bundled.tar.xz`，解压到 `<CODEX_HOME>/.tmp/bundled-marketplaces`
/// - openai-primary-runtime -> 归档 `codex-primary-runtime.tar.xz`，解压到 `%USERPROFILE%\.cache\codex-runtimes`
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

/// 单市场解压决策（纯函数，便于测试）：目标已就位则跳过；否则临时目录完整解压后整体改名。
pub(crate) fn bootstrap_one(spec: &BundleSpec, archive_dir: &Path) -> BootOutcome {
    let target = spec.dest.join(spec.top);
    if !target_needs_materialize(&target) {
        return BootOutcome::AlreadyPresent;
    }
    let archive = archive_dir.join(spec.archive_name);
    if !archive.is_file() {
        return BootOutcome::ArchiveMissing;
    }
    match materialize(&archive, spec, &target) {
        Ok(()) => BootOutcome::Extracted,
        Err(e) => BootOutcome::Failed(e),
    }
}

/// 原子物化：先完整解压到 `dest/.codex-ui-extract-<top>`，校验后整体改名到目标目录。
/// 任一步失败只清理临时目录，不落目标，交由下次启动重试。
fn materialize(archive: &Path, spec: &BundleSpec, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(&spec.dest)
        .map_err(|e| format!("创建目标根失败 {}: {e}", spec.dest.display()))?;
    let staging = spec.dest.join(format!(".codex-ui-extract-{}", spec.top));
    // 清理上次失败可能留下的临时目录；`NotFound`（首次/已无残留）视为成功，
    // 其它清理失败则直接失败，避免在脏暂存上继续解压，否则残留文件可能随整体改名进入目标目录。
    match std::fs::remove_dir_all(&staging) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("清理暂存目录失败 {}: {e}", staging.display())),
    }
    if let Err(e) = extract_tar_xz(archive, &staging) {
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
        if !is_empty_dir(target) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!(
                "目标目录已被写入内容，放弃覆盖: {}",
                target.display()
            ));
        }
        std::fs::remove_dir(target).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staging);
            format!("移除空目标目录失败 {}: {e}", target.display())
        })?;
    }
    std::fs::rename(&staged_top, target).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging);
        format!("移动解压结果失败 {}: {e}", target.display())
    })?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// 后台遍历全部内置市场：目标已就位或归档缺失则跳过；否则临时解压并整体改名。
/// 失败只告警不抛、目标不落，由下次启动重试。结束后通知等待方（标记完成）。
pub(crate) async fn bootstrap(server: Arc<CodexServer>) {
    let Some(dir) = archive_dir() else {
        server.bundled_mark_done();
        return;
    };
    // 无 marketplaces 目录（开发/裸跑）等价于「未随包提供」，保持未注册现状。
    if !dir.is_dir() {
        server.bundled_mark_done();
        return;
    }
    for name in BUNDLED_MARKETPLACES {
        let Some(spec) = bundle_spec(name) else {
            continue;
        };
        match bootstrap_one(&spec, &dir) {
            BootOutcome::AlreadyPresent => {}
            BootOutcome::ArchiveMissing => {
                server
                    .push_log(
                        "info",
                        format!("内置插件市场 {name} 的归档未随包提供，跳过解压"),
                    )
                    .await;
            }
            BootOutcome::Extracted => {
                server
                    .push_log("info", format!("已解压内置插件市场 {name}"))
                    .await;
            }
            BootOutcome::Failed(e) => {
                server
                    .push_log("warn", format!("解压内置插件市场 {name} 失败：{e}"))
                    .await;
            }
        }
    }
    server.bundled_mark_done();
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

/// 用 Rust 原生 xz + tar 解压 `.tar.xz` 到 `dest`。
/// - 创建 `dest`（等价 `tar -C` 要求目录存在）；
/// - 路径越界/符号链接条目直接跳过（归档为自有、可信数据，此处双保险）；
/// - 以归档内容为准覆盖写出（暂存目录可销毁，无需保留已存在文件）。
pub(crate) fn extract_tar_xz(archive: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest)
        .map_err(|e| format!("创建解压目录失败 {}: {e}", dest.display()))?;
    let file = std::fs::File::open(archive)
        .map_err(|e| format!("打开归档失败 {}: {e}", archive.display()))?;
    let xz = xz2::read::XzDecoder::new(file);
    let mut ar = tar::Archive::new(xz);
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

    /// 构造一个与真实结构同构的最小 `.tar.xz`：顶层目录 `<top>/`，内含若干 `<rel>` 文件。
    fn build_tar_xz(dir: &Path, top: &str, files: &[(&str, &str)]) -> PathBuf {
        let src = dir.join("_src");
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
    fn extract_tar_xz_unpacks_and_creates_marker() {
        let dir = TempDir::new().unwrap();
        let archive = build_tar_xz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1")],
        );
        let dest = dir.path().join("dest");
        extract_tar_xz(&archive, &dest).unwrap();
        let marker = dest.join("openai-bundled/.materialization-key");
        assert!(marker.is_file());
        assert_eq!(fs::read_to_string(&marker).unwrap(), "v1");
    }

    #[test]
    fn extract_tar_xz_overwrites_existing_files() {
        let dir = TempDir::new().unwrap();
        let archive = build_tar_xz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1"), ("plugins/a.txt", "hello")],
        );
        let dest = dir.path().join("dest");
        let marker = dest.join("openai-bundled/.materialization-key");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, "keep").unwrap();
        extract_tar_xz(&archive, &dest).unwrap();
        // 以归档为准覆盖写出（不再跳过已存在文件）
        assert_eq!(fs::read_to_string(&marker).unwrap(), "v1");
        assert!(dest.join("openai-bundled/plugins/a.txt").is_file());
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
        build_tar_xz(
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
        build_tar_xz(
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
        build_tar_xz(
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
    fn bootstrap_one_skips_nonempty_target_dir() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir_all(dest.join("openai-bundled")).unwrap();
        fs::write(dest.join("openai-bundled/.materialization-key"), "existing").unwrap();
        build_tar_xz(
            dir.path(),
            "openai-bundled",
            &[(".materialization-key", "v1")],
        );
        let spec = spec_for(&dest);
        assert_eq!(bootstrap_one(&spec, dir.path()), BootOutcome::AlreadyPresent);
        // 非空目标不被覆盖
        assert_eq!(
            fs::read_to_string(dest.join("openai-bundled/.materialization-key")).unwrap(),
            "existing"
        );
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
        fs::write(dir.path().join("openai-bundled.tar.xz"), b"not an archive").unwrap();
        let spec = spec_for(&dest);
        assert!(matches!(bootstrap_one(&spec, dir.path()), BootOutcome::Failed(_)));
        assert!(!dest.join("openai-bundled").exists());
    }
}
