import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { computed, nextTick, reactive } from "vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("../../composables/useCodex", () => {
  const store = reactive({
    turnActive: false,
    busy: false,
    loading: false,
    currentThreadId: null,
    itemsByThread: {} as Record<string, unknown[]>,
    activeWorkByThread: {},
    itemsRev: 0,
    userSendRev: 0,
    interactions: [] as {
      requestId: number;
      method: string;
      params: Record<string, unknown>;
      at: number;
    }[],
    planPrompt: null as {
      threadId: string;
      turnId: string;
      planText: string;
    } | null,
  });
  return {
    currentItems: vi.fn(),
    resolveSessionWorkspace: () => "",
    workspace: computed(() => ""),
    store,
    dismissPlanPrompt: vi.fn(),
    executePlan: vi.fn(),
    exitPlanMode: vi.fn(),
    respondInteraction: vi.fn(
      (interaction: { requestId: number }) => {
        const i = store.interactions.findIndex(
          (x) => x.requestId === interaction.requestId,
        );
        if (i >= 0) store.interactions.splice(i, 1);
      },
    ),
  };
});

import ChatView from "../ChatView.vue";
import { respondInteraction, store } from "../../composables/useCodex";
import type { ThreadItem } from "../../lib/types";
import type { SessionTab } from "../../composables/useCodex";

/** 会话标签 fixture：threadId 固定 t1，交互列表与 mock store 共享同一数组 */
function makeTab(): SessionTab {
  return {
    id: "tab-1",
    kind: "chat",
    title: "会话",
    icon: "chat",
    threadId: "t1",
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
    plan: null,
    loading: false,
    newChatWorkspace: null,
    interactions: store.interactions as unknown as SessionTab["interactions"],
  };
}

let tab: SessionTab;

