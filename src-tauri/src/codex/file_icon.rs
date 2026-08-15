//! 从 Windows Shell 提取文件类型系统图标，编码为 PNG data URI。
//!
//! 仅 Windows 生效（项目目标平台）；其它平台提供返回 `Ok(None)` 的桩实现，
//! 保证代码可在非 Windows 环境编译。前端在取不到图标时回退到内置 SVG。
//! 文件夹图标不在本模块范围（前端继续使用内置 SVG）。

use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

/// 列表图标默认请求尺寸（CSS 显示 14px，取 16px 保证清晰度）
const DEFAULT_ICON_SIZE: u32 = 16;

/// SHGetFileInfo 非线程安全（共享系统镜像列表），进程级串行化所有取图标调用；
/// 图标提取为亚毫秒级，串行开销可忽略。
static ICON_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 提取 `path` 对应文件类型图标并返回 PNG data URI；
/// 任何提取失败返回 `Ok(None)`（不中断批处理，前端回退 SVG）。
#[cfg(windows)]
pub fn icon_data_uri(path: &Path, size: u32) -> Result<Option<String>, String> {
    let size = if size == 0 { DEFAULT_ICON_SIZE } else { size };
    let _guard = ICON_LOCK
        .lock()
        .map_err(|_| "图标提取锁异常".to_string())?;
    unsafe { icon_data_uri_impl(path, size, false) }
}

#[cfg(not(windows))]
pub fn icon_data_uri(_path: &Path, _size: u32) -> Result<Option<String>, String> {
    Ok(None)
}

/// 按扩展名提取系统文件图标（文件无需存在）：配合 SHGFI_USEFILEATTRIBUTES，
/// Shell 按传入属性而非磁盘文件解析类型关联，用于菜单等按类型取图场景。
#[cfg(windows)]
pub fn icon_data_uri_for_ext(ext: &str, size: u32) -> Result<Option<String>, String> {
    let probe = format!("__codex_icon_probe{ext}");
    let size = if size == 0 { DEFAULT_ICON_SIZE } else { size };
    let _guard = ICON_LOCK
        .lock()
        .map_err(|_| "图标提取锁异常".to_string())?;
    unsafe { icon_data_uri_impl(Path::new(&probe), size, true) }
}

#[cfg(not(windows))]
pub fn icon_data_uri_for_ext(_ext: &str, _size: u32) -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(windows)]
unsafe fn icon_data_uri_impl(
    path: &Path,
    size: u32,
    use_file_attributes: bool,
) -> Result<Option<String>, String> {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    // SHGetFileInfo 需要线程内 COM 初始化；MTA 足够且适合阻塞线程。
    // RPC_E_CHANGED_MODE（已以其它模式初始化）视为成功，且无需 CoUninitialize。
    const RPC_E_CHANGED_MODE: i32 = -2_147_417_850; // 0x80010106
    let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
    let initialized = hr.is_ok();
    if !initialized && hr.0 != RPC_E_CHANGED_MODE {
        return Err(format!("COM 初始化失败: {hr:?}"));
    }

    let result = shell_file_icon_data_uri(path, size, use_file_attributes);
    if initialized {
        windows::Win32::System::Com::CoUninitialize();
    }
    result
}

/// 通过 SHGetFileInfoW 取文件类型图标 → RGBA → PNG → data URI。
#[cfg(windows)]
unsafe fn shell_file_icon_data_uri(
    path: &Path,
    size: u32,
    use_file_attributes: bool,
) -> Result<Option<String>, String> {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows::Win32::UI::Shell::{
        SHGetFileInfoW, SHFILEINFOW, SHGFI_FLAGS, SHGFI_ICON, SHGFI_SMALLICON,
        SHGFI_USEFILEATTRIBUTES,
    };
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut info = SHFILEINFOW::default();
    let ret = SHGetFileInfoW(
        PCWSTR(wide.as_ptr()),
        FILE_ATTRIBUTE_NORMAL,
        Some(&mut info),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON
            | SHGFI_SMALLICON
            | if use_file_attributes {
                SHGFI_USEFILEATTRIBUTES
            } else {
                SHGFI_FLAGS(0)
            },
    );
    if ret == 0 || info.hIcon.is_invalid() {
        return Ok(None);
    }
    let icon = info.hIcon;
    let out = icon_to_data_uri(icon, size);
    let _ = DestroyIcon(icon);
    out
}

/// HICON → 缩放 → PNG data URI。
///
/// 优先读 32bpp 颜色位图（alpha 直通，逐像素 BGRA→RGBA）；
/// 低色深/单色旧图标（如无关联程序的通用文档图标）走 DrawIconEx 合成
/// （结果像素为预乘 alpha，读回时按 alpha 还原直通值）。
#[cfg(windows)]
unsafe fn icon_to_data_uri(
    icon: windows::Win32::UI::WindowsAndMessaging::HICON,
    target: u32,
) -> Result<Option<String>, String> {
    use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HGDIOBJ};
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let mut ii = ICONINFO::default();
    let got_info = GetIconInfo(icon, &mut ii).is_ok();

    let direct = if got_info && !ii.hbmColor.is_invalid() {
        let mut bmp = BITMAP::default();
        let got = GetObjectW(
            HGDIOBJ(ii.hbmColor.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut BITMAP as *mut std::ffi::c_void),
        );
        if got != 0 && bmp.bmBitsPixel == 32 && bmp.bmWidth > 0 && bmp.bmHeight > 0 {
            read_dib_bitmap(ii.hbmColor, bmp.bmWidth as u32, bmp.bmHeight as u32).ok()
        } else {
            None
        }
    } else {
        None
    };
    if got_info {
        let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
    }

    let img = match direct {
        Some(img) => img,
        None => match draw_icon_composited(icon, target)? {
            Some(img) => img,
            None => return Ok(None),
        },
    };
    encode_png_data_uri(img, target)
}

