//! 浏览器桥接自愈：openai-bundled 的 chrome 插件重装后只落版本目录、不重建
//! `latest` junction，而原生宿主注册（HKCU 清单 + CODEX_HOME 下
//! chrome-native-hosts-v2.json）引用的都是 `...\chrome\latest\...` 路径，
//! 链接一断 Chrome 扩展就拉不起 extension-host，codex 的 `agent.browsers.list()`
//! 变空。本模块在插件安装后检查并修复该链接（必要时补写宿主注册），让控制
//! Chrome 的链路随安装即可用；另提供进程状态查询供卸载预检（运行中的
//! extension-host/node_repl 会锁住缓存目录文件，导致卸载报 os error 5），
//! 以及安装 chrome 插件前的「结束桥接进程」入口（同一批文件被占用时
//! `plugin/install` 备份缓存会失败）。

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::model_config;
use super::path_util::clean_path;

/// 修复报告（camelCase 序列化给前端）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRepairReport {
    /// ok：检查通过或修复成功；no-registration：本机无浏览器桥接注册，无需处理；
    /// error：修复失败（message 说明原因）。
    pub status: String,
    /// none / already-ok / created / recreated
    pub latest_action: String,
    pub latest_target: Option<String>,
    pub host_path_ok: bool,
    pub client_path_ok: bool,
    pub registry_fixed: bool,
    pub message: String,
}

/// 桥接进程状态（安装/卸载预检用）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub extension_host_running: bool,
    pub node_repl_running: bool,
}

/// 结束桥接进程的结果：stopped/failed 都是进程名（failed 含结束失败的原因）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStopReport {
    pub stopped: Vec<String>,
    pub failed: Vec<String>,
}

/// 需要结束的桥接进程名（扩展宿主由 Chrome 拉起、node_repl 由 codex 拉起，
/// 二者都可能占用插件缓存目录里的文件）。
const BRIDGE_PROCESS_NAMES: [&str; 2] = ["extension-host.exe", "node_repl.exe"];

/// 插件安装后自愈入口：修复 latest junction 与原生宿主注册。
#[tauri::command]
pub fn browser_bridge_repair() -> Result<BridgeRepairReport, String> {
    let home = model_config::codex_home()?;
    Ok(repair_in(&home))
}

/// 查询桥接相关进程是否在运行（安装/卸载预检）。
#[tauri::command]
pub fn browser_bridge_status() -> Result<BridgeStatus, String> {
    Ok(BridgeStatus {
        extension_host_running: process_running(BRIDGE_PROCESS_NAMES[0]),
        node_repl_running: process_running(BRIDGE_PROCESS_NAMES[1]),
    })
}

/// 结束桥接进程（安装 chrome 插件前由用户确认后调用）：
/// 逐 PID `TerminateProcess` 并等待退出，进程不存在不算失败。
#[tauri::command]
pub fn browser_bridge_stop() -> Result<BridgeStopReport, String> {
    Ok(stop_processes(&BRIDGE_PROCESS_NAMES))
}

/// 结束给定名字的进程，返回成功/失败名单（纯编排，便于测试注入伪进程名）。
fn stop_processes(names: &[&str]) -> BridgeStopReport {
    let mut stopped: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for name in names {
        let pids = pids_of(name);
        if pids.is_empty() {
            continue;
        }
        let mut failures: Vec<String> = Vec::new();
        let mut killed_any = false;
        for pid in pids {
            match terminate_pid(pid, std::time::Duration::from_secs(2)) {
                Ok(()) => killed_any = true,
                Err(e) => failures.push(format!("pid {pid}: {e}")),
            }
        }
        if killed_any {
            stopped.push((*name).to_string());
        }
        if !failures.is_empty() {
            failed.push(format!("{}（{}）", name, failures.join("；")));
        }
    }
    BridgeStopReport { stopped, failed }
}

impl BridgeRepairReport {
    fn no_registration(message: &str) -> Self {
        Self {
            status: "no-registration".into(),
            latest_action: "none".into(),
            latest_target: None,
            host_path_ok: false,
            client_path_ok: false,
            registry_fixed: false,
            message: message.into(),
        }
    }
}

