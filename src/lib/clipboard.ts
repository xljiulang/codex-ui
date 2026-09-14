import { invoke } from "@tauri-apps/api/core";

/**
 * 统一剪贴板复制：优先 navigator.clipboard，失败降级 textarea + execCommand，
 * 覆盖 WebView2/浏览器差异，供所有复制入口共用。
 */
export async function copyText(text: string): Promise<boolean> {
  if (!text) return false;
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(ta);
      return ok;
    } catch {
      return false;
    }
  }
}

/**
 * 复制图片到系统剪贴板（灯箱右键「复制图像」）。
 * `source` 传本地文件路径或 `data:` URL：后端解码后以 CF_DIB（+ CF_BITMAP）写入剪贴板，
 * 可直接粘贴到聊天/文档；远程 http(s) 图片后端会明确报错（不做下载）。
 * 失败时抛出后端错误（调用方据此提示原因），成功返回 true。
 */
export async function copyImage(source: string): Promise<boolean> {
  if (!source) throw new Error("图片来源为空");
  await invoke("clipboard_write_image", { source });
  return true;
}