/// 从 32bpp 位图直接读出自顶向下 BGRA，转 RGBA（直通 alpha）。
#[cfg(windows)]
unsafe fn read_dib_bitmap(
    hbm: windows::Win32::Graphics::Gdi::HBITMAP,
    w: u32,
    h: u32,
) -> Result<image::RgbaImage, String> {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DeleteDC, DIB_RGB_COLORS,
        GetDIBits, RGBQUAD,
    };

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32), // 自顶向下
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default()],
    };
    let hdc = CreateCompatibleDC(None);
    if hdc.is_invalid() {
        return Err("创建兼容 DC 失败".into());
    }
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    let lines = GetDIBits(
        hdc,
        hbm,
        0,
        h,
        Some(bgra.as_mut_ptr() as *mut c_void),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    let _ = DeleteDC(hdc);
    if lines != h as i32 {
        return Err("读取位图失败".into());
    }

    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| "像素缓冲无效".into())
}

/// 用 DrawIconEx 把任意格式图标（含 AND/XOR 掩码）合成到 32bpp DIB 后读回。
/// 输出像素为预乘 BGRA，按 alpha 还原为直通 RGBA。
#[cfg(windows)]
unsafe fn draw_icon_composited(
    icon: windows::Win32::UI::WindowsAndMessaging::HICON,
    target: u32,
) -> Result<Option<image::RgbaImage>, String> {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DeleteDC,
        DeleteObject, DIB_RGB_COLORS, HGDIOBJ, RGBQUAD, SelectObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DrawIconEx};

    let hdc = CreateCompatibleDC(None);
    if hdc.is_invalid() {
        return Ok(None);
    }
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: target as i32,
            biHeight: -(target as i32), // 自顶向下
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default()],
    };
    let hbmp = match CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
        Ok(b) => b,
        Err(_) => {
            let _ = DeleteDC(hdc);
            return Ok(None);
        }
    };
    let old = SelectObject(hdc, HGDIOBJ(hbmp.0));
    let drawn = DrawIconEx(hdc, 0, 0, icon, target as i32, target as i32, 0, None, DI_NORMAL);
    // 必须先读 DIB 内存再释放位图对象（DeleteObject 后 bits 即失效）
    let mut rgba: Option<image::RgbaImage> = None;
    if drawn.is_ok() && !bits.is_null() {
        let n = (target * target * 4) as usize;
        let bgra = std::slice::from_raw_parts(bits as *const u8, n);
        let mut out = Vec::with_capacity(n);
        for px in bgra.chunks_exact(4) {
            let a = px[3] as u32;
            let (r, g, b) = if a == 0 {
                (0u8, 0u8, 0u8)
            } else {
                (
                    ((px[2] as u32 * 255) / a).min(255) as u8,
                    ((px[1] as u32 * 255) / a).min(255) as u8,
                    ((px[0] as u32 * 255) / a).min(255) as u8,
                )
            };
            out.extend_from_slice(&[r, g, b, px[3]]);
        }
        rgba = image::RgbaImage::from_raw(target, target, out);
    }
    let _ = SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(hbmp.0));
    let _ = DeleteDC(hdc);
    Ok(rgba)
}

/// RGBA →（必要时缩放）→ PNG → data URI
#[cfg(windows)]
fn encode_png_data_uri(img: image::RgbaImage, target: u32) -> Result<Option<String>, String> {
    use image::ImageEncoder;

    let img = if img.width() != target || img.height() != target {
        image::imageops::resize(&img, target, target, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut png: Vec<u8> = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut png);
    enc.write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| format!("PNG 编码失败: {e}"))?;
    Ok(Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png)
    )))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn png_magic(uri: &str) -> bool {
        let data = uri
            .strip_prefix("data:image/png;base64,")
            .unwrap_or_default();
        let bytes = STANDARD.decode(data).unwrap_or_default();
        bytes.starts_with(&[0x89, b'P', b'N', b'G'])
    }

    #[test]
    fn icon_for_real_file_is_png() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("sample.rs");
        std::fs::write(&f, "fn main() {}").unwrap();
        let uri = icon_data_uri(&f, 16)
            .unwrap()
            .expect("应取到文件类型图标");
        assert!(uri.starts_with("data:image/png;base64,"));
        assert!(png_magic(&uri));
    }

    #[test]
    fn txt_icon_is_png() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.txt");
        std::fs::write(&f, "a").unwrap();
        let uri = icon_data_uri(&f, 16).unwrap().expect("应取到 txt 图标");
        assert!(png_magic(&uri));
    }

    #[test]
    fn missing_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no_such_file.xyz");
        assert!(icon_data_uri(&missing, 16).unwrap().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn icon_for_ext_is_png() {
        let uri = icon_data_uri_for_ext(".txt", 16)
            .unwrap()
            .expect("应按扩展名取到 txt 图标");
        assert!(uri.starts_with("data:image/png;base64,"));
        assert!(png_magic(&uri));
    }
}
