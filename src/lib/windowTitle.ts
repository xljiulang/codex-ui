import { getCurrentWindow } from "@tauri-apps/api/window";
import { pathBaseName } from "./format";

/** 独立窗口标题栏显示文件名（不含路径）；非 Tauri 环境（单测/浏览器）静默忽略 */
export async function setWindowTitleFromPath(path: string): Promise<void> {
  try {
    await getCurrentWindow().setTitle(pathBaseName(path));
  } catch {
    // 忽略
  }
}
