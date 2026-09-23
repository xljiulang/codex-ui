// useCodex 拆分模块：主窗口标题跟随活动 Tab 标题（原 useCodex.ts 的一部分，恢复历史功能）
import { watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { activeTab } from "../useTabs";

/** 无活动 tab 时的基础标题（由 boot.ts 写入：`Codex UI v<版本>`，getVersion 失败回退 `Codex UI`） */
let baseTitle = "Codex UI";

/** 设置无活动 tab 时的基础标题 */
export function setWindowBaseTitle(title: string): void {
  baseTitle = title;
}

/** 更新主窗口标题为当前活动 tab 的标题；无活动 tab 时回退基础标题。非 Tauri 环境静默忽略。 */
export async function updateWindowTitle(): Promise<void> {
  try {
    await getCurrentWindow().setTitle(activeTab.value?.title ?? baseTitle);
  } catch {
    // 非 Tauri 环境（如浏览器预览）忽略
  }
}

/** 是否为主窗口（label === "main"）；非 Tauri 环境（浏览器预览/单测）按主窗口处理 */
function isMainWindow(): boolean {
  try {
    return getCurrentWindow().label === "main";
  } catch {
    return true;
  }
}

// 标题自动跟随活动 tab（仅主窗口生效）：活动 tab 身份或标题变化时刷新，
// 覆盖新建/切换/重命名/AI 标题/工作区变化，无须逐个动作点补调用。与旧版模块级 watch 一致。
if (isMainWindow()) {
  watch(
    () => [activeTab.value?.id ?? "", activeTab.value?.title ?? ""].join("|"),
    () => void updateWindowTitle(),
  );
}
