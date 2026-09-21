// 知识库设置分区状态：列表（库名 + 来源目录）+ 删除 + 建库进度。
// 数据由 codexui-kb.exe 管理（%APPDATA%\com.codexui.app\kbs），
// 索引/检索由 skill 调用 CLI 完成；本模块只做命令转发与事件驱动的列表刷新。
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { setToast, toastError } from "./useCodex";

/** 列表行（对应 Rust `KbSummary`） */
export interface KnowledgeKb {
  file: string;
  kb: string;
  source: string;
  docs: number;
  chunks: number;
  updated_at: number;
  available: boolean;
  error: string | null;
}

/** 建库进度事件（对应 Rust `IndexProgress`） */
interface KnowledgeProgressEvent {
  kb: string;
  phase: string;
  processed: number;
  total: number;
  current: string;
}

/** 建库完成事件（成功带 summary、失败带 error） */
interface KnowledgeDoneEvent {
  kb: string;
  summary?: { text?: string };
  error?: string;
}

export const knowledgeKbs = ref<KnowledgeKb[]>([]);
export const knowledgeLoading = ref(false);
/** 进行中的建库进度：规范化库名 → 一行文案 */
export const knowledgeProgress = ref<Record<string, string>>({});

/** 进度键：与后端 kb_key 同口径（去首尾空白、折叠内部空白、小写） */
export function normalizeKbKey(kb: string): string {
  return kb.trim().split(/\s+/).join(" ").toLowerCase();
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** 读取知识库列表（按上次更新时间倒序，由后端排序） */
export async function refreshKnowledgeList(): Promise<void> {
  knowledgeLoading.value = true;
  try {
    const rows = await invoke<KnowledgeKb[] | null | undefined>("knowledge_list");
    knowledgeKbs.value = Array.isArray(rows) ? rows : [];
  } catch (e) {
    setToast(toastError(e));
  } finally {
    knowledgeLoading.value = false;
  }
}

/**
 * 删除某个知识库（只删索引，不动原始文档与模型）：
 * 优先按库名删；旧版/损坏库没有库名时按库文件名删。
 */
export async function deleteKnowledge(kb: KnowledgeKb): Promise<void> {
  await invoke("knowledge_delete", {
    kb: kb.kb || undefined,
    file: kb.file,
  });
  await refreshKnowledgeList();
}

/** 建库/增量更新（设置页无入口，保留给命令层与测试；skill 侧直接调 CLI） */
export async function startKnowledgeIndex(
  kb: string,
  source: string,
  full?: boolean,
): Promise<string> {
  const summary = await invoke<{ text?: string }>("knowledge_index_start", {
    kb,
    source,
    full,
  });
  return summary?.text ?? "";
}

/**
 * 订阅建库进度/完成事件：进度写进 `knowledgeProgress` 供列表行显示，
 * 完成时刷新列表并 toast 结果。返回取消订阅函数。
 */
export async function subscribeKnowledgeEvents(): Promise<UnlistenFn> {
  const unlistenProgress = await listen<KnowledgeProgressEvent>(
    "knowledge/index-progress",
    (e) => {
      const { kb, phase, processed, total, current } = e.payload;
      if (phase === "done") {
        const next = { ...knowledgeProgress.value };
        delete next[normalizeKbKey(kb)];
        knowledgeProgress.value = next;
        return;
      }
      const text = current
        ? `建库中 ${processed}/${total}（${baseName(current)}）`
        : `建库中 ${processed}/${total}`;
      knowledgeProgress.value = { ...knowledgeProgress.value, [normalizeKbKey(kb)]: text };
    },
  );
  const unlistenDone = await listen<KnowledgeDoneEvent>(
    "knowledge/index-done",
    (e) => {
      const { kb } = e.payload;
      const next = { ...knowledgeProgress.value };
      delete next[normalizeKbKey(kb)];
      knowledgeProgress.value = next;
      if (e.payload.error) setToast(`建库失败：${e.payload.error}`);
      else if (e.payload.summary?.text) setToast(e.payload.summary.text);
      void refreshKnowledgeList();
    },
  );
  return () => {
    unlistenProgress();
    unlistenDone();
  };
}