fn repair_in(home: &Path) -> BridgeRepairReport {
    let reg_path = home.join("chrome-native-hosts-v2.json");
    let text = match std::fs::read_to_string(&reg_path) {
        Ok(t) => t,
        Err(_) => {
            return BridgeRepairReport::no_registration(
                "未找到 chrome-native-hosts-v2.json（本机无浏览器桥接注册），无需处理",
            )
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            return BridgeRepairReport::no_registration(&format!(
                "chrome-native-hosts-v2.json 解析失败，跳过修复: {e}"
            ))
        }
    };
    let Some(entries) = value.get("entries").and_then(|v| v.as_array()) else {
        return BridgeRepairReport::no_registration(
            "chrome-native-hosts-v2.json 缺少 entries，无需处理",
        );
    };

    // 收集注册文件引用的路径与宿主注册信息
    let mut ref_paths: Vec<String> = Vec::new();
    let mut host_names: Vec<String> = Vec::new();
    let mut ext_ids: Vec<String> = Vec::new();
    for entry in entries {
        if let Some(paths) = entry.get("paths").and_then(|v| v.as_object()) {
            for key in ["extensionHostPath", "browserClientPath"] {
                if let Some(s) = paths.get(key).and_then(|v| v.as_str()) {
                    ref_paths.push(s.to_string());
                }
            }
        }
        collect_strings(entry, "nativeHostNames", &mut host_names);
        collect_strings(entry, "extensionIds", &mut ext_ids);
    }

    // 修复全部引用到 latest 的插件目录（host/client 路径通常指向同一目录，去重）
    let mut plugin_dirs: Vec<PathBuf> = Vec::new();
    for rp in &ref_paths {
        if let Some(dir) = plugin_dir_of_latest(Path::new(rp)) {
            let key = super::path_util::norm_path_key(&dir);
            if !plugin_dirs
                .iter()
                .any(|d| super::path_util::norm_path_key(d) == key)
            {
                plugin_dirs.push(dir);
            }
        }
    }
    let mut latest_action: Option<&'static str> = None;
    let mut latest_target: Option<String> = None;
    let mut repaired = false;
    let mut errors: Vec<String> = Vec::new();
    for dir in &plugin_dirs {
        match repair_latest_in(dir) {
            Ok((action, target)) => {
                if action == "already-ok" {
                    latest_action.get_or_insert(action);
                } else {
                    latest_action = Some(action);
                    repaired = true;
                }
                if let Some(t) = target {
                    latest_target = Some(clean_path(&t));
                }
            }
            Err(e) => errors.push(e),
        }
    }

    // 原生宿主注册（HKCU 键 + 清单文件）缺失时补写
    let mut registry_fixed = false;
    for entry in entries {
        match ensure_native_host_registration(entry, &host_names, &ext_ids) {
            Ok(fixed) => registry_fixed |= fixed,
            Err(e) => errors.push(e),
        }
    }

    let host_path_ok = ref_paths.first().is_some_and(|s| Path::new(s).exists());
    let client_path_ok = ref_paths.get(1).is_some_and(|s| Path::new(s).exists());
    let paths_ok = host_path_ok && client_path_ok;

    let (status, message) = if !errors.is_empty() {
        (
            "error",
            format!("浏览器桥接自愈失败：{}", errors.join("；")),
        )
    } else if !paths_ok && repaired {
        (
            "error",
            "latest 链接已重建，但注册路径仍不可达（注册引用的文件缺失）".to_string(),
        )
    } else if !paths_ok {
        (
            "error",
            "浏览器桥接注册路径不可达，且未找到可修复的 latest 链接".to_string(),
        )
    } else if repaired || registry_fixed {
        let mut m = match &latest_target {
            Some(t) => format!("已重建浏览器桥接链接 → {t}"),
            None => "浏览器桥接已修复".to_string(),
        };
        if registry_fixed {
            m.push_str("，并补写了原生宿主注册");
        }
        ("ok", m)
    } else {
        ("ok", "浏览器桥接检查通过，无需修复".to_string())
    };

    BridgeRepairReport {
        status: status.into(),
        latest_action: latest_action.unwrap_or("none").into(),
        latest_target,
        host_path_ok,
        client_path_ok,
        registry_fixed,
        message,
    }
}

