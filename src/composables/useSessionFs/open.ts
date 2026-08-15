import { invoke } from "@tauri-apps/api/core";
import {
  addAttachmentToActiveSession,
  setToast,
  store,
  toastError,
  workspace,
} from "../useCodex";
import { openFileTab, openPreviewTab } from "../useEditorTabs";
import { toUserAttachment } from "../../lib/mention";
import { pathBaseName } from "../../lib/format";
import { previewTypeForName } from "../../lib/preview";
import {
  dirNameOf,
  isPathUnderRoot,
  type FsEntry,
} from "../../lib/sessionFs";

/** 应用内打开文本文件：在主窗口左侧编辑器区打开/激活一个文件标签 */
export function openTextEditor(entry: FsEntry) {
  const root = workspace.value;
  if (!root) return;
  void openFileTab(root, entry.path);
}

/** 应用内打开 PDF 预览：在主窗口左侧编辑器区打开/激活 PDF 预览标签 */
export function openPdfPreview(entry: FsEntry) {
  const root = workspace.value;
  if (!root) return;
  void openPreviewTab("pdf", root, entry.path);
}

/** 应用内打开图像预览：在主窗口左侧编辑器区打开/激活图像预览标签 */
export function openImagePreview(entry: FsEntry) {
  const root = workspace.value;
  if (!root) return;
  void openPreviewTab("image", root, entry.path);
}

/**
 * 打开前探测文件内容是否为文本：true=文本、false=二进制/非文本、
 * null=探测失败（已 toast 错误；无工作目录时静默返回 null）
 */
export async function probeTextEntry(
  entry: FsEntry,
): Promise<boolean | null> {
  const root = workspace.value;
  if (!root) return null;
  try {
    return await invoke<boolean>("session_fs_probe_text", {
      workspace: root,
      path: entry.path,
    });
  } catch (e) {
    setToast(toastError(e));
    return null;
  }
}

/**
 * 对话本地链接：支持则在应用内 tab 打开（PDF/图片 → 预览标签，文本 → 编辑器），
 * 返回 true；否则返回 false，由调用方降级为资源管理器。
 * 工作区外文件以父目录作为根（仅本次读取/打开，不改变会话工作区）。
 * 测试钩子（__CODEX_UI_TEST__）开启时直接返回 false，保持 E2E 现有
 * reveal_path 分发记录不回归。
 */
export async function openPathInApp(path: string): Promise<boolean> {
  const testWin = window as unknown as { __CODEX_UI_TEST__?: boolean };
  if (testWin.__CODEX_UI_TEST__) return false;
  const session = workspace.value;
  if (!session) return false;
  const underSession = isPathUnderRoot(session, path);
  const root = underSession ? session : dirNameOf(path);
  const relPath = underSession ? path : pathBaseName(path);
  if (!root || !relPath) return false;

  const type = previewTypeForName(relPath);
  if (type === "pdf") {
    void openPreviewTab("pdf", root, relPath);
    return true;
  }
  if (type === "image") {
    // 图片走 asset 协议，需要绝对路径；root 仅作标签元数据
    void openPreviewTab("image", root, path);
    return true;
  }
  try {
    const isText = await invoke<boolean>("session_fs_probe_text", {
      workspace: root,
      path: relPath,
    });
    if (!isText) return false;
    void openFileTab(root, relPath);
    return true;
  } catch {
    // 目录/缺失/不可读：降级为资源管理器（原行为），不额外 toast
    return false;
  }
}

/** 添加为会话附件：路由到当前活动会话的 ComposerBar；异常态兜底 push store */
export function addAsAttachment(entry: FsEntry) {
  const a = toUserAttachment(entry.name, entry.path);
  if (!addAttachmentToActiveSession(a)) {
    store.attachments.push(a);
  }
  setToast(`已添加「${entry.name}」为会话附件`);
}
