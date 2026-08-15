import { nextTick, onBeforeUnmount, ref, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { setToast, toastError } from "./useCodex";
import { baseName, toUserAttachment } from "../lib/mention";
import type { UserInput } from "../lib/types";

/** 粘贴的截图/位图最大字节数（原路径文件不受限） */
const MAX_PASTED_IMAGE_BYTES = 20 * 1024 * 1024;

/**
 * 附件摄取：粘贴/拖放文件 → 附件区（与编辑器内联引用分开的“行附件”）。
 * 组件持有 rowAttachments 状态，通过 syncAttachments 回调同步到 store。
 */
export function useComposerAttachments(options: {
  rowAttachments: Ref<UserInput[]>;
  syncAttachments: () => void;
}) {
  const dragging = ref(false);
  let dropUnlisten: UnlistenFn | undefined;

  /** 从剪贴板 MIME/文件名推断图片扩展名（白名单与后端 save_pasted_image 一致） */
  function imageExtFromType(type: string, fallbackName: string): string {
    const norm = (ext: string) => (ext === "jpeg" ? "jpg" : ext);
    const m = /^image\/(png|jpe?g|gif|webp|bmp)$/i.exec(type);
    if (m) return norm(m[1].toLowerCase());
    const fn = /\.(png|jpe?g|gif|webp|bmp)$/i.exec(fallbackName);
    if (fn) return norm(fn[1].toLowerCase());
    return "png";
  }

  /**
   * 图片/文件 → 附件区（粘贴与拖放共用核心逻辑）：
   * - 图片项：优先用原始路径，读不到（截图/网页位图）则落盘；
   * - 非图片文件项：仅支持原始路径，读不到提示暂不支持。
   */
  async function addFilesWithPaths(
    files: File[],
    originalPaths: string[],
    source: "粘贴" | "拖放",
  ) {
    const originalByBase = new Map<string, string>();
    for (const p of originalPaths) {
      const b = baseName(p).toLowerCase();
      if (b && !originalByBase.has(b)) originalByBase.set(b, p);
    }

    let added = 0;
    for (const f of files) {
      const name = f.name || "pasted";
      const orig = originalByBase.get(name.toLowerCase());
      if (f.type.startsWith("image/")) {
        if (orig) {
          options.rowAttachments.value.push(toUserAttachment(name, orig));
          added++;
          continue;
        }
        if (f.size > MAX_PASTED_IMAGE_BYTES) {
          setToast(`${source}的图片过大（>20MB），已跳过`);
          continue;
        }
        try {
          const bytes = new Uint8Array(await f.arrayBuffer());
          const saved = await invoke<string>("save_pasted_image", {
            bytes: Array.from(bytes),
            name: `pasted.${imageExtFromType(f.type, name)}`,
          });
          options.rowAttachments.value.push({ type: "localImage", path: saved });
          added++;
        } catch (e) {
          setToast(toastError(e));
        }
      } else if (orig) {
        options.rowAttachments.value.push(toUserAttachment(name, orig));
        added++;
      } else {
        setToast(`暂不支持该${source}（无法获取原始路径）: ${name}`);
      }
    }
    if (added) {
      options.syncAttachments();
      await nextTick();
    }
  }

  /** 粘贴图片/文件 → 附件区（原始路径来自剪贴板 CF_HDROP） */
  async function handlePastedFiles(files: File[]) {
    let originalPaths: string[] = [];
    try {
      originalPaths = await invoke<string[]>("clipboard_file_paths");
    } catch {
      originalPaths = [];
    }
    await addFilesWithPaths(files, originalPaths, "粘贴");
  }

  /** ProseMirror paste 入口：有文件/图片项则消费事件，否则走默认（文本粘贴） */
  function handlePasteDom(e: ClipboardEvent): boolean {
    const fileItems = Array.from(e.clipboardData?.items ?? []).filter(
      (it) => it.kind === "file",
    );
    if (!fileItems.length) return false;
    // DataTransferItem 只在 paste 事件同步阶段有效：先取出 File，再异步处理
    const files = fileItems
      .map((it) => it.getAsFile())
      .filter((f): f is File => !!f);
    // 取不到 File（如已失效的 DataTransferItem）：放行默认粘贴，避免吞掉文本
    if (!files.length) return false;
    e.preventDefault();
    void handlePastedFiles(files);
    return true;
  }

  /** Tauri 拖放事件：drop 时直接拿到绝对路径数组（WebView2 下 HTML5 dataTransfer.files 为空） */
  async function setupDragDrop() {
    try {
      dropUnlisten = await getCurrentWebview().onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          dragging.value = true;
        } else if (p.type === "leave") {
          dragging.value = false;
        } else if (p.type === "drop") {
          dragging.value = false;
          const paths = p.paths ?? [];
          if (paths.length) void addDroppedPaths(paths);
        }
      });
    } catch {
      // 非 Tauri 环境（浏览器/单测）忽略，走 HTML5 drop 兜底
    }
  }

  /** 拖放路径 → 附件区：图片按扩展名 → localImage，其它 → mention */
  async function addDroppedPaths(paths: string[]) {
    let added = 0;
    for (const path of paths) {
      options.rowAttachments.value.push(
        toUserAttachment(baseName(path) || "dropped", path),
      );
      added++;
    }
    if (added) {
      options.syncAttachments();
      await nextTick();
    }
  }

  function onDragOver() {
    dragging.value = true;
  }

  function onDragLeave(e: DragEvent) {
    const current = e.currentTarget as HTMLElement | null;
    if (!current || !current.contains(e.relatedTarget as Node | null)) {
      dragging.value = false;
    }
  }

  /** 拖放图片/文件 → 附件区（原始路径来自拖放 File.path），纯文本拖放放行 */
  function onDrop(e: DragEvent) {
    dragging.value = false;
    const files = Array.from(e.dataTransfer?.files ?? []);
    if (!files.length) return;
    e.preventDefault();
    const paths = files
      .map((f) => (f as File & { path?: string }).path)
      .filter((p): p is string => !!p);
    void addFilesWithPaths(files, paths, "拖放");
  }

  onBeforeUnmount(() => {
    dropUnlisten?.();
  });

  return {
    dragging,
    handlePasteDom,
    setupDragDrop,
    onDragOver,
    onDragLeave,
    onDrop,
  };
}
