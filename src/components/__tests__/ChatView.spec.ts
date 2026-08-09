import { describe, expect, it, vi } from "vitest";
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
  }),
}));

import ChatView from "../ChatView.vue";
import { currentItems, store } from "../../composables/useCodex";
import type { ThreadItem } from "../../lib/types";

const mockedItems = vi.mocked(currentItems);

describe("ChatView 日期分隔线", () => {
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

  it("虚拟化结构：总高 spacer + 绝对定位行 + 稳定 key", () => {
    const day = new Date(2026, 7, 9, 10, 0).getTime();
    mockedItems.mockReturnValue(
      Array.from(
        { length: 8 },
        (_, i) =>
          ({
            id: "m" + i,
            type: "agentMessage",
            text: "x" + i,
            startedAtMs: day,
          }) as ThreadItem,
      ),
    );
    const wrapper = mount(ChatView, {
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: { template: "<div class='msg-stub' />" },
        },
      },
    });
    const wrap = wrapper.find(".virtual-wrap");
    expect(wrap.exists()).toBe(true);
    expect(wrap.attributes("style")).toContain("height:");
    const vrows = wrapper.findAll(".vrow");
    // happy-dom 视口尺寸为 0 → 退化为全量渲染
    expect(vrows).toHaveLength(9); // 1 条日期分隔 + 8 条消息
    expect(vrows[0].attributes("data-key")).toContain("sep-");
    expect(vrows[1].attributes("data-key")).toBe("m0");
    expect(vrows[1].attributes("style")).toContain("top: 34px");
    expect(vrows[2].attributes("style")).toContain("214px");
    expect(wrapper.findAll(".msg-stub")).toHaveLength(8);
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
});
