import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError } from "./useCodex";
import { openPathInApp } from "./useSessionFs";

/** 应用内打开文件（文本走编辑器标签），失败回退资源管理器定位（reveal_path） */
export async function openPathInAppOrReveal(path: string) {
  if (!path) return;
  const opened = await openPathInApp(path);
  if (opened) return;
  try {
    await invoke("reveal_path", { path });
  } catch (e) {
    setToast(toastError(e));
  }
}
