// useCodex 拆分模块：历史会话列表与标题（原 useCodex.ts 的一部分，纯移动，行为不变）
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { buildTurnInput } from "../../lib/mention";
import type { ThreadSummary } from "../../lib/types";
import { getPinCapability, getTitleHelperCapability, sortThreads } from "./capabilities";
import { resolveSessionWorkspace } from "./items";
import { findSessionTabByThread, sessionTabTitle } from "./sessionState";
import { backgroundThreadIds, store } from "./store";
import { setToast, toastError } from "./toast";


export function isThreadNotFound(e: unknown): boolean {
  return String(e).toLowerCase().includes("thread not found");
}


/** 全量加载历史会话：逐页拉取直至 cursor 为空（防死循环上限 200 页） */
export async function refreshThreads() {
  if (store.loadingHistory) return;
  store.loadingHistory = true;
  const all: ThreadSummary[] = [];
  let cursor: string | null = null;
  try {
    for (let i = 0; i < 200; i++) {
      const res: {
        data: ThreadSummary[];
        nextCursor: string | null;
      } = await invoke("thread_list", {
        limit: 50,
        cursor,
      });
      const page = res.data ?? [];
      all.push(...page);
      cursor = res.nextCursor ?? null;
      if (!cursor || page.length === 0) break;
    }
    store.threads = sortThreads(all);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    store.loadingHistory = false;
  }
}


/** 搜索历史会话（thread/search）：全量翻页，结果写入 store.threads 并附带摘要 */
export async function searchThreads(term: string) {
  const t = term.trim();
  if (!t) {
    clearSearch();
    return;
  }
  if (store.loadingHistory) return;
  store.loadingHistory = true;
  const all: { thread: ThreadSummary; snippet: string }[] = [];
  let cursor: string | null = null;
  try {
    for (let i = 0; i < 200; i++) {
      const res: {
        data: { thread: ThreadSummary; snippet: string }[];
        nextCursor: string | null;
      } = await invoke("codex_rpc", {
        method: "thread/search",
        params: {
          searchTerm: t,
          limit: 50,
          cursor,
          sourceKinds: ["cli", "vscode", "exec", "appServer", "unknown"],
        },
      });
      const page = res.data ?? [];
      all.push(...page);
      cursor = res.nextCursor ?? null;
      if (!cursor || page.length === 0) break;
    }
    const snippets: Record<string, string> = {};
    for (const r of all) snippets[r.thread.id] = r.snippet ?? "";
    store.searchSnippets = snippets;
    store.threads = sortThreads(all.map((r) => r.thread));
    store.searchActive = true;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    store.loadingHistory = false;
  }
}


/** 退出搜索，恢复常规列表 */
export function clearSearch() {
  store.searchActive = false;
  store.searchSnippets = {};
  void refreshThreads();
}


/**
 * 重命名会话；返回是否成功（手动重命名 / 首条消息作标题 / AI 总结写回共用）。
 * source 决定 nameIsFirstMessage 标记：manual（默认）清除标记，
 * first-message 置位（AI 总结可覆盖）；auto-summary 不修改标记（由调用方维护）。
 */
