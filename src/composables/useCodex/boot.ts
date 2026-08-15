// useCodex 拆分模块：启动（原 useCodex.ts 的一部分，纯移动，行为不变）
import { getCurrentWindow } from "@tauri-apps/api/window";
import { wireEvents } from "./events";
import { ensureThreadPlugins, ensureSkills, loadModels, loadSettings, refreshServer } from "./settings";
import { store } from "./store";
import { refreshThreads } from "./threads";
import { NEW_CHAT_PLUGIN_KEY } from "./types";


/** 启动加载态最长展示时长：防止某个 invoke 挂起导致加载动画永久显示 */
const BOOT_MAX_MS = 15_000;


export async function init() {
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
    void ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY); // 应用启动预初始化新对话插件缓存
    void ensureSkills(); // 应用启动预加载技能列表（$ 菜单与回显悬浮提示共用）
    await refreshThreads();
    await wireEvents();
    // 监听注册后补取一次状态：避免后端启动成功的首次推送早于监听注册被丢弃
    void refreshServer().catch(() => undefined);
  } finally {
    window.clearTimeout(bootTimer);
    store.booting = false;
  }
}
