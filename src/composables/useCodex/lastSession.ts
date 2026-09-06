// useCodex 拆分模块：最后活跃会话 id 记录与退出落盘（settings.json 的 last_session_id）
import { watch } from "vue";
import { TabKind } from "../../lib/tabs";
import { activeTab } from "../useTabs";
import { saveSettings } from "./settings";
import { store } from "./store";

/** 会话标签激活时记录到内存（不落盘）；退出时经 flushLastSession 写入配置文件 */
let activeSessionId = "";

/** 测试用：清空内存记录（不影响已注册的跟踪 watch） */
export function __resetLastSessionForTest(): void {
  activeSessionId = "";
}

/** 读取配置文件中持久化的最后活跃会话 id（上次退出时写入）；无记录返回空串 */
export function readLastSessionId(): string {
  return store.settings.last_session_id ?? "";
}

/**
 * 退出时把内存记录落盘到 settings.json 的 last_session_id：
 * 本次运行未激活过会话或值未变化则跳过，避免多余配置文件写入。
 */
export async function flushLastSession(): Promise<void> {
  if (!activeSessionId || readLastSessionId() === activeSessionId) return;
  try {
    await saveSettings({ last_session_id: activeSessionId });
  } catch {
    // 落盘失败仅影响下次启动恢复，不打断退出流程
  }
}

let tracking = false;

/**
 * 跟踪会话标签激活并记录最后活跃会话 id（深 watch 覆盖新建会话后 threadId
 * 补写的场景）。仅写内存不落盘：避免每次切换会话标签都写配置文件；
 * 真正落盘发生在托盘「退出」收尾时（useCloseGuard 调 flushLastSession）。
 */
export function trackLastSession(): void {
  if (tracking) return;
  tracking = true;
  watch(
    activeTab,
    (tab) => {
      if (tab && tab.kind === TabKind.Session && tab.threadId) {
        activeSessionId = tab.threadId;
      }
    },
    { deep: true },
  );
}