export async function renameThread(
  threadId: string,
  name: string,
  source: "manual" | "first-message" | "auto-summary" = "manual",
): Promise<boolean> {
  const n = name.trim();
  if (!n) return false;
  try {
    await invoke("thread_set_name", { threadId, name: n });
    const t = store.threads.find((x) => x.id === threadId);
    if (t) t.name = n;
    const tab = findSessionTabByThread(threadId);
    if (tab) {
      tab.name = n;
      if (source !== "auto-summary") {
        tab.nameIsFirstMessage = source === "first-message";
      }
      tab.title = sessionTabTitle(tab);
    }
    return true;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}


/** 清洗模型生成的标题：去引号/Markdown 标记、折叠空白、截断 50 字 */
export function sanitizeTitle(raw: string): string {
  const t = raw
    .replace(/[`*_#>]/g, "")
    .replace(/^["'“”‘’\s]+|["'“”‘’\s]+$/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!t) return "";
  return t.length > 50 ? t.slice(0, 50) : t;
}


/**
 * 仿 VS Code：临时线程总结首条消息，为会话生成短标题（不指定模型，用默认模型）。
 * 与主回合并行执行、失败静默（保留默认标题）；支持 experimentalApi 才执行，
 * 不支持 ephemeral 时退化为普通线程总结后删除。
 */
export async function autoTitleThread(threadId: string, firstMessagePlain: string) {
  const text = firstMessagePlain.replace(/\s+/g, " ").trim();
  if (!text || text.length <= 15) return; // 短文保持默认标题，不消耗模型
  const tab = findSessionTabByThread(threadId);
  // 仅当尚无名称、或名称来自首条消息（可被总结覆盖）时继续；手动命名不覆盖
  if (tab?.name && !tab.nameIsFirstMessage) return;
  const t = store.threads.find((x) => x.id === threadId);
  if ((!tab?.name || !tab.nameIsFirstMessage) && (t?.name || tab?.name)) return;
  const cap = await getTitleHelperCapability();
  if (!cap?.experimentalApi) return; // 不支持 experimentalApi：不总结

  let helperThreadId: string | null = null;
  let helperTurnId: string | null = null;
  let settled = false;
  const oneOff: UnlistenFn[] = [];
  let titleText = "";
  const titleByItem = new Map<string, string>();
  let timer: ReturnType<typeof setTimeout> | null = null;

  const releaseOneOff = () => {
    for (const un of oneOff) {
      try {
        un();
      } catch {
        // 忽略注销失败
      }
    }
    oneOff.length = 0;
  };

  const cleanup = async () => {
    if (!helperThreadId) return;
    backgroundThreadIds.delete(helperThreadId);
    try {
      if (cap.ephemeral) {
        await invoke("codex_rpc", {
          method: "thread/unsubscribe",
          params: { threadId: helperThreadId },
        });
      } else {
        await invoke("thread_delete", { threadId: helperThreadId });
      }
    } catch {
      // 清理失败不影响主会话
    }
  };

  const finish = async (status?: string) => {
    if (settled) return;
    settled = true;
    if (timer) clearTimeout(timer);
    releaseOneOff();
    if (status === "completed") {
      const title = sanitizeTitle(titleText);
      if (title) {
        // 目标线程若已在总结期间被手动命名，不再覆盖；首条消息名可覆盖
        const cur = findSessionTabByThread(threadId);
        if (!cur?.name || cur.nameIsFirstMessage) {
          if (await renameThread(threadId, title, "auto-summary")) {
            if (cur) cur.nameIsFirstMessage = false;
            setToast("当前会话的标题已简化");
            // 历史列表同步最终标题（搜索态下不覆盖搜索结果）
            if (!store.searchActive) void refreshThreads();
          }
        }
      }
    }
    await cleanup();
  };

  try {
    const startParams: Record<string, unknown> = {
      cwd: resolveSessionWorkspace(),
      approvalPolicy: "never",
      sandbox: "read-only",
    };
    if (cap.ephemeral) {
      startParams.ephemeral = true;
    }
    const started = await invoke<{ thread?: { id?: string } }>("thread_start", {
      params: startParams,
    });
    helperThreadId = started?.thread?.id ?? null;
    if (!helperThreadId) return;
    backgroundThreadIds.add(helperThreadId);

    oneOff.push(
      await listen("item/agentMessage/delta", (e) => {
        const p = e.payload as {
          threadId?: string;
          itemId?: string;
          delta?: string;
        };
        if (p.threadId !== helperThreadId || !p.itemId) return;
        titleByItem.set(
          p.itemId,
          (titleByItem.get(p.itemId) ?? "") + (p.delta ?? ""),
        );
        // 标题取最后一个 agentMessage 的累积文本
        titleText = [...titleByItem.values()].pop() ?? titleText;
      }),
      await listen("turn/started", (e) => {
        const p = e.payload as {
          threadId?: string;
          turn?: { id?: string };
        };
        if (p.threadId !== helperThreadId) return;
        helperTurnId = p.turn?.id ?? null;
      }),
      await listen("turn/completed", (e) => {
        const p = e.payload as {
          threadId?: string;
          turn?: { id?: string; status?: string };
        };
        if (p.threadId !== helperThreadId) return;
        void finish(p.turn?.status);
      }),
    );

    // 超时兜底：尽力中断临时回合并清理，标题保持默认
    timer = setTimeout(() => {
      if (settled) return;
      void (async () => {
        if (helperThreadId && helperTurnId) {
          try {
            await invoke("turn_interrupt", {
              threadId: helperThreadId,
              turnId: helperTurnId,
            });
          } catch {
            // 中断失败不阻塞清理
          }
        }
        await finish();
      })();
    }, 30_000);

    const input = buildTurnInput(
      `给下面用户消息生成一个不超过 30 字的中文会话标题，只输出标题本身，不要任何解释、引号或 Markdown。\n\n用户消息：\n${text}`,
      [],
    );
    await invoke("turn_start", {
      params: {
        threadId: helperThreadId,
        input,
        approvalPolicy: "never",
        sandboxPolicy: { type: "readOnly", networkAccess: false },
      },
    });
  } catch {
    settled = true;
    releaseOneOff();
    await cleanup();
  }
}


/** 固定/取消固定会话（置顶） */
export async function togglePin(threadId: string, pinned: boolean) {
  const t = store.threads.find((x) => x.id === threadId);
  const prev = t?.isPinned;
  try {
    const cap = await getPinCapability();
    if (!cap || cap.protocol === "unsupported") {
      setToast("当前 Codex 版本不支持置顶");
      return;
    }
    if (t) t.isPinned = pinned;
    if (cap.protocol === "metadata_is_pinned") {
      // 旧版协议：isPinned 布尔元数据
      await invoke("codex_rpc", {
        method: "thread/metadata/update",
        params: { threadId, isPinned: pinned },
      });
    } else {
      const sectionId = pinned ? cap.pinnedSectionId : null;
      if (pinned && !sectionId) {
        if (t) t.isPinned = prev;
        setToast("当前 Codex 版本不支持置顶");
        return;
      }
      if (cap.protocol === "section_move") {
        // 新版协议：threadSection/move（0.147+ 改名为 thread/section/move，按探测结果调用）
        const method = cap.sectionMoveMethod ?? "threadSection/move";
        await invoke("codex_rpc", {
          method,
          params: { threadId, sectionId },
        });
      } else {
        // 分区时代协议：metadata/update 携带 sectionId
        await invoke("codex_rpc", {
          method: "thread/metadata/update",
          params: { threadId, sectionId },
        });
      }
    }
    await refreshThreads();
  } catch (e) {
    if (t) t.isPinned = prev;
    setToast(toastError(e));
  }
}
