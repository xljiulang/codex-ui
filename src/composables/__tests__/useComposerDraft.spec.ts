import { describe, expect, it, vi } from "vitest";
import { ref, type Ref } from "vue";
import type { Editor } from "@tiptap/core";
import type { SessionTab } from "../useCodex/types";
import type { UserInput } from "../../lib/types";
import { useComposerDraft } from "../useComposerDraft";

function tabFixture(): SessionTab {
  return {
    id: "sess-1",
    kind: "chat",
    title: "会话",
    icon: "chat",
    threadId: "t1",
    name: "",
    nameIsFirstMessage: false,
    permissionMode: "ask-for-approval",
    taskMode: "execute",
    model: null,
    effort: null,
    draftJson: "",
    draftAttachments: [],
    draftRefs: {},
    origin: "history",
    workspace: null,
    resumedThreadId: null,
    turnActive: false,
    currentTurnId: null,
    turnInterrupted: false,
    goalText: null,
    goalStatus: null,
    goalArmed: false,
    threadTokenUsage: null,
    followupQueue: [],
    attachments: [],
    planPrompt: null,
    loading: false,
    newChatWorkspace: null,
    interactions: [],
  };
}

function editorMock(): Editor {
  return {
    getJSON: vi.fn(() => ({ type: "doc", content: [] })),
    commands: {
      setContent: vi.fn(),
    },
  } as unknown as Editor;
}

function setup() {
  const tab = tabFixture();
  const editor = ref(editorMock()) as Ref<Editor | null | undefined>;
  const rowAttachments = ref<UserInput[]>([]);
  const refsById = ref(new Map<string, UserInput>());
  const hasText = ref(false);
  const draft = useComposerDraft({
    tab: () => tab,
    editor,
    rowAttachments,
    refsById,
    hasText,
  });
  return { tab, editor, rowAttachments, refsById, hasText, draft };
}

describe("useComposerDraft", () => {
  it("有内容时保存草稿：JSON/附件/引用快照", () => {
    const { tab, rowAttachments, refsById, hasText, draft } = setup();
    rowAttachments.value.push({ type: "localImage", path: "D:/p.png" });
    refsById.value.set("ref-1", {
      type: "skill",
      name: "s",
      path: "C:/x/SKILL.md",
    });
    hasText.value = true;
    draft.saveDraftToTab();
    expect(tab.draftJson).toContain('"doc"');
    expect(tab.draftAttachments).toEqual([
      { type: "localImage", path: "D:/p.png" },
    ]);
    expect(tab.draftRefs).toEqual({
      "ref-1": { type: "skill", name: "s", path: "C:/x/SKILL.md" },
    });
  });

  it("无内容时不写草稿", () => {
    const { tab, draft } = setup();
    draft.saveDraftToTab();
    expect(tab.draftJson).toBe("");
    expect(tab.draftAttachments).toEqual([]);
  });

  it("恢复草稿：还原引用/附件并设置编辑器内容", () => {
    const { tab, rowAttachments, refsById, draft, editor } = setup();
    tab.draftJson = JSON.stringify({ type: "doc", content: [{ type: "p" }] });
    tab.draftAttachments = [{ type: "localImage", path: "D:/p.png" }];
    tab.draftRefs = { "r1": { type: "mention", name: "a", path: "D:/a.cs" } };
    draft.restoreDraftFromTab();
    expect(refsById.value.get("r1")).toEqual({
      type: "mention",
      name: "a",
      path: "D:/a.cs",
    });
    expect(rowAttachments.value).toEqual([
      { type: "localImage", path: "D:/p.png" },
    ]);
    expect(editor.value?.commands.setContent).toHaveBeenCalledWith({
      type: "doc",
      content: [{ type: "p" }],
    });
  });

  it("损坏草稿静默丢弃", () => {
    const { tab, draft, editor } = setup();
    tab.draftJson = "{bad json";
    draft.restoreDraftFromTab();
    expect(editor.value?.commands.setContent).not.toHaveBeenCalled();
  });
});
