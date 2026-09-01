import { ref } from "vue";

/** 空态整卡目标下限与编辑器↔整卡的高差（工具条 + 卡片边框），与 CSS 保持一致 */
const CARD_MIN_H = 128;
const COMPOSER_CHROME_H = 50;
const MIN_COMPOSER_RATIO = 0.16;

/** 输入区垂直拖拽调整高度（WebView2 pointer 事件 + body 状态类防文本选择） */
export function useComposerResize() {
  const composerHeight = ref<number | null>(null);
  const resizingComposer = ref(false);
  let resizeStartY = 0;
  let resizeStartH = minComposerHeight();

  /** 输入框最小高度（编辑器盒高）：使空态整卡 = max(CARD_MIN_H, 16vh) */
  function minComposerHeight(): number {
    return (
      Math.max(CARD_MIN_H, Math.round(window.innerHeight * MIN_COMPOSER_RATIO)) -
      COMPOSER_CHROME_H
    );
  }

  function maxComposerHeight(): number {
    return Math.max(minComposerHeight(), Math.round(window.innerHeight / 2));
  }

  function currentComposerHeight(): number {
    const el = document.querySelector<HTMLElement>(".rich-editor .ProseMirror");
    return el
      ? Math.round(el.getBoundingClientRect().height)
      : minComposerHeight();
  }

  function startComposerResize(e: PointerEvent) {
    e.preventDefault();
    resizingComposer.value = true;
    resizeStartY = e.clientY;
    resizeStartH = composerHeight.value ?? currentComposerHeight();
    window.addEventListener("pointermove", onComposerResizeMove);
    window.addEventListener("pointerup", endComposerResize);
    document.body.classList.add("resizing-composer");
  }

  function onComposerResizeMove(e: PointerEvent) {
    const h = resizeStartH + (resizeStartY - e.clientY);
    composerHeight.value = Math.min(
      maxComposerHeight(),
      Math.max(minComposerHeight(), Math.round(h)),
    );
  }

  function endComposerResize() {
    resizingComposer.value = false;
    window.removeEventListener("pointermove", onComposerResizeMove);
    window.removeEventListener("pointerup", endComposerResize);
    document.body.classList.remove("resizing-composer");
  }

  /** 窗口尺寸变化后收缩超出上限、抬升低于下限的高度 */
  function clampComposerHeightOnResize() {
    if (composerHeight.value != null) {
      composerHeight.value = Math.min(
        Math.max(composerHeight.value, minComposerHeight()),
        maxComposerHeight(),
      );
    }
  }

  return {
    composerHeight,
    resizingComposer,
    startComposerResize,
    endComposerResize,
    clampComposerHeightOnResize,
  };
}
