import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";

vi.mock("../../composables/useCodex", () => ({
  currentItems: vi.fn(),
  store: reactive({
    turnActive: false,
    busy: false,
    loadingThread: false,
    turnInterrupted: false,
    currentThreadId: null,
    activeWorkByThread: {},
    itemsRev: 0,
  }),
}));

import ChatView from "../ChatView.vue";
import { currentItems, store } from "../../composables/useCodex";
import type { ThreadItem } from "../../lib/types";

const mockedItems = vi.mocked(currentItems);

describe("ChatView 日期分隔线", () => {
  beforeEach(() => {
    // happy-dom 用同步 rAF 简化吸底滚动测试（返回 undefined，避免 scrollRaf 守卫卡死）
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return undefined;
    });
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
  });

  it("跨天消息之间插入日期分隔线", () => {
    const day1 = new Date(2026, 7, 9, 10, 0).getTime();
    const day2 = new Date(2026, 7, 10, 9, 0).getTime();
    mockedItems.mockReturnValue([
      { id: "a1", type: "agentMessage", text: "第一天", startedAtMs: day1 },
      { id: "a2", type: "agentMessage", text: "第二天", startedAtMs: day2 },
    ] as ThreadItem[]);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: { template: "<div class='msg-stub' />" },
        },
      },
    });
    const seps = wrapper.findAll(".date-sep");
    expect(seps).toHaveLength(2);
    expect(seps[0].text()).toContain("8月9日");
    expect(seps[1].text()).toContain("8月10日");
  });

  it("同一天不重复插入分隔线，缺少时间戳时不插入", () => {
    const day1 = new Date(2026, 7, 9, 10, 0).getTime();
    mockedItems.mockReturnValue([
      { id: "a1", type: "agentMessage", text: "x", startedAtMs: day1 },
      { id: "a2", type: "agentMessage", text: "y", startedAtMs: day1 },
      { id: "a3", type: "agentMessage", text: "无时间戳" },
    ] as ThreadItem[]);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: { template: "<div class='msg-stub' />" },
        },
      },
    });
    expect(wrapper.findAll(".date-sep")).toHaveLength(1);
    expect(wrapper.findAll(".msg-stub")).toHaveLength(3);
  });

  it("无消息时显示空状态", () => {
    mockedItems.mockReturnValue([]);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
          EmptyState: { template: "<div class='empty-stub' />" },
        },
      },
    });
    expect(wrapper.find(".empty-stub").exists()).toBe(true);
    store.turnActive = false;
  });

  it("思考中提示随进行中计数显示/隐藏", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.activeWorkByThread = { t1: 0 };
    mockedItems.mockReturnValue([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
        },
      },
    });
    expect(wrapper.find(".thinking-chip").exists()).toBe(true);
    store.activeWorkByThread.t1 = 1;
    await nextTick();
    expect(wrapper.find(".thinking-chip").exists()).toBe(false);
    store.turnActive = false;
    store.currentThreadId = null;
    store.activeWorkByThread = {};
  });

  it("无障碍播报区域随回合状态更新", async () => {
    mockedItems.mockReturnValue([]);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
          EmptyState: { template: "<div />" },
        },
      },
    });
    const live = wrapper.find('.sr-only[aria-live="polite"]');
    expect(live.exists()).toBe(true);
    store.turnActive = true;
    await nextTick();
    expect(live.text()).toBe("正在生成回复");
    store.turnActive = false;
    await nextTick();
    expect(live.text()).toBe("回复完成");
  });

  it("新增消息时仍自动吸底滚动", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    mockedItems.mockReturnValue(arr);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: { template: "<div class='msg-stub' />" },
        },
      },
    });
    const scroller = wrapper.find(".chat-scroll").element as HTMLElement;
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    scroller.scrollTop = 0;
    arr.push({ id: "a2", type: "agentMessage", text: "y" } as ThreadItem);
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
  });

  it("任意消息变更（itemsRev 增长）时吸底滚动", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
      { id: "a2", type: "agentMessage", text: "y" } as ThreadItem,
    ]);
    mockedItems.mockReturnValue(arr);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
        },
      },
    });
    const scroller = wrapper.find(".chat-scroll").element as HTMLElement;
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    scroller.scrollTop = 0;
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
  });

  it("上滑暂停后 itemsRev 增长不再滚动", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
      { id: "a2", type: "agentMessage", text: "y" } as ThreadItem,
    ]);
    mockedItems.mockReturnValue(arr);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
        },
      },
    });
    const scroller = wrapper.find(".chat-scroll").element as HTMLElement;
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    // 模拟用户上滑：滚动距离超过阈值后 stickToBottom 变 false
    const scrollEv = new Event("scroll");
    Object.defineProperty(scrollEv, "isTrusted", { get: () => true });
    scroller.dispatchEvent(scrollEv);
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(0);
  });

  it("程序化未信任 scroll 事件不解除吸底", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    mockedItems.mockReturnValue(arr);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
        },
      },
    });
    const scroller = wrapper.find(".chat-scroll").element as HTMLElement;
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    scroller.scrollTop = 0;
    // 模拟程序化吸底写入后派发的 scroll 事件（isTrusted=false，dist>60）
    scroller.dispatchEvent(new Event("scroll"));
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
  });

  it("末尾消息流式追加时仍吸底滚动", async () => {
    const arr = reactive([
      {
        id: "a1",
        type: "agentMessage",
        text: "x",
        streaming: true,
      } as ThreadItem,
    ]);
    mockedItems.mockReturnValue(arr);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
        },
      },
    });
    const scroller = wrapper.find(".chat-scroll").element as HTMLElement;
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    scroller.scrollTop = 0;
    arr[0].text = "x appended";
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
  });

  it("末尾思考过程内容增长时仍吸底滚动", async () => {
    const arr = reactive([
      {
        id: "r1",
        type: "reasoning",
        content: ["第一段"],
        streaming: true,
      } as ThreadItem,
    ]);
    mockedItems.mockReturnValue(arr);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
        },
      },
    });
    const scroller = wrapper.find(".chat-scroll").element as HTMLElement;
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    scroller.scrollTop = 0;
    arr[0].content = ["第一段", "第二段"];
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
  });

  it("DOM 结构变化（如 worker 渲染落地）时吸底滚动", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    mockedItems.mockReturnValue(arr);
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
        },
      },
    });
    const scroller = wrapper.find(".chat-scroll").element as HTMLElement;
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    scroller.scrollTop = 0;
    // 模拟 markdown/输出 HTML 直接插入导致的 DOM 高度增长
    scroller.appendChild(document.createElement("div"));
    await vi.waitFor(() => expect(scroller.scrollTop).toBe(1000), {
      timeout: 2000,
      interval: 10,
    });
  });
});
