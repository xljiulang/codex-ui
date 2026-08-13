/**
 * 聚焦输入框：优先用 ComposerBar 暴露的 Tiptap 实例（可正确放置光标），
 * 兜底聚焦 ProseMirror DOM。新建会话后统一走此入口保持行为一致。
 */
export function focusComposer() {
  const ed = (
    window as unknown as {
      __CODEX_UI_EDITOR__?: { commands?: { focus?: () => void } };
    }
  ).__CODEX_UI_EDITOR__;
  if (ed?.commands?.focus) {
    ed.commands.focus();
    return;
  }
  document.querySelector<HTMLElement>(".composer .ProseMirror")?.focus();
}