fn collect_strings(value: &serde_json::Value, key: &str, out: &mut Vec<String>) {
    if let Some(arr) = value.get(key).and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                out.push(s.to_string());
            }
        }
    }
}

/// 从注册路径推导引用 latest 的插件目录：
/// `...\openai-bundled\chrome\latest\x.exe` → `...\openai-bundled\chrome`；
/// 路径不含 latest 组件时返回 None。
fn plugin_dir_of_latest(ref_path: &Path) -> Option<PathBuf> {
    let cleaned = clean_path(ref_path);
    let parts: Vec<&str> = cleaned.split('\\').filter(|s| !s.is_empty()).collect();
    let idx = parts
        .iter()
        .rposition(|c| c.eq_ignore_ascii_case("latest"))?;
    if idx == 0 || idx + 1 >= parts.len() {
        return None;
    }
    Some(parts[..idx].join("\\").into())
}

/// 版本目录名比较：按 `.` 分段数值升序（非数字段按 0，完全相等回退字典序）。
fn cmp_version_names(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (va, vb) = (key(a), key(b));
    for i in 0..va.len().max(vb.len()) {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        if x != y {
            return x.cmp(&y);
        }
    }
    a.cmp(b)
}

/// 在插件目录下挑选最新版本目录（跳过 latest 本身；无候选返回 None）。
fn pick_newest_version_dir(plugin_dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(String, PathBuf)> = None;
    for entry in std::fs::read_dir(plugin_dir).ok()? {
        let Ok(entry) = entry else { continue };
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.eq_ignore_ascii_case("latest") {
            continue;
        }
        match &best {
            Some((bn, _)) if cmp_version_names(name, bn) != std::cmp::Ordering::Greater => {}
            _ => best = Some((name.to_string(), p)),
        }
    }
    best.map(|(_, p)| p)
}

#[derive(Debug, PartialEq, Eq)]
enum LatestState {
    /// 链接存在且目标可达
    Ok,
    /// 链接存在但目标不可达（悬空）
    Dangling,
    /// 同名路径被实体目录/文件占用（不是链接）
    NotAReparsePoint,
    /// 不存在
    Missing,
}

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[cfg(windows)]
fn latest_state(latest: &Path) -> LatestState {
    use std::os::windows::fs::MetadataExt;

    match std::fs::symlink_metadata(latest) {
        Ok(md) => {
            if md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                LatestState::NotAReparsePoint
            } else if latest.exists() {
                LatestState::Ok
            } else {
                LatestState::Dangling
            }
        }
        Err(_) => LatestState::Missing,
    }
}

#[cfg(not(windows))]
fn latest_state(latest: &Path) -> LatestState {
    match std::fs::symlink_metadata(latest) {
        Ok(md) => {
            if md.file_type().is_symlink() {
                if latest.exists() {
                    LatestState::Ok
                } else {
                    LatestState::Dangling
                }
            } else {
                LatestState::NotAReparsePoint
            }
        }
        Err(_) => LatestState::Missing,
    }
}

/// 修复单个插件目录的 latest 链接，返回 (action, target)。
fn repair_latest_in(plugin_dir: &Path) -> Result<(&'static str, Option<PathBuf>), String> {
    let latest = plugin_dir.join("latest");
    let action = match latest_state(&latest) {
        LatestState::Ok => return Ok(("already-ok", None)),
        LatestState::Dangling => {
            // remove_dir 对 junction 只移除链接本身，不触碰目标内容
            std::fs::remove_dir(&latest)
                .map_err(|e| format!("移除悬空链接失败({}): {e}", latest.display()))?;
            "recreated"
        }
        LatestState::Missing => "created",
        LatestState::NotAReparsePoint => {
            let empty = std::fs::read_dir(&latest)
                .map(|mut it| it.next().is_none())
                .unwrap_or(false);
            if !empty {
                return Err(format!(
                    "latest 路径被非空实体目录占用，无法自动修复: {}",
                    latest.display()
                ));
            }
            std::fs::remove_dir(&latest)
                .map_err(|e| format!("移除空目录失败({}): {e}", latest.display()))?;
            "recreated"
        }
    };
    let Some(target) = pick_newest_version_dir(plugin_dir) else {
        return Err(format!(
            "插件目录下没有可用的版本目录: {}",
            plugin_dir.display()
        ));
    };
    create_junction(&latest, &target)?;
    Ok((action, Some(target)))
}

