import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError, workspace } from "../useCodex";
import type { FsEntry } from "../../lib/sessionFs";
import { copyBuffer } from "./state";
import { refreshAll } from "./tree";

export function copyEntry(entry: FsEntry) {
  copyBuffer.value = [entry.path];
  setToast(`已复制「${entry.name}」，可在目录上右键粘贴`);
}

async function readClipboardPaths(): Promise<string[]> {
  try {
    return await invoke<string[]>("clipboard_file_paths");
  } catch {
    return [];
  }
}

/** 把内部复制记录（或系统剪贴板文件）粘贴进目标目录 */
export async function pasteInto(targetDir: string) {
  const root = workspace.value;
  if (!root) return;
  const sources = copyBuffer.value.length
    ? [...copyBuffer.value]
    : await readClipboardPaths();
  if (!sources.length) {
    setToast("剪贴板中没有可粘贴的文件或文件夹");
    return;
  }
  try {
    const created = await invoke<FsEntry[]>("session_fs_paste", {
      workspace: root,
      destDir: targetDir,
      sources,
    });
    copyBuffer.value = [];
    setToast(`已粘贴 ${created.length} 项`);
    await refreshAll();
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 粘贴是否可用：内部复制记录非空或系统剪贴板含文件（读取失败按不可用处理） */
export async function pasteAvailable(): Promise<boolean> {
  if (copyBuffer.value.length) return true;
  const paths = await readClipboardPaths();
  return paths.length > 0;
}

/** 在目录下新建文本文件（唯一命名由后端保证），成功后刷新并返回条目；失败返回 null */
export async function createTextFile(dir: string): Promise<FsEntry | null> {
  const root = workspace.value;
  if (!root) return null;
  try {
    const created = await invoke<FsEntry>("session_fs_create_file", {
      workspace: root,
      dir,
    });
    setToast(`已创建「${created.name}」`);
    await refreshAll();
    return created;
  } catch (e) {
    setToast(toastError(e));
    return null;
  }
}

/** 在目录下新建文件夹（唯一命名由后端保证），成功后刷新并返回条目；失败返回 null */
export async function createFolder(dir: string): Promise<FsEntry | null> {
  const root = workspace.value;
  if (!root) return null;
  try {
    const created = await invoke<FsEntry>("session_fs_create_dir", {
      workspace: root,
      dir,
    });
    setToast(`已创建「${created.name}」`);
    await refreshAll();
    return created;
  } catch (e) {
    setToast(toastError(e));
    return null;
  }
}

export async function renameEntry(path: string, newName: string) {
  const root = workspace.value;
  if (!root) return;
  try {
    await invoke<FsEntry>("session_fs_rename", {
      workspace: root,
      path,
      newName,
    });
    await refreshAll();
  } catch (e) {
    setToast(toastError(e));
  }
}

export async function deleteEntry(path: string) {
  const root = workspace.value;
  if (!root) return;
  try {
    await invoke("session_fs_delete", { workspace: root, path });
    await refreshAll();
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 树内拖拽移动：把 src 移动到 destDir 目录下；成功后刷新并提示，返回是否成功 */
export async function moveEntry(
  src: string,
  destDir: string,
): Promise<boolean> {
  const root = workspace.value;
  if (!root) return false;
  try {
    await invoke<FsEntry>("session_fs_move", {
      workspace: root,
      src,
      destDir,
    });
    const name = src.split(/[\\/]/).pop() ?? src;
    setToast(`已移动「${name}」`);
    await refreshAll();
    return true;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}

export function revealInExplorer(path: string) {
  void invoke("reveal_path", { path }).catch((e) => setToast(toastError(e)));
}
