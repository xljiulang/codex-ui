import { afterEach, describe, expect, it, vi } from "vitest";
import { focusComposer } from "../composerFocus";

function attachEditorFallback(): HTMLElement {
  const composer = document.createElement("div");
  composer.className = "composer";
  const pm = document.createElement("div");
  pm.className = "ProseMirror";
  composer.appendChild(pm);
  document.body.appendChild(composer);
  return pm;
}

describe("focusComposer 输入框聚焦", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    document.body.innerHTML = "";
  });

  it("优先调用 Tiptap 实例的 commands.focus", () => {
    const focus = vi.fn();
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {
      commands: { focus },
    };
    focusComposer();
    expect(focus).toHaveBeenCalledTimes(1);
  });

  it("实例存在但无 commands.focus 时回退聚焦 ProseMirror DOM", () => {
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {};
    const pm = attachEditorFallback();
    const spy = vi.spyOn(pm, "focus");
    focusComposer();
    expect(spy).toHaveBeenCalled();
  });

  it("无编辑器实例时回退聚焦 .composer .ProseMirror", () => {
    const pm = attachEditorFallback();
    const spy = vi.spyOn(pm, "focus");
    focusComposer();
    expect(spy).toHaveBeenCalled();
  });
});
