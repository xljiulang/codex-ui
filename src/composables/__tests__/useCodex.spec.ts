import { describe, expect, it, beforeEach } from "vitest";
import { vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

import { invoke } from "@tauri-apps/api/core";
import {
  __resetPinnedSectionForTest,
  ensureSkills,
  ensureThreadPlugins,
  interrupt,
  newEmptyChat,
  openThread,
  sendPrompt,
  sortThreads,
  store,
  togglePin,
  toastError,
} from "../useCodex";

const mockedInvoke = vi.mocked(invoke);

const SKILLS_RESPONSE = {
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

const PLUGINS_RESPONSE = {
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

describe("toastError 错误提示提取", () => {
  it("优先提取 error.error.message / error.message，回退 String(e)", () => {
    expect(toastError({ error: { message: "服务端错误" } })).toBe("服务端错误");
    expect(toastError({ message: "普通错误" })).toBe("普通错误");
    expect(toastError("raw string")).toBe("raw string");
    expect(toastError(42)).toBe("42");
    expect(toastError(null)).toBe("null");
  });
});

describe("队列模式下发送提示", () => {
  beforeEach(() => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.settings.followup_mode = "queue";
    store.toast = "";
    store.followupQueue.splice(0);
    store.attachments.splice(0);
  });

  it("回合进行中且为队列模式：消息入队并提示", async () => {
    await sendPrompt("第二条消息");
    expect(store.followupQueue).toHaveLength(1);
    expect(store.followupQueue[0].text).toBe("第二条消息");
    expect(store.toast).toContain("已加入队列");
  });
});

describe("ensureThreadPlugins 对话级插件缓存", () => {
  beforeEach(() => {
    store.threadPlugins = {};
    mockedInvoke.mockReset();
  });

  it("为对话归一化 plugin/list：过滤未安装/禁用插件并按 id 去重", async () => {
    mockedInvoke.mockResolvedValue(PLUGINS_RESPONSE);
    await ensureThreadPlugins("t1");
    expect(store.threadPlugins["t1"]).toEqual({
      loaded: true,
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          displayName: "Documents",
          description: "文档处理",
          path: "C:/x/documents",
          iconPath: "C:/x/documents/icon.png",
          iconUrl: "",
          brandColor: "#2563EB",
        },
        {
          id: "pdf@openai-primary-runtime",
          name: "pdf",
          displayName: "PDF",
          description: "",
          path: "C:/x/pdf",
          iconPath: "",
          iconUrl: "",
          brandColor: "",
        },
      ],
    });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/list",
      params: {},
    });
    const skillsCalls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(skillsCalls).toHaveLength(0);
  });

  it("同对话复用缓存不重复请求，不同对话各自缓存", async () => {
    mockedInvoke.mockResolvedValue(PLUGINS_RESPONSE);
    await ensureThreadPlugins("t1");
    await ensureThreadPlugins("t1"); // 同对话复用
    let calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/list",
    );
    expect(calls).toHaveLength(1);

    await ensureThreadPlugins("t2"); // 新对话预加载
    await ensureThreadPlugins("t1"); // 仍复用
    calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/list",
    );
    expect(calls).toHaveLength(2);
  });

  it("plugin/list 失败时该对话缓存为空且不回退 skills/list", async () => {
    mockedInvoke.mockRejectedValue(new Error("boom"));
    await ensureThreadPlugins("t1");
    expect(store.threadPlugins["t1"]).toEqual({
      plugins: [],
      loaded: false,
    });
    const skillsCalls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(skillsCalls).toHaveLength(0);
  });
});

describe("ensureSkills 技能全局缓存", () => {
  beforeEach(() => {
    store.skills = [];
    store.skillsLoaded = false;
    mockedInvoke.mockReset();
  });

  it("归一化 skills/list：过滤禁用项并缓存", async () => {
    mockedInvoke.mockResolvedValue(SKILLS_RESPONSE);
    await ensureSkills();
    expect(store.skillsLoaded).toBe(true);
    expect(store.skills).toEqual([
      {
        name: "csharp-code-rules",
        key: "csharp-code-rules",
        path: "C:/x/skills/csharp-code-rules/SKILL.md",
        desc: "C# 代码规范长描述",
        shortDesc: "C# 代码规范短描述",
      },
    ]);
    const calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(calls).toHaveLength(1);
  });

  it("幂等：第二次调用不重复请求", async () => {
    mockedInvoke.mockResolvedValue(SKILLS_RESPONSE);
    await ensureSkills();
    await ensureSkills();
    const calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(calls).toHaveLength(1);
  });
});

