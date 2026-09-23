//! 主窗口毛玻璃同步：系统 Acrylic（Windows 10/11；Windows 11 由 DWM
//! TransientWindow backdrop 提供系统毛玻璃背景）。
//! codex-ui 仅面向 Windows；窗口常驻透明（transparent），「毛玻璃特效」开关
//! 决定是否应用系统背景效果，关闭时由前端纯色根背景兜底（本模块清除效果）。
//!
//! 说明：不采用 Mica——Tauri transparent 窗口下 `DWMSBT_MAINWINDOW` 会“设置成功
//! 但不渲染”（window-vibrancy issue #141），Acrylic 在透明窗口下可靠。
use tauri::{AppHandle, Manager};

use window_vibrancy::{apply_acrylic, clear_acrylic, clear_mica};

use crate::codex::settings::{self, AppSettings};

/// 按指定设置应用/清除主窗口 Acrylic 毛玻璃；失败仅记录到 stderr，不影响设置保存。
pub(crate) fn apply(app: &AppHandle, s: &AppSettings) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if !s.glass_effect {
        let _ = clear_acrylic(&win);
        let _ = clear_mica(&win); // 兜底：早期版本若曾应用过 Mica
        return;
    }
    let (r, g, b) = settings::glass_tint_rgb(&s.theme);
    // Acrylic tint 用约 37%（0x60/255）主题色叠加：纯模糊过透，纯色会盖住背景
    if let Err(e) = apply_acrylic(&win, Some((r, g, b, 0x60))) {
        eprintln!("应用 Acrylic 毛玻璃失败: {e}");
    }
}

/// 恢复 Windows 11 原生窗口圆角：保留无阴影/透明毛玻璃的同时，通过 DWM
/// 角落偏好（DWMWCP_ROUND）让 DWM 裁剪四角；旧版/不支持时忽略，不阻断启动。
#[cfg(windows)]
pub(crate) fn apply_round_corners(win: &tauri::WebviewWindow) {
    use std::mem::size_of;

    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWM_WINDOW_CORNER_PREFERENCE,
    };

    // Tauri 依赖 windows 0.61，本项目 windows crate 为 0.62；
    // HWND 均为裸指针包装，这里做指针级转换以匹配 0.62 的 DWM API。
    let Ok(hwnd_orig) = win.hwnd() else {
        return;
    };
    let hwnd = windows::Win32::Foundation::HWND(hwnd_orig.0);
    // DWMWCP_ROUND = 2，Windows 11 使用 DWM 系统裁角
    let corner = DWM_WINDOW_CORNER_PREFERENCE(2);
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    };
    if let Err(e) = result {
        eprintln!("设置窗口圆角失败: {e}");
    }
}
