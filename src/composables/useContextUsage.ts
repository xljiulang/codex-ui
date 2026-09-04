import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { formatTokens } from "../lib/format";
import { activeSessionTab, setToast, toastError, type SessionTab } from "./useCodex";

/** 上下文窗口使用情况：window 未知时不显示 */
export function useContextUsage(tab?: SessionTab) {
  const compacting = ref(false);

  /** 数据来源：优先使用传入的 tab，缺省回退当前活动会话标签 */
  const source = computed(() => tab ?? activeSessionTab());

  const ctxUsage = computed(() => {
    const u = source.value?.threadTokenUsage;
    if (!u || u.window == null || u.window <= 0) return null;
    return {
      pct: Math.min(100, Math.round((u.used / u.window) * 100)),
      used: u.used,
      window: u.window,
    };
  });

  const ctxTooltip = computed(() => {
    if (compacting.value) return "正在压缩上下文…";
    const u = ctxUsage.value;
    if (!u) return "";
    return `上下文已用 ${formatTokens(u.used)}，共 ${formatTokens(u.window)}，双击进行压缩`;
  });

  /** 发起 thread/compact/start：回合进行中也可压缩，由服务端处理 */
  async function compactNow() {
    const tid = source.value?.threadId;
    if (!tid || compacting.value) return;
    compacting.value = true;
    try {
      await invoke("codex_rpc", {
        method: "thread/compact/start",
        params: { threadId: tid },
      });
      setToast("已开始压缩上下文");
    } catch (e) {
      setToast(toastError(e));
    } finally {
      compacting.value = false;
    }
  }

  return { ctxUsage, ctxTooltip, compacting, compactNow };
}
