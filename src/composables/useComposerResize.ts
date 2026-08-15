import { ref } from "vue";

/** 输入框可拖拽高度：最低为现有自动高度，最高为窗口一半 */
const MIN_COMPOSER_HEIGHT = 120;

/** 输入区垂直拖拽调整高度（WebView2 pointer 事件 + body 状态类防文本选择） */
export function useComposerResize() {
  const composerHeight = ref<number | null>(null);
  const resizingComposer = ref(false);
  let resizeStartY = 0;
  let resizeStartH = MIN_COMPOSER_HEIGHT;

  function maxComposerHeight(): number {
    return Math.max(MIN_COMPOSER_HEIGHT, Math.round(window.innerHeight / 2));
  }

  function currentComposerHeight(): number {
    const el = document.querySelector<HTMLElement>(".rich-editor .ProseMirror");
    return el
      ? Math.round(el.getBoundingClientRect().height)
      : MIN_COMPOSER_HEIGHT;
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
      Math.max(MIN_COMPOSER_HEIGHT, Math.round(h)),
    );
  }

  function endComposerResize() {
    resizingComposer.value = false;
    window.removeEventListener("pointermove", onComposerResizeMove);
    window.removeEventListener("pointerup", endComposerResize);
    document.body.classList.remove("resizing-composer");
  }

  /** 窗口尺寸变化后收缩超出上限的高度 */
  function clampComposerHeightOnResize() {
    if (composerHeight.value != null) {
      composerHeight.value = Math.min(composerHeight.value, maxComposerHeight());
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
