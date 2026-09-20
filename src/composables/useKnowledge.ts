// 知识库设置分区状态：列表（按工作目录一对一）+ 删除 + 建库进度。
// 数据在 <app data dir>/knowledge/：kbs/*.sqlite 为各工作目录的索引库，
// model/ 为随包向量模型；本模块只做命令转发与事件驱动的列表刷新。
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { setToast, toastError } from "./useCodex";

/** 列表行（对应 Rust `KbSummary`） */
export interface KnowledgeKb {
  file: string;
  cwd: string;
  docs: number;
  chunks: number;
  updated_at: number;
  available: boolean;
  error: string | null;
}

/** 建库进度事件（对应 Rust `IndexProgress`） */
interface KnowledgeProgressEvent {
  cwd: string;
  phase: string;
  processed: number;
  total: number;
  current: string;
}

/** 建库完成事件（成功带 summary、失败带 error） */
interface KnowledgeDoneEvent {
  cwd: string;
  summary?: { text?: string };
  error?: string;
}

export const knowledgeKbs = ref<KnowledgeKb[]>([]);
export const knowledgeLoading = ref(false);
/** 进行中的建库进度：规范化工作目录 → 一行文案 */
export const knowledgeProgress = ref<Record<string, string>>({});

function normalizeKey(cwd: string): string {
  return cwd.trim().replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
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

/** 删除某个知识库（只删索引，不动原始文档与模型） */
export async function deleteKnowledge(file: string): Promise<void> {
  await invoke("knowledge_delete", { file });
  await refreshKnowledgeList();
}

/** 建库/增量更新（设置页无入口，这里保留给测试与后续手工触发） */
export async function startKnowledgeIndex(
  cwd: string,
  paths?: string[],
  full?: boolean,
): Promise<string> {
  const summary = await invoke<{ text?: string }>("knowledge_index_start", {
    cwd,
    paths,
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
      const { cwd, phase, processed, total, current } = e.payload;
      if (phase === "done") {
        const next = { ...knowledgeProgress.value };
        delete next[normalizeKey(cwd)];
        knowledgeProgress.value = next;
        return;
      }
      const text = current
        ? `建库中 ${processed}/${total}（${baseName(current)}）`
        : `建库中 ${processed}/${total}`;
      knowledgeProgress.value = { ...knowledgeProgress.value, [normalizeKey(cwd)]: text };
    },
  );
  const unlistenDone = await listen<KnowledgeDoneEvent>(
    "knowledge/index-done",
    (e) => {
      const { cwd } = e.payload;
      const next = { ...knowledgeProgress.value };
      delete next[normalizeKey(cwd)];
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
