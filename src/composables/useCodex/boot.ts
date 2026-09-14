// useCodex 拆分模块：启动（原 useCodex.ts 的一部分，纯移动，行为不变）
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { getPinnedSectionId } from "./pinnedSection";
import { wireEvents } from "./events";
import { loadModels, loadSettings, refreshServer } from "./settings";
import { store } from "./store";
import { refreshThreads } from "./threads";
import { setToast } from "./toast";
import { setWindowBaseTitle, updateWindowTitle } from "./windowTitle";
import { trackLastSession } from "./lastSession";
import { loadScheduledTasks } from "./scheduledTasks";


/** 启动加载态最长展示时长：防止某个 invoke 挂起导致加载动画永久显示 */
const BOOT_MAX_MS = 15_000;


export async function init() {
  // E2E 测试钩子（与 ComposerBar 暴露 __CODEX_UI_EDITOR__ 同模式）：
  // 无原生目录选择器创建会话（探针在临时目录启动应用后使用）。
  // 仅当以 CODEX_UI_TEST=1 启动（scripts E2E 启动器设置）才暴露；
  // 正常启动不暴露——避免 openLink/openPathInApp 被测试钩子短路，
  // 导致生产环境文件链接/引用点击无反应。
  try {
    const testMode = await invoke<boolean>("test_hook_enabled");
    if (testMode) {
      (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__ = {
        newSession: async () => {
          const { openNewSession } = await import("./actions");
          await openNewSession();
        },
      };
    }
  } catch {
    // 非 Tauri 环境（浏览器预览）忽略
  }
  const bootTimer = window.setTimeout(() => {
    store.booting = false;
  }, BOOT_MAX_MS);
  try {
    // 先拿到工作目录：沙箱可写根与资源/Git 面板需要它；历史列表有意展示全部目录的会话。
    await Promise.all([loadSettings(), refreshServer()]);
    // 跟踪会话标签激活并记录最后活跃会话 id 到内存（退出时由 useCloseGuard
    // 落盘到 settings.json 的 last_session_id）；设置加载完成后挂载，
    // 避免用默认值覆盖刚读到的持久化值。
    trackLastSession();
    // 预取定时任务列表（此后由 scheduled-tasks/event 快照驱动更新）
    void loadScheduledTasks();
    // 主窗口标题跟随活动 tab；无活动 tab 时回退 “Codex UI v<版本>”。
    // 非 Tauri 环境 getVersion 不可用，回退无版本标题；setTitle 失败同样静默忽略。
    let windowTitle = "Codex UI";
    try {
      const version = await getVersion();
      if (version) windowTitle = `Codex UI v${version}`;
    } catch {
      // 非 Tauri 环境：回退无版本标题
    }
    setWindowBaseTitle(windowTitle);
    void updateWindowTitle();
    void loadModels();
    await refreshThreads();
    await wireEvents();
    // 监听注册后补取一次状态：避免后端启动成功的首次推送早于监听注册被丢弃
    void refreshServer().catch(() => undefined);
    // 预取置顶分区 id（失败静默，首次点击置顶时再重试）
    void getPinnedSectionId();
    // 等待后端探测 codex 版本（最多 5s）：仅低于 0.149.0 时一次性提示
    void (async () => {
      for (let i = 0; i < 25; i++) {
        if (store.server.versionTooOld === true) {
          const v = store.server.codexVersion ?? "未知版本";
          setToast(`当前 codex 版本 ${v} 低于 0.149.0，需要 0.149.0 及以上版本，部分功能可能异常`);
          return;
        }
        if (store.server.versionTooOld === false) return;
        await new Promise((r) => setTimeout(r, 200));
      }
    })();
  } finally {
    window.clearTimeout(bootTimer);
    store.booting = false;
  }
}
