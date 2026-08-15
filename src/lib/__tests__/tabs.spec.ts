import { describe, expect, it } from "vitest";
import { isTabWorking, TabIcon, TabKind } from "../tabs";
import type { SessionTab } from "../../composables/useCodex";
import type { EditorTab } from "../../composables/useEditorTabs";

function sessionTab(over: Partial<SessionTab> = {}): SessionTab {
  return {
    id: "s1",
    kind: TabKind.Chat,
    title: "会话",
    icon: TabIcon.Chat,
    threadId: "t1",
    name: "",
    origin: "history",
    workspace: "D:/repo",
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
    ...over,
  };
}

function plainTab(kind: string): EditorTab {
  return {
    kind,
    id: "t-" + kind,
    title: kind,
    icon: TabIcon.File,
    workspace: "D:/repo",
    loading: false,
  } as unknown as EditorTab;
}

describe("isTabWorking 统一工作中判定", () => {
  it("枚举值与既有字符串字面量一致", () => {
    expect(TabKind.Chat).toBe("chat");
    expect(TabKind.File).toBe("file");
    expect(TabKind.Diff).toBe("diff");
    expect(TabKind.Preview).toBe("preview");
    expect(TabKind.Terminal).toBe("terminal");
    expect(TabIcon.Chat).toBe("chat");
    expect(TabIcon.Terminal).toBe("terminal");
    expect(TabIcon.File).toBe("file");
  });

  it("会话标签：回合进行中或目标激活为工作中", () => {
    expect(isTabWorking(sessionTab({ turnActive: true }))).toBe(true);
    expect(isTabWorking(sessionTab({ goalStatus: "active" }))).toBe(true);
    expect(isTabWorking(sessionTab())).toBe(false);
    expect(
      isTabWorking(sessionTab({ turnActive: true, goalStatus: "complete" })),
    ).toBe(true);
  });

  it("终端标签：busy 且未退出无错误为工作中", () => {
    expect(
      isTabWorking(
        plainTab(TabKind.Terminal) as unknown as EditorTab & {
          busy: boolean;
          exited: boolean;
          error: string;
        },
      ),
    ).toBe(false);
    const term = {
      ...plainTab(TabKind.Terminal),
      busy: true,
      exited: false,
      error: "",
    } as unknown as EditorTab;
    expect(isTabWorking(term)).toBe(true);
    expect(
      isTabWorking({
        ...term,
        exited: true,
      } as unknown as EditorTab),
    ).toBe(false);
    expect(
      isTabWorking({
        ...term,
        error: "spawn boom",
      } as unknown as EditorTab),
    ).toBe(false);
  });

  it("文件/diff/预览标签：恒非工作中", () => {
    expect(isTabWorking(plainTab(TabKind.File))).toBe(false);
    expect(isTabWorking(plainTab(TabKind.Diff))).toBe(false);
    expect(isTabWorking(plainTab(TabKind.Preview))).toBe(false);
  });
});