/// 生成原生宿主清单 JSON（与 OpenAI 桌面端写入格式一致）。
fn manifest_json(name: &str, extension_ids: &[String], host_path: &str) -> String {
    let origins: Vec<String> = extension_ids
        .iter()
        .map(|id| format!("chrome-extension://{id}/"))
        .collect();
    let value = serde_json::json!({
        "allowed_origins": origins,
        "description": "ChatGPT browser native messaging host",
        "name": name,
        "path": clean_path(Path::new(host_path)),
        "type": "stdio",
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

/// 校验并按需补写原生宿主注册（HKCU\...\NativeMessagingHosts\<name> → 清单文件）。
/// 返回是否发生了补写。
fn ensure_native_host_registration(
    entry: &serde_json::Value,
    host_names: &[String],
    ext_ids: &[String],
) -> Result<bool, String> {
    if host_names.is_empty() {
        return Ok(false);
    }
    let Some(host_path) = entry
        .get("paths")
        .and_then(|p| p.get("extensionHostPath"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    else {
        return Ok(false);
    };

    let manifest_dir = local_app_data()?.join("OpenAI").join("extension");
    let mut fixed = false;
    for name in host_names {
        let manifest_path = manifest_dir.join(format!("{name}.json"));
        let reg_sub = format!(r"Software\Google\Chrome\NativeMessagingHosts\{name}");

        // 注册表默认值指向的清单存在，且清单内 path 可达 → 视为已注册
        let mut need_manifest = true;
        let mut need_registry = true;
        if let Ok(Some(mp)) = read_reg_default(&reg_sub) {
            if Path::new(&mp).exists() {
                let manifest_ok = std::fs::read_to_string(&mp)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .and_then(|v| {
                        v.get("path")
                            .and_then(|p| p.as_str())
                            .map(|s| Path::new(s).exists())
                    })
                    .unwrap_or(false);
                need_manifest = !manifest_ok;
                need_registry = false;
            }
        }
        if need_manifest {
            let json = manifest_json(name, ext_ids, host_path);
            model_config::atomic_write(&manifest_path, &json)?;
            fixed = true;
        }
        if need_manifest || need_registry {
            write_reg_default(&reg_sub, &manifest_path.to_string_lossy())?;
            fixed = true;
        }
    }
    Ok(fixed)
}

fn local_app_data() -> Result<PathBuf, String> {
    if let Some(v) = std::env::var_os("LOCALAPPDATA") {
        if !v.is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        if !profile.is_empty() {
            return Ok(Path::new(&profile).join("AppData").join("Local"));
        }
    }
    Err("无法定位 %LOCALAPPDATA%（未设置且找不到 %USERPROFILE%）".to_string())
}

/// 创建目录联接（等价 `mklink /J link target`）：先建空目录再挂 reparse point。
/// junction 无需管理员权限。
#[cfg(windows)]
pub(crate) fn create_junction(link: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir(link).map_err(|e| format!("创建目录失败({}): {e}", link.display()))?;
    if let Err(e) = set_junction_reparse(link, target) {
        // 失败时清掉刚建的空目录，避免残留半成品
        let _ = std::fs::remove_dir(link);
        return Err(e);
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn create_junction(_link: &Path, _target: &Path) -> Result<(), String> {
    Err("仅支持 Windows".to_string())
}

/// 在已存在的空目录上写入 MOUNT_POINT reparse point。
#[cfg(windows)]
fn set_junction_reparse(link: &Path, target: &Path) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_MODE,
        OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
    use windows::Win32::System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT;
    use windows::Win32::System::IO::DeviceIoControl;

    if !target.is_absolute() {
        return Err(format!("junction 目标必须是绝对路径: {}", target.display()));
    }

    // MountPointReparseBuffer 布局（对齐 junction crate 的已知可用实现）：
    // SubstituteName = \??\<target>，PrintName = <target>，两个名字各带 1 个 NUL
    // 且 NUL 不计入各自 Length；ReparseDataLength 覆盖四个偏移/长度字段 + PathBuffer。
    let subst: Vec<u16> = OsStr::new("\\??\\")
        .encode_wide()
        .chain(target.as_os_str().encode_wide())
        .collect();
    let print: Vec<u16> = target.as_os_str().encode_wide().collect();
    let subst_bytes = subst.len() * 2;
    let print_bytes = print.len() * 2;
    let reparse_data_len = 8 + subst_bytes + 2 + print_bytes + 2;
    let mut buf = vec![0u8; 8 + reparse_data_len];
    buf[0..4].copy_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_ne_bytes());
    buf[4..6].copy_from_slice(&(reparse_data_len as u16).to_ne_bytes());
    // Reserved(2) 与 SubstituteNameOffset(2)=0 保持 0
    buf[10..12].copy_from_slice(&(subst_bytes as u16).to_ne_bytes());
    buf[12..14].copy_from_slice(&((subst_bytes + 2) as u16).to_ne_bytes());
    buf[14..16].copy_from_slice(&(print_bytes as u16).to_ne_bytes());
    for (i, w) in subst.iter().enumerate() {
        buf[16 + i * 2..16 + i * 2 + 2].copy_from_slice(&w.to_ne_bytes());
    }
    // subst 的结尾 NUL 初始即为 0
    let print_start = 16 + subst_bytes + 2;
    for (i, w) in print.iter().enumerate() {
        buf[print_start + i * 2..print_start + i * 2 + 2].copy_from_slice(&w.to_ne_bytes());
    }

    let link_w: Vec<u16> = link.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut bytes_returned = 0u32;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(link_w.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            Some(HANDLE::default()),
        )
        .map_err(|e| format!("打开目录失败({}): {e}", link.display()))?
    };
    let result = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_REPARSE_POINT,
            Some(buf.as_ptr().cast()),
            buf.len() as u32,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        )
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.map_err(|e| format!("设置 reparse point 失败({}): {e}", link.display()))
}

#[cfg(windows)]
fn pids_of(exe_name: &str) -> Vec<u32> {
    use std::mem::size_of;

    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut pids: Vec<u32> = Vec::new();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let Ok(snapshot) = snapshot else {
        return pids;
    };
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let target = exe_name.to_lowercase();
    let mut iter = unsafe { Process32FirstW(snapshot, &mut entry) };
    while iter.is_ok() {
        let name = String::from_utf16_lossy(&entry.szExeFile);
        if name.trim_end_matches('\0').to_lowercase() == target {
            pids.push(entry.th32ProcessID);
        }
        iter = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    pids
}

#[cfg(not(windows))]
fn pids_of(_exe_name: &str) -> Vec<u32> {
    Vec::new()
}

fn process_running(exe_name: &str) -> bool {
    !pids_of(exe_name).is_empty()
}

/// 结束指定 PID 并等待其退出（Windows 实现见下；非 Windows 恒返回 Err）。
#[cfg(windows)]
fn terminate_pid(pid: u32, wait: std::time::Duration) -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    // 结束进程要 PROCESS_TERMINATE，随后的等待还要 PROCESS_SYNCHRONIZE
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) };
    let Ok(handle) = handle else {
        return Err(format!("打开进程失败: {}", std::io::Error::last_os_error()));
    };
    let result = (|| -> Result<(), String> {
        unsafe { TerminateProcess(handle, 1) }
            .map_err(|e| format!("结束进程失败: {e}（{}）", std::io::Error::last_os_error()))?;
        let waited = unsafe { WaitForSingleObject(handle, wait.as_millis() as u32) };
        if waited != WAIT_OBJECT_0 {
            return Err(format!(
                "等待进程退出超时: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    })();
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

#[cfg(not(windows))]
fn terminate_pid(_pid: u32, _wait: std::time::Duration) -> Result<(), String> {
    Err("仅 Windows 支持结束桥接进程".to_string())
}

#[cfg(windows)]
fn read_reg_default(sub_key: &str) -> Result<Option<String>, String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(sub_key, KEY_QUERY_VALUE)
        .map_err(|e| format!("读取注册表失败({sub_key}): {e}"))?;
    Ok(key.get_value::<String, _>("").ok())
}

#[cfg(windows)]
fn write_reg_default(sub_key: &str, value: &str) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(sub_key)
        .map_err(|e| format!("创建注册表键失败({sub_key}): {e}"))?;
    key.set_value("", &value)
        .map_err(|e| format!("写注册表默认值失败({sub_key}): {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_version_dir(root: &Path, name: &str, file: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(file), b"payload").unwrap();
        d
    }

    #[test]
    fn stop_processes_ignores_missing_processes() {
        let report = stop_processes(&["codex-ui-definitely-not-running.exe"]);
        assert!(report.stopped.is_empty(), "{:?}", report.stopped);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
    }

    /// 枚举工具能按名字找到当前进程（自身 exe 名来自 current_exe）。
    #[cfg(windows)]
    #[test]
    fn pids_of_finds_current_process() {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Some(name) = exe.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            return;
        };
        let pids = pids_of(&name);
        assert!(
            pids.contains(&std::process::id()),
            "应在 {name} 的 PID 列表里，实际 {pids:?}"
        );
    }

    /// 真起一个子进程并按 PID 结束：验证 OpenProcess/TerminateProcess/等待链路可用。
    #[cfg(windows)]
    #[test]
    fn terminate_pid_kills_real_process() {
        let mut child = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("应能启动 powershell");
        let pid = child.id();
        terminate_pid(pid, std::time::Duration::from_secs(5)).expect("应能结束该进程");
        // 进程已被结束：reap 后不应还在运行
        let _ = child.wait();
        let still_running = pids_of("powershell.exe").contains(&pid);
        assert!(!still_running, "pid {pid} 应已退出");
    }

    #[test]
    fn version_compare_numeric_by_segments() {
        assert_eq!(
            cmp_version_names("26.730.61309", "26.730.61639"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            cmp_version_names("0.10.0", "0.2.4"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_version_names("1.0.19", "1.0.19"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn pick_newest_version_dir_skips_latest() {
        let dir = TempDir::new().unwrap();
        write_version_dir(dir.path(), "26.730.61309", "a.txt");
        write_version_dir(dir.path(), "26.730.61639", "a.txt");
        std::fs::create_dir(dir.path().join("latest")).unwrap();
        let picked = pick_newest_version_dir(dir.path()).unwrap();
        assert_eq!(
            picked.file_name().unwrap().to_string_lossy(),
            "26.730.61639"
        );
    }

    #[test]
    fn pick_newest_version_dir_none_when_no_candidates() {
        let dir = TempDir::new().unwrap();
        assert!(pick_newest_version_dir(dir.path()).is_none());
    }

    #[test]
    fn plugin_dir_of_latest_extracts_prefix() {
        let p = Path::new(
            r"C:\Users\Admin\.codex\plugins\cache\openai-bundled\chrome\latest\extension-host\windows\x64\extension-host.exe",
        );
        assert_eq!(
            plugin_dir_of_latest(p).unwrap().to_string_lossy(),
            r"C:\Users\Admin\.codex\plugins\cache\openai-bundled\chrome"
        );
        assert!(plugin_dir_of_latest(Path::new(r"C:\x\y\z.exe")).is_none());
    }

    #[test]
    fn latest_state_missing_and_real_dir() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            latest_state(&dir.path().join("latest")),
            LatestState::Missing
        );
        std::fs::create_dir(dir.path().join("latest")).unwrap();
        assert_eq!(
            latest_state(&dir.path().join("latest")),
            LatestState::NotAReparsePoint
        );
    }

    #[test]
    fn manifest_json_contains_origins_and_clean_path() {
        let json = manifest_json(
            "com.openai.codexextension",
            &["hehggadaopoacecdllhhajmbjkdcmajg".into()],
            r"\\?\C:\a\extension-host.exe",
        );
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "stdio");
        assert_eq!(v["name"], "com.openai.codexextension");
        assert_eq!(v["path"], r"C:\a\extension-host.exe");
        assert_eq!(
            v["allowed_origins"][0],
            "chrome-extension://hehggadaopoacecdllhhajmbjkdcmajg/"
        );
    }

    #[test]
    fn repair_in_without_registration_file_reports_no_registration() {
        let dir = TempDir::new().unwrap();
        let report = repair_in(dir.path());
        assert_eq!(report.status, "no-registration");
    }

    #[cfg(windows)]
    #[test]
    fn junction_round_trip_created_ok_and_dangling() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("chrome");
        write_version_dir(&plugin_dir, "1.0.1", "payload.txt");

        // 缺失 → created
        let (action, target) = repair_latest_in(&plugin_dir).unwrap();
        assert_eq!(action, "created");
        assert_eq!(
            target.unwrap().file_name().unwrap().to_string_lossy(),
            "1.0.1"
        );
        let latest = plugin_dir.join("latest");
        assert!(latest.join("payload.txt").exists());

        // 完好 → already-ok
        let (action, target) = repair_latest_in(&plugin_dir).unwrap();
        assert_eq!(action, "already-ok");
        assert!(target.is_none());

        // junction 实际指向 1.0.1；删除目标 → 悬空 → 重建到剩余的最新版本目录
        let old_target = plugin_dir.join("1.0.1");
        write_version_dir(&plugin_dir, "1.0.2", "payload.txt");
        std::fs::remove_dir_all(&old_target).unwrap();
        assert_eq!(latest_state(&latest), LatestState::Dangling);
        let (action, target) = repair_latest_in(&plugin_dir).unwrap();
        assert_eq!(action, "recreated");
        assert_eq!(
            target.unwrap().file_name().unwrap().to_string_lossy(),
            "1.0.2"
        );
        assert!(latest.join("payload.txt").exists());
    }

    #[cfg(windows)]
    #[test]
    fn repair_in_rebuilds_latest_from_registration_file() {
        // 注册文件无 nativeHostNames 时不触注册表，可安全在单测中跑全流程
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let plugin_dir = home
            .join("plugins")
            .join("cache")
            .join("openai-bundled")
            .join("chrome");
        // 按注册文件的真实结构落载荷：宿主 exe 与 browser-client 深层路径
        let host_dir = plugin_dir
            .join("26.730.61309")
            .join("extension-host")
            .join("windows")
            .join("x64");
        std::fs::create_dir_all(&host_dir).unwrap();
        std::fs::write(host_dir.join("extension-host.exe"), b"exe").unwrap();
        let scripts_dir = plugin_dir.join("26.730.61309").join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        std::fs::write(scripts_dir.join("browser-client.mjs"), b"mjs").unwrap();
        let host_path = plugin_dir
            .join("latest")
            .join("extension-host")
            .join("windows")
            .join("x64")
            .join("extension-host.exe");
        let client_path = plugin_dir
            .join("latest")
            .join("scripts")
            .join("browser-client.mjs");
        let reg = serde_json::json!({
            "schemaVersion": 2,
            "entries": [{
                "paths": {
                    "extensionHostPath": host_path.to_string_lossy(),
                    "browserClientPath": client_path.to_string_lossy(),
                }
            }]
        });
        std::fs::write(home.join("chrome-native-hosts-v2.json"), reg.to_string()).unwrap();

        let report = repair_in(home);
        assert_eq!(report.status, "ok", "message: {}", report.message);
        assert_eq!(report.latest_action, "created");
        assert!(report.host_path_ok, "message: {}", report.message);
        assert!(report.client_path_ok, "message: {}", report.message);
        assert!(!report.registry_fixed);
        assert!(client_path.exists(), "client 路径应经 latest 链接可达");
    }
}
