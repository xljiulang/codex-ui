import { __resetPinnedSectionForTest, __resetTitleHelperCapabilityForTest, sortThreads } from "../useCodex/capabilities";
import { disposeEvents, wireEvents } from "../useCodex/events";
import { __resetSessionTabsForTest, activeSessionTab } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import { autoTitleThread, sanitizeTitle, togglePin } from "../useCodex/threads";
import { activeTabId } from "../useEditorTabs";
import { capturedListeners, fireListen, makeSessionTab, mockListenCapture, resetUseCodexState, tabs } from "./useCodexTestHarness";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

const { mockWin } = vi.hoisted(() => ({
  mockWin: {
    label: "main",
    isMinimized: vi.fn().mockResolvedValue(false),
    unminimize: vi.fn(),
    setFocus: vi.fn(),
    setTitle: vi.fn(),
    setProgressBar: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => mockWin),
  ProgressBarStatus: { Indeterminate: "Indeterminate", None: "None" },
}));

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);

beforeEach(() => {
  resetUseCodexState(mockedInvoke, mockedListen);
});

describe("置顶 togglePin（新版 Pinned 分区协议）", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetPinnedSectionForTest();
    store.threads = [];
    store.loadingHistory = false;
    store.toast = "";
  });

  function rpcCalls(method: string) {
    return mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string })?.method === method,
    );
  }

  function capabilityCalls() {
    return mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "codex_pin_capability",
    );
  }

  function mockCapability(
    protocol: string,
    pinnedSectionId: string | null,
    refreshedPinned = true,
  ) {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({ protocol, pinnedSectionId });
      }
      if (cmd === "codex_rpc") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [
            {
              id: "t1",
              name: "会话",
              createdAt: 0,
              recencyAt: 0,
              isPinned: refreshedPinned,
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
  }

  it("metadata_section 置顶：探测到分区协议后用 Pinned 分区 id 调用 metadata/update", async () => {
    mockCapability("metadata_section", "sec-1", true);
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(1);
    expect(rpcCalls("thread/metadata/update")[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", sectionId: "sec-1" },
    });
    expect(store.threads[0].isPinned).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("thread_list", {
      limit: 50,
      cursor: null,
    });
  });

  it("metadata_section 取消置顶：sectionId 传 null", async () => {
    mockCapability("metadata_section", "sec-1", false);
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", false);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(1);
    expect(rpcCalls("thread/metadata/update")[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", sectionId: null },
    });
    expect(store.threads[0].isPinned).toBe(false);
  });

  it("section_move 置顶/取消：走新版 threadSection/move 协议", async () => {
    let refreshedPinned = true;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({ protocol: "section_move", pinnedSectionId: "sec-1" });
      }
      if (cmd === "codex_rpc") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [
            {
              id: "t1",
              name: "会话",
              createdAt: 0,
              recencyAt: 0,
              isPinned: refreshedPinned,
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("threadSection/move")).toHaveLength(1);
    expect(rpcCalls("threadSection/move")[0][1]).toEqual({
      method: "threadSection/move",
      params: { threadId: "t1", sectionId: "sec-1" },
    });
    expect(store.threads[0].isPinned).toBe(true);

    refreshedPinned = false;
    await togglePin("t1", false);

    expect(rpcCalls("threadSection/move")).toHaveLength(2);
    expect(rpcCalls("threadSection/move")[1][1]).toEqual({
      method: "threadSection/move",
      params: { threadId: "t1", sectionId: null },
    });
    expect(store.threads[0].isPinned).toBe(false);
  });

  it("section_move 新方法名：探测返回 thread/section/move 时按其调用", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({
          protocol: "section_move",
          pinnedSectionId: "sec-1",
          sectionMoveMethod: "thread/section/move",
        });
      }
      if (cmd === "codex_rpc") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [
            {
              id: "t1",
              name: "会话",
              createdAt: 0,
              recencyAt: 0,
              isPinned: true,
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/section/move")).toHaveLength(1);
    expect(rpcCalls("thread/section/move")[0][1]).toEqual({
      method: "thread/section/move",
      params: { threadId: "t1", sectionId: "sec-1" },
    });
    expect(rpcCalls("threadSection/move")).toHaveLength(0);
    expect(store.threads[0].isPinned).toBe(true);
  });

  it("metadata_is_pinned 置顶/取消：旧版走 metadata/update isPinned 布尔", async () => {
    let refreshedPinned = true;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({ protocol: "metadata_is_pinned", pinnedSectionId: null });
      }
      if (cmd === "codex_rpc") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [
            {
              id: "t1",
              name: "会话",
              createdAt: 0,
              recencyAt: 0,
              isPinned: refreshedPinned,
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(1);
    expect(rpcCalls("thread/metadata/update")[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", isPinned: true },
    });
    expect(store.threads[0].isPinned).toBe(true);

    refreshedPinned = false;
    await togglePin("t1", false);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(2);
    expect(rpcCalls("thread/metadata/update")[1][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", isPinned: false },
    });
    expect(store.threads[0].isPinned).toBe(false);
  });

  it("unsupported：只弹提示不发请求，本地状态不变", async () => {
    mockCapability("unsupported", null);
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(0);
    expect(rpcCalls("threadSection/move")).toHaveLength(0);
    expect(store.threads[0].isPinned).toBeUndefined();
    expect(store.toast).toContain("不支持置顶");
  });

  it("探测失败：提示且不发起置顶请求，便于下次重试", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.reject(new Error("server not ready"));
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(0);
    expect(rpcCalls("threadSection/move")).toHaveLength(0);
    expect(store.threads[0].isPinned).toBeUndefined();
    expect(store.toast).toContain("不支持置顶");
  });

  it("探测结果缓存：连续两次置顶只探测一次", async () => {
    mockCapability("metadata_is_pinned", null);
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);
    await togglePin("t1", false);

    expect(capabilityCalls()).toHaveLength(1);
  });

  it("写失败时回滚本地置顶状态并提示", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({ protocol: "metadata_section", pinnedSectionId: "sec-1" });
      }
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "thread/metadata/update") {
        return Promise.reject(new Error("update failed"));
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(store.threads[0].isPinned).toBeUndefined();
    expect(rpcCalls("thread/metadata/update")).toHaveLength(1);
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
describe("sanitizeTitle 标题清洗", () => {
  it("去掉引号与 Markdown 标记、折叠空白", () => {
    expect(sanitizeTitle('  "修复 **登录** 页报错" ')).toBe("修复 登录 页报错");
    expect(sanitizeTitle("`重构` 模块\n\n换行\t空白")).toBe("重构 模块 换行 空白");
    expect(sanitizeTitle("标题。")).toBe("标题。");
  });

  it("空输入返回空串，超长截断 50 字", () => {
    expect(sanitizeTitle("   \n ")).toBe("");
    expect(sanitizeTitle("很".repeat(60))).toHaveLength(50);
  });
});
describe("autoTitleThread 临时线程标题总结", () => {
  const LONG_TEXT = "这是一个非常长的用户消息，用来验证标题总结功能能否正常触发和写回。".repeat(2);

  beforeEach(() => {
    disposeEvents(); // 重置 wired，避免前面 wireEvents 用例的监听残留
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    __resetTitleHelperCapabilityForTest();
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.toast = "";
    store.currentThreadId = "t1";
    store.currentThreadName = "";
    store.currentThreadWorkspace = "D:/repo";
    store.server.startupWorkspace = "D:/repo";
    store.threads = [{ id: "t1", name: null, preview: "旧预览", createdAt: 0, recencyAt: 0 }];
  });

  it("不支持 experimentalApi：完全不发起临时线程", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: false, ephemeral: false });
      }
      return Promise.resolve(undefined);
    });
    await autoTitleThread("t1", LONG_TEXT);
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());
  });

  it("短文保持默认标题，不消耗模型", async () => {
    await autoTitleThread("t1", "短消息");
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());
  });

  it("阈值边界：纯文本 15 字不触发，16 字触发", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      return Promise.resolve(undefined);
    });

    await autoTitleThread("t1", "一二三四五六七八九十一二三四五"); // 15 字
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());

    mockedInvoke.mockClear();
    const p = autoTitleThread("t1", "一二三四五六七八九十一二三四五六"); // 16 字
    await p;
    expect(mockedInvoke).toHaveBeenCalledWith("thread_start", expect.anything());

    // 结算临时回合并清理，避免 30s 兜底定时器悬空
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "interrupted" },
    });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
        method: "thread/unsubscribe",
        params: { threadId: "helper1" },
      });
    }, { timeout: 3000, interval: 20 });
  });

  it("线程已有名称时不覆盖", async () => {
    const tab = tabs[0];
    tab.name = "手动标题";
    tab.nameIsFirstMessage = false;
    store.currentThreadName = "手动标题";
    await autoTitleThread("t1", LONG_TEXT);
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());
  });

  it("名称来自首条消息时总结可覆盖并复位标记", async () => {
    const tab = tabs[0];
    tab.name = "帮我修复登录页面报错";
    tab.nameIsFirstMessage = true;
    store.currentThreadName = "帮我修复登录页面报错";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "thread_list")
        return Promise.resolve({ data: [], nextCursor: null });
      return Promise.resolve(undefined);
    });

    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "修复登录页面报错",
    });
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "completed" },
    });

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
        threadId: "t1",
        name: "修复登录页面报错",
      });
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(tab.nameIsFirstMessage).toBe(false);
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_list", expect.anything());
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
        method: "thread/unsubscribe",
        params: { threadId: "helper1" },
      });
    }, { timeout: 3000, interval: 20 });
  });

  it("ephemeral 路径：模型标题写回，临时线程注销", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") {
        const params = (args as { params?: Record<string, unknown> }).params ?? {};
        expect(params.ephemeral).toBe(true);
        expect(params.model).toBeUndefined();
        expect(params.sandbox).toBe("read-only");
        expect(params.cwd).toBe("D:/repo");
        return Promise.resolve({ thread: { id: "helper1" } });
      }
      if (cmd === "turn_start") {
        const params = (args as { params?: Record<string, unknown> }).params ?? {};
        expect(
          (params.sandboxPolicy as { type?: string } | undefined)?.type,
        ).toBe("readOnly");
        return Promise.resolve({ turn: { id: "ht1" } });
      }
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "thread_list")
        return Promise.resolve({ data: [], nextCursor: null });
      return Promise.resolve(undefined);
    });

    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "修复登录",
    });
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "页面报错问题",
    });
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "completed" },
    });

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
        threadId: "t1",
        name: "修复登录页面报错问题",
      });
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(store.toast).toContain("当前会话的标题已简化");
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
        method: "thread/unsubscribe",
        params: { threadId: "helper1" },
      });
    }, { timeout: 3000, interval: 20 });
  });

  it("不支持 ephemeral：普通线程总结后删除", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: false });
      }
      if (cmd === "thread_start") {
        const params = (args as { params?: Record<string, unknown> }).params ?? {};
        expect(params.ephemeral).toBeUndefined();
        expect(params.model).toBeUndefined();
        expect(params.sandbox).toBe("read-only");
        return Promise.resolve({ thread: { id: "helper1" } });
      }
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "thread_list")
        return Promise.resolve({ data: [], nextCursor: null });
      return Promise.resolve(undefined);
    });

    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "重构模块",
    });
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "completed" },
    });

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
        threadId: "t1",
        name: "重构模块",
      });
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(store.toast).toContain("当前会话的标题已简化");
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_delete", {
        threadId: "helper1",
      });
    }, { timeout: 3000, interval: 20 });
  });

  it("回合失败：不写回标题，仍清理临时线程", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      return Promise.resolve(undefined);
    });

    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "failed" },
    });

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
        method: "thread/unsubscribe",
        params: { threadId: "helper1" },
      });
    }, { timeout: 3000, interval: 20 });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "thread_set_name",
      expect.anything(),
    );
    expect(store.toast).not.toContain("当前会话的标题已简化");
  });

  it("后台临时线程事件被隔离：不影响全局进行中状态", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });

    await wireEvents();
    const p = autoTitleThread("t1", LONG_TEXT);
    await p;

    // 后台线程 turn/started：不置为进行中
    fireListen("turn/started", { threadId: "helper1", turn: { id: "ht1" } });
    expect(activeSessionTab()?.turnActive).toBe(false);

    // 后台线程 turn/completed：不结束全局状态、不触发历史刷新
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "后台标题",
    });
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "completed" },
    });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
        threadId: "t1",
        name: expect.any(String),
      });
    }, { timeout: 3000, interval: 20 });
    expect(activeSessionTab()?.turnActive).toBe(false);

    // 主线程事件照常工作
    fireListen("turn/started", { threadId: "t1", turn: { id: "mt1" } });
    expect(activeSessionTab()?.turnActive).toBe(true);
    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "mt1", status: "completed" },
    });
    await vi.waitFor(() => expect(activeSessionTab()?.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
  });
});
