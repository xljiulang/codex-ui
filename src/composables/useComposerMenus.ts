import type { Ref } from "vue";
import { store } from "./useCodex";

/** @ / $ 提及弹层触发状态 */
export type ComposerMention =
  | { kind: "@" | "$"; token: string; start: number }
  | null;

export type ComposerMenu = "perm" | "task" | "model";

/** 输入区三个按钮菜单（权限/任务/模型）的开关与外部点击关闭语义 */
export function useComposerMenus(options: { mention: Ref<ComposerMention> }) {
  function closeMenus() {
    store.permOpen = false;
    store.taskOpen = false;
    store.modelOpen = false;
    options.mention.value = null;
  }

  /** 切换三个按钮菜单：打开一个时关闭另外两个，再点一次当前按钮则关闭 */
  function toggleMenu(which: ComposerMenu) {
    const willOpen = !store[`${which}Open`];
    store.permOpen = false;
    store.taskOpen = false;
    store.modelOpen = false;
    store[`${which}Open`] = willOpen;
  }

  function onKeydownGlobal(e: KeyboardEvent) {
    if (e.key === "Escape") closeMenus();
  }

  function onWindowMousedown(e: MouseEvent) {
    // 三个按钮弹出层：点击外部任意区域自动关闭。用 mousedown 而非 click，
    // 与 GitView 分支弹层一致——WebView2 原生菜单「粘贴」只合成 click、
    // 不合成 mousedown，避免粘贴等操作误关弹层。
    // 用 Element 而非 HTMLElement：点击 svg/path 等 SVG 目标也应正确判断。
    if (!(e.target instanceof Element)) {
      store.permOpen = false;
      store.taskOpen = false;
      store.modelOpen = false;
      return;
    }
    // 弹出层内部与三个触发按钮不自动关闭（按钮自身的 click 负责切换）
    if (
      e.target.closest(".popup-menu") ||
      e.target.closest(".perm-chip, .task-chip, .model-chip, .goal-chip")
    ) {
      return;
    }
    store.permOpen = false;
    store.taskOpen = false;
    store.modelOpen = false;
  }

  return { closeMenus, toggleMenu, onKeydownGlobal, onWindowMousedown };
}