describe("ChatView 日期分隔线", () => {
  beforeEach(() => {
    store.interactions.splice(0);
    tab = reactive(makeTab());
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
    store.itemsByThread["t1"] = ([
      { id: "a1", type: "agentMessage", text: "第一天", startedAtMs: day1 },
      { id: "a2", type: "agentMessage", text: "第二天", startedAtMs: day2 },
    ] as ThreadItem[]);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsByThread["t1"] = ([
      { id: "a1", type: "agentMessage", text: "x", startedAtMs: day1 },
      { id: "a2", type: "agentMessage", text: "y", startedAtMs: day1 },
      { id: "a3", type: "agentMessage", text: "无时间戳" },
    ] as ThreadItem[]);
    const wrapper = mount(ChatView, { props: { tab },
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

  it("按回合分组渲染：userMessage 起始新回合，历史续接归入伪回合", () => {
    store.itemsByThread["t1"] = ([
      { id: "a0", type: "agentMessage", text: "历史" },
      { id: "u1", type: "userMessage", text: "问题1" },
      { id: "r1", type: "reasoning", content: ["思考"] },
      { id: "a1", type: "agentMessage", text: "回答1" },
      { id: "u2", type: "userMessage", text: "问题2" },
      { id: "a2", type: "agentMessage", text: "回答2" },
    ] as ThreadItem[]);
    const wrapper = mount(ChatView, { props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: {
            props: ["item"],
            template: "<div class='msg-stub'>{{ item.type }}</div>",
          },
        },
      },
    });
    const turns = wrapper.findAll(".turn");
    expect(turns).toHaveLength(3);
    expect(turns[0].findAll(".msg-stub").map((x) => x.text())).toEqual([
      "agentMessage",
    ]);
    expect(turns[1].findAll(".msg-stub").map((x) => x.text())).toEqual([
      "userMessage",
      "reasoning",
      "agentMessage",
    ]);
    expect(turns[2].findAll(".msg-stub").map((x) => x.text())).toEqual([
      "userMessage",
      "agentMessage",
    ]);
  });

  it("无消息时显示空状态", () => {
    store.itemsByThread["t1"] = ([]);
    const wrapper = mount(ChatView, { props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
          EmptyState: { template: "<div class='empty-stub' />" },
        },
      },
    });
    expect(wrapper.find(".empty-stub").exists()).toBe(true);
    tab.turnActive = false;
  });

  it("思考中提示随进行中计数显示/隐藏", async () => {
    tab.turnActive = true;
    store.activeWorkByThread = { t1: 0 };
    store.itemsByThread["t1"] = ([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    const wrapper = mount(ChatView, { props: { tab },
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
    tab.turnActive = false;
    store.activeWorkByThread = {};
  });

  it("待处理交互内嵌渲染在消息流末尾，回答后移除", async () => {
    store.itemsByThread["t1"] = ([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    const wrapper = mount(ChatView, { props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
          EmptyState: { template: "<div />" },
        },
      },
    });
    expect(wrapper.find(".interaction-bubble").exists()).toBe(false);

    store.interactions.push({
      requestId: 1,
      method: "item/tool/requestUserInput",
      params: {
        questions: [
          {
            id: "q1",
            header: "",
            question: "要处理哪个项目？",
            isOther: false,
            isSecret: false,
            options: [{ label: "A", description: "项目 A" }],
          },
        ],
      },
      at: Date.now(),
    });
    await nextTick();
    expect(wrapper.find(".chat-scroll .interaction-bubble").exists()).toBe(true);

    await wrapper
      .findAll(".interaction-foot .btn")
      .find((b) => b.text().trim() === "提交")!
      .trigger("click");
    expect(respondInteraction).toHaveBeenCalledTimes(1);
    await nextTick();
    expect(wrapper.find(".interaction-bubble").exists()).toBe(false);
  });

  it("交互挂起时不显示“思考中”提示", async () => {
    tab.turnActive = true;
    store.activeWorkByThread = { t1: 0 };
    store.interactions.push({
      requestId: 2,
      method: "item/tool/requestUserInput",
      params: { questions: [] },
      at: Date.now(),
    });
    store.itemsByThread["t1"] = ([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    const wrapper = mount(ChatView, { props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
        },
      },
    });
    expect(wrapper.find(".thinking-chip").exists()).toBe(false);

    store.interactions.splice(0);
    await nextTick();
    expect(wrapper.find(".thinking-chip").exists()).toBe(true);

    tab.turnActive = false;
    store.activeWorkByThread = {};
  });

  it("无障碍播报区域随回合状态更新", async () => {
    store.itemsByThread["t1"] = ([]);
    const wrapper = mount(ChatView, { props: { tab },
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
    tab.turnActive = true;
    await nextTick();
    expect(live.text()).toBe("正在生成回复");
    tab.turnActive = false;
    await nextTick();
    expect(live.text()).toBe("回复完成");
  });

  it("新增消息时仍自动吸底滚动", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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

  it("历史会话加载完成：强制测量后滚动到最新消息", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
      { id: "a2", type: "agentMessage", text: "y" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    tab.loading = true;
    const wrapper = mount(ChatView, {
      props: { tab },
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

    // loading true→false（历史会话整批落地）：settleToBottom 强制测量后滚到底部
    tab.loading = false;
    await nextTick();
    await flushPromises();
    await flushPromises();
    expect(scroller.scrollTop).toBe(1000);
    // 测量类已移除，不残留强制渲染状态
    expect(scroller.classList.contains("measuring")).toBe(false);
    wrapper.unmount();
  });

  it("任意消息变更（itemsRev 增长）时吸底滚动", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
      { id: "a2", type: "agentMessage", text: "y" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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

  it("单次正常上滑即解除吸底，itemsRev 增长不再滚动", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
      { id: "a2", type: "agentMessage", text: "y" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    // 先让吸底写入一次，建立“程序化吸底位置”基线
    store.itemsRev++;
    await nextTick();
    // 用户正常上滑 100px（dist = 100 > 60，scrollTop 低于基线）→ 一次解除
    scroller.scrollTop = 900;
    const scrollEv = new Event("scroll");
    Object.defineProperty(scrollEv, "isTrusted", { get: () => true });
    scroller.dispatchEvent(scrollEv);
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(900); // 已解除，不再被拉回
  });

  it("上滑解除吸底后手动发送（userSendRev 递增）强制回到底部并继续吸底", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsRev++;
    await nextTick(); // 基线：scrollTop = 1000
    // 用户上滑 50px（dist 50 > 32）→ 解除吸底
    scroller.scrollTop = 950;
    const ev = new Event("scroll");
    Object.defineProperty(ev, "isTrusted", { get: () => true });
    scroller.dispatchEvent(ev);
    // 手动发送（sendPrompt 递增 userSendRev）→ 强制回到底部
    store.userSendRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
    // 后续内容到达仍持续吸底
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
  });

  it("排队消息自动发送（仅追加 userMessage）不打断阅读位置", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsRev++;
    await nextTick(); // 基线：scrollTop = 1000
    // 用户上滑 50px（dist 50 > 32）→ 解除吸底
    scroller.scrollTop = 950;
    const ev = new Event("scroll");
    Object.defineProperty(ev, "isTrusted", { get: () => true });
    scroller.dispatchEvent(ev);
    // 队列消息在回合结束后自动落地：追加 userMessage，但 userSendRev 不变
    arr.push({ id: "u1", type: "userMessage", text: "第二条" } as ThreadItem);
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(950); // 不被拉回，保持阅读位置
  });

  it("轻微上滑（不足解除阈值）不解除，新内容到达仍吸底跟随", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsRev++;
    await nextTick(); // 基线：scrollTop = 1000
    // 用户仅上滑 20px（dist = 20 < 32 阈值）→ 不解除
    scroller.scrollTop = 980;
    const scrollEv = new Event("scroll");
    Object.defineProperty(scrollEv, "isTrusted", { get: () => true });
    scroller.dispatchEvent(scrollEv);
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1000); // 仍吸底并跟随
  });

  it("上滑超过解除阈值（50px）一次即解除", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsRev++;
    await nextTick(); // 基线：scrollTop = 1000
    // 用户上滑 50px（dist = 50 > 32 阈值）→ 一次解除
    scroller.scrollTop = 950;
    const scrollEv = new Event("scroll");
    Object.defineProperty(scrollEv, "isTrusted", { get: () => true });
    scroller.dispatchEvent(scrollEv);
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(950); // 已解除，不再被拉回
  });

  it("切换会话后重置吸底状态", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsRev++;
    await nextTick(); // 基线：scrollTop = 1000
    // 用户上滑 50px（dist 50 > 32）→ 解除吸底
    scroller.scrollTop = 950;
    const scrollEv = new Event("scroll");
    Object.defineProperty(scrollEv, "isTrusted", { get: () => true });
    scroller.dispatchEvent(scrollEv);
    // 切换到新会话 → 重置吸底并回到底部
    tab.threadId = "t2";
    await nextTick();
    await flushPromises();
    await flushPromises();
    expect(scroller.scrollTop).toBe(1000);
    // 新内容到达后继续吸底
    store.itemsRev++;
    await nextTick();
    await flushPromises();
    expect(scroller.scrollTop).toBe(1000);
  });

  it("追赶窗口内向下滚动不解除吸底", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsRev++;
    await nextTick(); // 基线：scrollTop = 1000
    // 内容 chunk 落地，用户向下滚进新内容（scrollTop 高于基线）→ 不解除
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1400,
    });
    scroller.scrollTop = 1100;
    const scrollEv = new Event("scroll");
    Object.defineProperty(scrollEv, "isTrusted", { get: () => true });
    scroller.dispatchEvent(scrollEv);
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1400); // 仍吸底并跟随到新底部
  });

  it("解除吸底后滚回底部附近自动恢复", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsRev++;
    await nextTick(); // 基线：scrollTop = 1000
    // 先上滑解除
    scroller.scrollTop = 900;
    const ev1 = new Event("scroll");
    Object.defineProperty(ev1, "isTrusted", { get: () => true });
    scroller.dispatchEvent(ev1);
    // 滚回距底部 60px 内（dist = 50）→ 恢复吸底
    scroller.scrollTop = 950;
    const ev2 = new Event("scroll");
    Object.defineProperty(ev2, "isTrusted", { get: () => true });
    scroller.dispatchEvent(ev2);
    store.itemsRev++;
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
  });

  it("程序化未信任 scroll 事件不解除吸底", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, { props: { tab },
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

describe("ChatView 计划已就绪气泡", () => {
  beforeEach(() => {
    tab.planPrompt = null;
  });

  it("planPrompt 设置后消息流末尾渲染计划气泡，解决后移除", async () => {
    store.itemsByThread["t1"] = ([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    const wrapper = mount(ChatView, { props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
          EmptyState: { template: "<div />" },
        },
      },
    });
    expect(wrapper.find(".chat-scroll .interaction-bubble").exists()).toBe(false);

    tab.planPrompt = {
      threadId: "t1",
      turnId: "turn-1",
      planText: "# 修复方案",
    };
    await nextTick();
    const bubbles = wrapper.findAll(".chat-scroll .interaction-bubble");
    expect(bubbles).toHaveLength(1);
    expect(bubbles[0].text()).toContain("计划已就绪");
    expect(bubbles[0].text()).not.toContain("# 修复方案");

    tab.planPrompt = null;
    await nextTick();
    expect(wrapper.find(".chat-scroll .interaction-bubble").exists()).toBe(false);
    wrapper.unmount();
  });

  it("与交互气泡并存时 InlineInteraction 在前、计划气泡在后", async () => {
    store.itemsByThread["t1"] = ([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.interactions.push({
      requestId: 3,
      method: "item/commandExecution/requestApproval",
      params: { command: "echo hi", reason: "测试" },
      at: Date.now(),
    });
    tab.planPrompt = {
      threadId: "t1",
      turnId: "turn-1",
      planText: "# 修复方案",
    };
    const wrapper = mount(ChatView, { props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: true,
          EmptyState: { template: "<div />" },
        },
      },
    });
    await nextTick();
    const bubbles = wrapper.findAll(".chat-scroll .interaction-bubble");
    expect(bubbles).toHaveLength(2);
    expect(bubbles[0].text()).toContain("批准执行命令");
    expect(bubbles[1].text()).toContain("计划已就绪");
    wrapper.unmount();
    store.interactions.splice(0);
    tab.planPrompt = null;
  });
});

describe("ChatView Updated Plan 任务清单", () => {
  it("有 plan 时渲染计划卡片（说明 + 步骤状态）", () => {
    tab = reactive(makeTab());
    tab.plan = {
      explanation: "分两步完成",
      steps: [
        { step: "第一步", status: "inProgress" },
        { step: "第二步", status: "completed" },
        { step: "第三步", status: "pending" },
      ],
    };
    const wrapper = mount(ChatView, {
      props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: { template: "<div class='msg-stub' />" },
        },
      },
    });
    expect(wrapper.find(".assistant-card").exists()).toBe(true);
    expect(wrapper.find(".assistant-card.plan-card").exists()).toBe(true);
    expect(wrapper.text()).toContain("分两步完成");
    expect(wrapper.findAll(".plan-step")).toHaveLength(3);
    expect(wrapper.find(".plan-step.in-progress").exists()).toBe(true);
    expect(wrapper.find(".plan-step.done").exists()).toBe(true);
    expect(wrapper.find(".plan-step.pending").exists()).toBe(true);
    wrapper.unmount();
  });

  it("plan 为 null 时不渲染计划卡片", () => {
    tab = reactive(makeTab());
    const wrapper = mount(ChatView, {
      props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: { template: "<div class='msg-stub' />" },
        },
      },
    });
    expect(wrapper.find(".assistant-card").exists()).toBe(false);
    wrapper.unmount();
  });
});

describe("ChatView 右上角会话 token 用量", () => {
  it("有输入/输出时渲染，缺失或缺一不渲染", async () => {
    tab = reactive(makeTab());
    tab.threadTokenUsage = {
      used: 5000,
      window: 10000,
      input: 12000,
      output: 34000,
    };
    const wrapper = mount(ChatView, {
      props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: { template: "<div class='msg-stub' />" },
        },
      },
    });
    const el = wrapper.find(".chat-token-usage");
    expect(el.exists()).toBe(true);
    expect(el.findAll(".token-usage-part svg")).toHaveLength(2);
    expect(el.findAll(".token-usage-part").map((x) => x.text())).toEqual([
      "12K",
      "34K",
    ]);
    expect(el.text()).toContain("12K");
    expect(el.text()).toContain("34K");

    tab.threadTokenUsage = { used: 5000, window: 10000 };
    await nextTick();
    expect(wrapper.find(".chat-token-usage").exists()).toBe(false);
    wrapper.unmount();
  });
});