describe("切换会话自动标准停止旧回合", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.threadPlugins = {};
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentTurnId = null;
    store.currentThreadOrigin = null;
    store.currentThreadCwd = null;
    store.resumedThreadId = null;
    store.threadTokenUsage = null;
    store.goalText = null;
    store.taskMode = "execute";
  });

  it("新建对话时标准停止旧回合（turn_interrupt），再复位到新对话", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    store.showHistory = true;
    await newEmptyChat();
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.currentThreadId).toBeNull();
    expect(store.currentTurnId).toBeNull();
    // 历史面板只由头部按钮控制，新建对话不自动关闭
    expect(store.showHistory).toBe(true);
  });

  it("目标模式下新建对话：先清旧会话目标，再中断旧回合", async () => {
    store.turnActive = true;
    store.taskMode = "goal";
    store.goalText = "旧目标";
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    await newEmptyChat();
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.currentThreadId).toBeNull();
  });

  it("切换到其他历史会话时标准停止旧回合，并打开目标会话", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    store.showHistory = true;
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t2", name: "会话2", turns: [] },
        });
      }
      if (cmd === "codex_rpc") {
        const method = (args as { params?: { method?: string } })?.params
          ?.method;
        if (method === "thread/turns/list") {
          return Promise.resolve({ data: [], nextCursor: null });
        }
      }
      if (cmd === "goal_get") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
    await openThread("t2");
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.currentThreadId).toBe("t2");
    // 点击历史会话不关闭历史面板（手动触发）
    expect(store.showHistory).toBe(true);
  });

  it("点击当前正在进行的会话不算切换，不中断", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t1", name: "会话1", turns: [] },
        });
      }
      return Promise.resolve(undefined);
    });
    await openThread("t1");
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
  });

  it("目标模式标准停止：先清目标再 turn/interrupt（默认操作当前会话）", async () => {
    store.turnActive = true;
    store.taskMode = "goal";
    store.goalText = "目标";
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    await interrupt();
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.goalText).toBeNull();
    expect(store.taskMode).toBe("execute");
  });

  it("显式传入旧线程/回合 id 时：目标清除作用于旧线程，且不污染新会话的回合 id", async () => {
    store.taskMode = "goal";
    store.goalText = "旧目标";
    store.currentThreadId = "t2"; // 模拟 store 已切到新会话
    store.currentTurnId = null;
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "turn_interrupt") {
        const turnId = (args as { turnId?: string })?.turnId;
        if (turnId === "turn-1") {
          return Promise.reject(
            new Error("turn not found but found abc-123"),
          );
        }
      }
      return Promise.resolve(undefined);
    });
    await interrupt("t1", "turn-1");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    // 重试用服务端返回的活跃回合 id，但不写进 store（那是新会话的状态）
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "abc-123",
    });
    expect(store.currentTurnId).toBeNull();
  });
});

describe("置顶 togglePin（新版 Pinned 分区协议）", () => {
  const FALLBACK_ID = "01984de2-8f74-7c91-a3b2-5c5e937cf318";

  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetPinnedSectionForTest();
    store.threads = [];
    store.loadingHistory = false;
  });

  function metadataCalls() {
    return mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string })?.method === "thread/metadata/update",
    );
  }

  it("置顶：从 threadSection/list 发现 Pinned 分区并用其 id 调用 metadata/update", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "threadSection/list") {
        return Promise.resolve({ data: [{ id: "sec-1", name: "Pinned" }] });
      }
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "thread/metadata/update") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0, isPinned: true }],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(metadataCalls()).toHaveLength(1);
    expect(metadataCalls()[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", sectionId: "sec-1" },
    });
    expect(store.threads[0].isPinned).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("thread_list", {
      limit: 50,
      cursor: null,
    });
  });

  it("取消置顶：sectionId 传 null", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "thread/metadata/update") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0, isPinned: false }],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", false);

    expect(metadataCalls()).toHaveLength(1);
    expect(metadataCalls()[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", sectionId: null },
    });
    expect(store.threads[0].isPinned).toBe(false);
  });

  it("threadSection/list 失败时回退固定内置分区 id", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "threadSection/list") {
        return Promise.reject(new Error("unknown method"));
      }
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "thread/metadata/update") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(metadataCalls()[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", sectionId: FALLBACK_ID },
    });
  });

  it("metadata/update 失败时回滚本地置顶状态并提示", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "threadSection/list") {
        return Promise.resolve({ data: [{ id: "sec-1", name: "Pinned" }] });
      }
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "thread/metadata/update") {
        return Promise.reject(new Error("update failed"));
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(store.threads[0].isPinned).toBeUndefined();
    expect(metadataCalls()).toHaveLength(1);
  });

  it("sortThreads 从 section 推导置顶：固定优先，再按最近时间降序", () => {
    const list = [
      { id: "b", name: "未置顶新", createdAt: 0, recencyAt: 2 },
      { id: "a", name: "置顶旧", createdAt: 0, recencyAt: 1, section: { id: "sec-1", name: "Pinned" } },
      { id: "c", name: "未置顶更新", createdAt: 0, recencyAt: 3 },
    ];
    const sorted = sortThreads(list);
    expect(sorted.map((t) => t.id)).toEqual(["a", "c", "b"]);
    expect(sorted[0].isPinned).toBe(true);
  });
});
