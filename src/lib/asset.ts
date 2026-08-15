import { convertFileSrc } from "@tauri-apps/api/core";

/** 本地路径 → Tauri asset URL；转换失败时回退原路径（路径展示兜底）。 */
export function assetUrl(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}
