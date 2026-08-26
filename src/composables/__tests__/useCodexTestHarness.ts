// useCodex 拆分后各模块 spec 共享的测试运行时。
// 注意：vi.mock 前置块（hoisting 限制）仍按文件保留在各 spec 中，不放入本文件。
import { type MockInstance } from "vitest";
import { tabs as _tabs } from "../useEditorTabs";
import { disposeEvents } from "../useCodex/events";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import type { ModelInfo, SessionTab } from "../useCodex/types";

export const capturedListeners: Record<
  string,
  Array<(ev: { payload?: unknown }) => void>
> = {};

let activeListenMock: MockInstance | undefined;

/** 无参调用时复用最近一次传入的 mockedListen（迁移自旧 spec 的块内直调兼容） */
export function mockListenCapture(mockedListen?: MockInstance) {
  if (mockedListen) activeListenMock = mockedListen;
  if (!activeListenMock) {
    throw new Error("mockListenCapture: missing mockedListen");
  }
  activeListenMock.mockImplementation(
    async (event: string, cb: (ev: { payload?: unknown }) => unknown) => {
      (capturedListeners[event] ??= []).push(
        cb as (ev: { payload?: unknown }) => void,
      );
      return () => {};
    },
  );
}

export function fireListen(event: string, payload: unknown) {
  for (const cb of capturedListeners[event] ?? []) {
    cb({ payload });
  }
}

export function makeSessionTab(
  id: string,
  threadId: string | null,
  over: Partial<SessionTab> = {},
): SessionTab {
  return {
    id,
    threadId,
    name: "",
    nameIsFirstMessage: false,
    permissionMode: "ask-for-approval",
    taskMode: "default",
    model: null,
    effort: null,
    plugins: { plugins: [], loaded: false },
    skills: { skills: [], loaded: false },
    creatingChat: false,
    draftJson: JSON.stringify({ type: "doc", content: [] }),
    draftAttachments: [],
    draftRefs: {},
    origin: threadId ? "history" : null,
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
    plan: null,
    loading: false,
    newChatWorkspace: null,
    interactions: [],
    ...over,
    kind: "chat",
    title: "",
    icon: "chat",
  };
}

/** 发送路径测试共用的默认模型 fixture（真实应用启动时模型列表至少含默认模型） */
export const DEFAULT_MODEL: ModelInfo = {
  id: "gpt-5",
  model: "gpt-5",
  displayName: "gpt-5",
  description: "默认模型",
  hidden: false,
  isDefault: true,
  supportedReasoningEfforts: [],
  defaultReasoningEffort: "",
};

export const SKILLS_RESPONSE = {
  data: [
    {
      skills: [
        {
          name: "csharp-code-rules",
          key: "csharp-code-rules",
          path: "C:/x/skills/csharp-code-rules/SKILL.md",
          description: "C# 代码规范长描述",
          interface: {
            shortDescription: "C# 代码规范短描述",
          },
        },
        {
          name: "disabled-skill",
          key: "disabled-skill",
          path: "C:/x/disabled/SKILL.md",
          desc: "已禁用",
          enabled: false,
        },
      ],
    },
  ],
};

export const PLUGINS_RESPONSE = {
  marketplaces: [
    {
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          installed: true,
          enabled: true,
          source: { path: "C:/x/documents" },
          interface: {
            displayName: "Documents",
            shortDescription: "文档处理",
            composerIcon: "C:/x/documents/icon.png",
            composerIconUrl: null,
            brandColor: "#2563EB",
          },
        },
        {
          id: "pdf@openai-primary-runtime",
          name: "pdf",
          installed: true,
          enabled: true,
          source: { path: "C:/x/pdf" },
          interface: { displayName: "PDF" },
        },
        {
          id: "disabled@openai-curated",
          name: "disabled",
          installed: false,
          enabled: false,
          source: { path: "C:/x/disabled" },
          interface: { displayName: "Disabled" },
        },
      ],
    },
    {
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          installed: true,
          enabled: true,
          source: { path: "C:/x/dup" },
          interface: { displayName: "Duplicate" },
        },
      ],
    },
  ],
};

/** 统一列表在本 spec 中只放会话标签 fixture，按 SessionTab 数组使用 */
export const tabs = _tabs as unknown as SessionTab[];

/** 各 spec 共享的 beforeEach 重置（对应原 useCodex.spec.ts 顶部全局 beforeEach） */
export function resetUseCodexState(
  mockedInvoke: MockInstance,
  mockedListen: MockInstance,
) {
  disposeEvents(); // 重置 wired，避免 init() 内的 wireEvents 与其它用例互相干扰
  for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
  mockListenCapture(mockedListen);
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation((cmd: string) => {
    if (cmd === "thread_list") {
      return Promise.resolve({ data: [], nextCursor: null });
    }
    return Promise.resolve(undefined);
  });
  __resetSessionTabsForTest();
  store.booting = true;
}
