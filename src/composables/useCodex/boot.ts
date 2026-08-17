// useCodex 拆分模块：启动（原 useCodex.ts 的一部分，纯移动，行为不变）
import { getCurrentWindow } from "@tauri-apps/api/window";
import { wireEvents } from "./events";
import { loadModels, loadSettings, refreshServer } from "./settings";
import { store } from "./store";
import { refreshThreads } from "./threads";


/** 启动加载态最长展示时长：防止某个 invoke 挂起导致加载动画永久显示 */
const BOOT_MAX_MS = 15_000;


export async function init() {
  // E2E 测试钩子（与 ComposerBar 暴露 __CODEX_UI_EDITOR__ 同模式）：
  // 无原生目录选择器创建会话（探针在临时目录启动应用后使用）。
  try {
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__ = {
      newSession: async () => {
        const { openNewSession } = await import("./actions");
        await openNewSession();
      },
    };
  } catch {
    // 非浏览器环境忽略
  }
  const bootTimer = window.setTimeout(() => {
    store.booting = false;
  }, BOOT_MAX_MS);
  try {
    // 先拿到工作目录：沙箱可写根与资源/Git 面板需要它；历史列表有意展示全部目录的会话。
    await Promise.all([loadSettings(), refreshServer()]);
    // 主窗口标题固定为 “Codex UI”，与当前会话无关（非 Tauri 环境静默忽略）
    try {
      await getCurrentWindow().setTitle("Codex UI");
    } catch {
      // 忽略非 Tauri 环境
    }
    void loadModels();
    await refreshThreads();
    await wireEvents();
    // 监听注册后补取一次状态：避免后端启动成功的首次推送早于监听注册被丢弃
    void refreshServer().catch(() => undefined);
  } finally {
    window.clearTimeout(bootTimer);
    store.booting = false;
  }
}
