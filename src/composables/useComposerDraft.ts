import type { Ref } from "vue";
import type { Editor, JSONContent } from "@tiptap/core";
import type { SessionTab } from "./useCodex/types";
import type { UserInput } from "../lib/types";

/** 输入草稿：按会话标签持久化（内容/附件/内联引用），激活恢复、切走快照 */
export function useComposerDraft(options: {
  tab: () => SessionTab | undefined;
  editor: Ref<Editor | null | undefined>;
  rowAttachments: Ref<UserInput[]>;
  refsById: Ref<Map<string, UserInput>>;
  hasText: Ref<boolean>;
}) {
  /** 草稿保存：把当前输入内容快照到会话标签；内容为空时清空草稿，避免删空后旧草稿复活 */
  function saveDraftToTab() {
    const tab = options.tab();
    if (!tab) return;
    const ed = options.editor.value;
    const json = ed ? (ed.getJSON() as JSONContent) : null;
    const hasDraft =
      options.hasText.value ||
      options.rowAttachments.value.length > 0 ||
      options.refsById.value.size > 0;
    if (!hasDraft) {
      tab.draftJson = "";
      tab.draftAttachments = [];
      tab.draftRefs = {};
      return;
    }
    tab.draftJson = json ? JSON.stringify(json) : "";
    tab.draftAttachments = [...options.rowAttachments.value];
    tab.draftRefs = Object.fromEntries(options.refsById.value.entries());
  }

  /** 草稿恢复：标签激活时还原输入内容与内联引用 */
  function restoreDraftFromTab() {
    const ed = options.editor.value;
    const tab = options.tab();
    if (!ed || !tab?.draftJson) return;
    options.refsById.value = new Map(Object.entries(tab.draftRefs ?? {}));
    options.rowAttachments.value = [...(tab.draftAttachments ?? [])];
    try {
      const json = JSON.parse(tab.draftJson) as JSONContent;
      ed.commands.setContent(json);
    } catch {
      // 草稿损坏时静默丢弃，保留空输入
    }
  }

  return { saveDraftToTab, restoreDraftFromTab };
}
