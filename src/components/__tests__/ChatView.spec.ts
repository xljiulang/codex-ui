import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
    setToast: vi.fn(),
    toastError: (e: unknown) => (e instanceof Error ? e.message : String(e)),
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
import { invoke } from "@tauri-apps/api/core";
import { respondInteraction, setToast, store } from "../../composables/useCodex";
import type { ThreadItem } from "../../lib/types";
import type { SessionTab } from "../../composables/useCodex";

const mockedInvoke = vi.mocked(invoke);

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
    collaborationMode: "default",
    model: null,
    effort: null,
    plugins: { plugins: [], loaded: false },
    skills: { skills: [], loaded: false },
    creatingChat: false,
    draftJson: JSON.stringify({ type: "doc", content: [] }),
    draftAttachments: [],
    draftRefs: {},
    origin: "session",
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
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
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
    expect(wrapper.find(".waiting-chip").exists()).toBe(true);
    expect(wrapper.find(".waiting-chip").text()).toMatch(
      /等待响应\(\d+\.\d+s\)/,
    );
    await vi.advanceTimersByTime(1500);
    expect(wrapper.find(".waiting-chip").text()).toMatch(
      /等待响应\([1-9]\d*\.\d+s\)/,
    );
    store.activeWorkByThread.t1 = 1;
    await nextTick();
    expect(wrapper.find(".waiting-chip").exists()).toBe(false);
    tab.turnActive = false;
    store.activeWorkByThread = {};
    wrapper.unmount();
    vi.useRealTimers();
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
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
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
    expect(wrapper.find(".waiting-chip").exists()).toBe(false);

    store.interactions.splice(0);
    await nextTick();
    expect(wrapper.find(".waiting-chip").exists()).toBe(true);
    expect(wrapper.find(".waiting-chip").text()).toMatch(
      /等待响应\(\d+\.\d+s\)/,
    );

    tab.turnActive = false;
    store.activeWorkByThread = {};
    wrapper.unmount();
    vi.useRealTimers();
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

  it("上滑解除吸底后显示圆形回到底部图标按钮，点击后恢复吸底并隐藏", async () => {
    const arr = reactive([
      { id: "a1", type: "agentMessage", text: "x" } as ThreadItem,
    ]);
    store.itemsByThread["t1"] = (arr);
    const wrapper = mount(ChatView, {
      props: { tab },
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
    await nextTick(); // 基线：scrollTop = 1000，吸底中不显示按钮
    expect(wrapper.find(".scroll-bottom-btn").exists()).toBe(false);

    // 用户上滑 50px（dist 50 > 32）→ 解除吸底，显示图标按钮
    scroller.scrollTop = 950;
    const ev = new Event("scroll");
    Object.defineProperty(ev, "isTrusted", { get: () => true });
    scroller.dispatchEvent(ev);
    await nextTick();
    const btn = wrapper.find(".scroll-bottom-btn");
    expect(btn.exists()).toBe(true);
    expect(btn.attributes("aria-label")).toBe("回到底部");
    expect(btn.find("svg").exists()).toBe(true);

    // 点击后回到底部并恢复吸底，按钮随即隐藏
    await btn.trigger("click");
    await nextTick();
    expect(scroller.scrollTop).toBe(1000);
    expect(wrapper.find(".scroll-bottom-btn").exists()).toBe(false);
    wrapper.unmount();
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

describe("ChatView 回合定位按钮", () => {
  let rects: WeakMap<Element, number>;

  function rect(top = 0): DOMRect {
    return {
      top,
      bottom: top,
      left: 0,
      right: 0,
      width: 0,
      height: 0,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect;
  }

  function anchorStub() {
    return {
      props: ["item"],
      template:
        "<div class='msg-stub' :data-turn-anchor=\"item.type === 'userMessage' ? '' : null\">{{ item.type }}</div>",
    };
  }

  function userThread(count: number): ThreadItem[] {
    const arr: ThreadItem[] = [];
    for (let i = 1; i <= count; i++) {
      arr.push({
        id: `u${i}`,
        type: "userMessage",
        content: [
          {
            type: "text",
            text: `问题${i}`,
            text_elements: [],
          },
        ],
      } as ThreadItem);
      if (i < count) {
        arr.push({
          id: `a${i}`,
          type: "agentMessage",
          text: `回答${i}`,
        } as ThreadItem);
      }
    }
    return arr;
  }

  function mountChat() {
    return mount(ChatView, {
      props: { tab },
      global: {
        stubs: {
          ComposerBar: true,
          MessageItem: anchorStub(),
        },
      },
    });
  }

  /** 按锚点序注入视口几何，并触发一次锚点重算 */
  async function installGeometry(
    wrapper: ReturnType<typeof mountChat>,
    tops: number[],
  ) {
    const scroller = wrapper.find(".chat-scroll").element as HTMLElement;
    rects.set(scroller, 0);
    const nodes = wrapper.findAll("[data-turn-anchor]");
    nodes.forEach((n, i) => rects.set(n.element, tops[i] ?? 0));
    // 保持 scrollTop=0 时重算，使内容坐标等于注入的 top；滚动高度稍后再补
    store.itemsRev++;
    await nextTick();
    await flushPromises();
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    return scroller;
  }

  function trustedScroll(el: HTMLElement, top: number) {
    el.scrollTop = top;
    const ev = new Event("scroll");
    Object.defineProperty(ev, "isTrusted", { get: () => true });
    el.dispatchEvent(ev);
  }

  beforeEach(() => {
    store.interactions.splice(0);
    tab = reactive(makeTab());
    vi.useFakeTimers();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return undefined;
    });
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
    rects = new WeakMap();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        const el = this;
        if (el.classList.contains("chat-scroll")) return rect(0);
        const scroller = el.closest(".chat-scroll") as HTMLElement | null;
        const scrollTop = scroller ? scroller.scrollTop : 0;
        return rect((rects.get(el) ?? 0) - scrollTop);
      },
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  /** 推进后台全文预热（waitForRenderQuiet 的 120ms + rAF 同步） */
  async function warmReady() {
    await nextTick();
    await vi.advanceTimersByTimeAsync(200);
    await nextTick();
    await flushPromises();
  }

  /** 悬停导航按钮（根容器）展开卡片 */
  async function openNavCard(
    wrapper: ReturnType<typeof mountChat>,
  ) {
    await wrapper.find(".turn-nav").trigger("mouseenter");
    await nextTick();
  }

  it("预热完成前隐藏，完成后 ≥2 个回合显示单个导航按钮；0/1 个不显示", async () => {
    store.itemsByThread["t1"] = reactive(userThread(1));
    const single = mountChat();
    await nextTick();
    await flushPromises();
    expect(single.find(".turn-nav").exists()).toBe(false);
    await warmReady();
    expect(single.find(".turn-nav").exists()).toBe(false);
    single.unmount();

    store.itemsByThread["t1"] = reactive(userThread(2));
    const pair = mountChat();
    await nextTick();
    await flushPromises();
    expect(pair.find(".turn-nav").exists()).toBe(false);
    await warmReady();
    expect(pair.find(".turn-nav").exists()).toBe(true);
    expect(pair.findAll(".turn-nav-btn")).toHaveLength(1);
    pair.unmount();
  });

  it("回合进行中导航图标旋转：等待响应慢速、执行工具中常速，结束后静止", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    const wrapper = mountChat();
    await warmReady();

    // 回合未进行：静止
    let btn = wrapper.find(".turn-nav-btn");
    expect(btn.classes()).not.toContain("spin-slow");
    expect(btn.classes()).not.toContain("spin-fast");

    // 等待响应：回合进行中、无进行中工作 → 慢速
    tab.turnActive = true;
    store.activeWorkByThread = { t1: 0 };
    await nextTick();
    btn = wrapper.find(".turn-nav-btn");
    expect(btn.classes()).toContain("spin-slow");

    // 执行工具中：有进行中工作 → 常速
    store.activeWorkByThread.t1 = 1;
    await nextTick();
    btn = wrapper.find(".turn-nav-btn");
    expect(btn.classes()).toContain("spin-fast");
    expect(btn.classes()).not.toContain("spin-slow");

    // 回合结束 → 静止
    tab.turnActive = false;
    store.activeWorkByThread = {};
    await nextTick();
    btn = wrapper.find(".turn-nav-btn");
    expect(btn.classes()).not.toContain("spin-fast");
    expect(btn.classes()).not.toContain("spin-slow");
  });

  it("悬停导航按钮展开卡片：条目正序、含文本预览、当前回合高亮", async () => {
    store.itemsByThread["t1"] = reactive(userThread(3));
    const wrapper = mountChat();
    await warmReady();
    const scroller = await installGeometry(wrapper, [100, 300, 500]);
    trustedScroll(scroller, 1000);
    await nextTick();

    await openNavCard(wrapper);
    expect(wrapper.find(".turn-nav-card").exists()).toBe(true);
    const items = wrapper.findAll(".turn-nav-item");
    expect(items).toHaveLength(3);
    expect(
      items.map((x) => x.find(".turn-nav-item-title").text()),
    ).toEqual(["问题1", "问题2", "问题3"]);
    expect(items[2].classes()).toContain("active");
    wrapper.unmount();
  });

  it("卡片条目时间跟随消息规则：当天 HH:mm，非当天带日期", async () => {
    vi.setSystemTime(new Date(2026, 8, 3, 12, 0));
    store.itemsByThread["t1"] = reactive([
      {
        id: "u1",
        type: "userMessage",
        startedAtMs: new Date(2026, 8, 3, 9, 5).getTime(),
        content: [{ type: "text", text: "今天的问题", text_elements: [] }],
      } as ThreadItem,
      {
        id: "u2",
        type: "userMessage",
        startedAtMs: new Date(2026, 7, 9, 14, 5).getTime(),
        content: [{ type: "text", text: "早前的问题", text_elements: [] }],
      } as ThreadItem,
    ]);
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);
    expect(
      wrapper.findAll(".turn-nav-item-time").map((x) => x.text()),
    ).toEqual(["09:05", "8月9日 14:05"]);
    wrapper.unmount();
  });

  it("点击卡片条目跳转到对应回合并自动关闭；连续两次点击连续推进", async () => {
    store.itemsByThread["t1"] = reactive(userThread(3));
    const wrapper = mountChat();
    await warmReady();
    const scroller = await installGeometry(wrapper, [100, 300, 500]);
    trustedScroll(scroller, 1000);
    await nextTick();

    await openNavCard(wrapper);
    await flushPromises();
    await wrapper.findAll(".turn-nav-item")[1].trigger("click");
    await nextTick();
    await flushPromises();
    expect(scroller.scrollTop).toBe(300);
    expect(wrapper.find(".turn-nav-card").exists()).toBe(false);
    expect(wrapper.findAll("[data-turn-anchor]")[1].classes()).toContain(
      "turn-highlight",
    );

    // 再次开卡并点击更早回合：连续跳转仍然精确
    await openNavCard(wrapper);
    await wrapper.findAll(".turn-nav-item")[0].trigger("click");
    await nextTick();
    await flushPromises();
    expect(scroller.scrollTop).toBe(100);
    wrapper.unmount();
  });

  it("移出自动收起，Escape 与点击外部均关闭卡片", async () => {
    store.itemsByThread["t1"] = reactive(userThread(3));
    const wrapper = mountChat();
    await warmReady();

    await openNavCard(wrapper);
    expect(wrapper.find(".turn-nav-card").exists()).toBe(true);
    await wrapper.find(".turn-nav").trigger("mouseleave");
    await vi.advanceTimersByTimeAsync(200);
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(false);

    await openNavCard(wrapper);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(false);

    await openNavCard(wrapper);
    document.body.dispatchEvent(
      new MouseEvent("pointerdown", { bubbles: true }) as unknown as PointerEvent,
    );
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(false);
    wrapper.unmount();
  });

  it("纯悬停开关：移出后重新进入立即重开；点击按钮不再切换开关", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    const wrapper = mountChat();
    await warmReady();

    await openNavCard(wrapper);
    expect(wrapper.find(".turn-nav-card").exists()).toBe(true);

    // 移出经 150ms 延迟关闭，再悬停立即重开（不依赖定时器）
    await wrapper.find(".turn-nav").trigger("mouseleave");
    await vi.advanceTimersByTimeAsync(200);
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(false);
    await openNavCard(wrapper);
    expect(wrapper.find(".turn-nav-card").exists()).toBe(true);

    // 鼠标点击按钮不再切换：展开状态下点击仍保持打开，移出后才关闭
    await wrapper.get(".turn-nav-btn").trigger("click");
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(true);
    await wrapper.find(".turn-nav").trigger("mouseleave");
    await vi.advanceTimersByTimeAsync(200);
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(false);
    wrapper.unmount();
  });

  it("间隙穿越不误关：移出后 150ms 内重新进入保持打开", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    const wrapper = mountChat();
    await warmReady();

    await openNavCard(wrapper);
    await wrapper.find(".turn-nav").trigger("mouseleave");
    await wrapper.find(".turn-nav").trigger("mouseenter");
    await vi.advanceTimersByTimeAsync(200);
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(true);
    wrapper.unmount();
  });

  it("键盘 Enter/Space 可开合卡片（无 hover 的键盘路径）", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    const wrapper = mountChat();
    await warmReady();
    const btn = wrapper.get(".turn-nav-btn");

    await btn.trigger("keydown", { key: "Enter" });
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(true);

    await btn.trigger("keydown", { key: " " });
    await nextTick();
    expect(wrapper.find(".turn-nav-card").exists()).toBe(false);
    wrapper.unmount();
  });

  it("切换线程后重置并隐藏回合定位控件", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    const wrapper = mountChat();
    await warmReady();
    expect(wrapper.find(".turn-nav").exists()).toBe(true);

    store.itemsByThread["t2"] = reactive(userThread(3));
    tab.threadId = "t2";
    await nextTick();
    await flushPromises();
    expect(wrapper.find(".turn-nav").exists()).toBe(false);
    await warmReady();
    expect(wrapper.find(".turn-nav").exists()).toBe(true);
    wrapper.unmount();
  });

  it("纯图片回合在卡片中回退为 [图片] 占位文本", async () => {
    store.itemsByThread["t1"] = reactive([
      {
        id: "u1",
        type: "userMessage",
        content: [{ type: "localImage", path: "D:/a.png" }],
      } as ThreadItem,
      {
        id: "u2",
        type: "userMessage",
        content: [
          { type: "text", text: "只看图", text_elements: [] },
        ],
      } as ThreadItem,
    ]);
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);
    const texts = wrapper
      .findAll(".turn-nav-item-title")
      .map((x) => x.text());
    expect(texts[0]).toBe("[图片]");
    expect(texts[1]).toBe("只看图");
    wrapper.unmount();
  });

  it("仅文件消息在导航卡片中降级为 @文件名", async () => {
    store.itemsByThread["t1"] = reactive([
      {
        id: "u1",
        type: "userMessage",
        content: [
          {
            type: "text",
            text: [
              "# Files mentioned by the user:",
              "## a.cs: D:/a.cs",
              "",
              "## My request:",
              "",
            ].join("\n"),
            text_elements: [],
          },
        ],
      } as ThreadItem,
      {
        id: "u2",
        type: "userMessage",
        content: [{ type: "text", text: "普通问题", text_elements: [] }],
      } as ThreadItem,
    ]);
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);
    expect(
      wrapper.findAll(".turn-nav-item-title").map((x) => x.text()),
    ).toEqual(["@a.cs", "普通问题"]);
    wrapper.unmount();
  });

  it("执行计划用户消息以计划标题作为导航标题，普通消息保持纯文本预览", async () => {
    store.itemsByThread["t1"] = reactive([
      {
        id: "u1",
        type: "userMessage",
        content: [
          {
            type: "text",
            text: "PLEASE IMPLEMENT THIS PLAN:\n# 重构方案\n- 步骤1",
            text_elements: [],
          },
        ],
      } as ThreadItem,
      {
        id: "u2",
        type: "userMessage",
        content: [{ type: "text", text: "普通问题", text_elements: [] }],
      } as ThreadItem,
    ]);
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);
    expect(
      wrapper.findAll(".turn-nav-item-title").map((x) => x.text()),
    ).toEqual(["重构方案", "普通问题"]);
    wrapper.unmount();
  });

  it("条目标题使用完整文本（不做 200 字截断，CSS 单行省略负责视觉）", async () => {
    const rawLong = "长".repeat(300);
    store.itemsByThread["t1"] = reactive([
      {
        id: "u1",
        type: "userMessage",
        content: [{ type: "text", text: rawLong, text_elements: [] }],
      } as ThreadItem,
      {
        id: "u2",
        type: "userMessage",
        content: [{ type: "text", text: "短标题", text_elements: [] }],
      } as ThreadItem,
    ]);
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);
    const titles = wrapper.findAll(".turn-nav-item-title");
    expect(titles[0].text()).toBe(rawLong);
    expect(titles[1].text()).toBe("短标题");
    wrapper.unmount();
  });

  it("悬停任一条目显示用户消息气泡预览，移出/收起后隐藏", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);

    const rows = wrapper.findAll(".turn-nav-item");
    await rows[0].trigger("mouseenter");
    await nextTick();
    const preview = wrapper.find(".turn-nav-preview");
    expect(preview.exists()).toBe(true);
    // 预览内渲染该条 userMessage（MessageItem 桩文本为类型名）
    expect(preview.find(".msg-stub").text()).toBe("userMessage");

    // 移出条目：短暂延迟后隐藏预览
    await rows[0].trigger("mouseleave");
    await vi.advanceTimersByTimeAsync(200);
    await nextTick();
    expect(wrapper.find(".turn-nav-preview").exists()).toBe(false);

    // 悬停另一条目再次显示；收起卡片时预览一并消失
    await rows[1].trigger("mouseenter");
    await nextTick();
    expect(wrapper.find(".turn-nav-preview").exists()).toBe(true);
    await wrapper.find(".turn-nav").trigger("mouseleave");
    await vi.advanceTimersByTimeAsync(200);
    await nextTick();
    expect(wrapper.find(".turn-nav-preview").exists()).toBe(false);
    expect(wrapper.find(".turn-nav-card").exists()).toBe(false);
    wrapper.unmount();
  });

  it("导航按钮始终渲染单个图标，不随上下文数据切换", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    tab.threadTokenUsage = { used: 5000, window: 10000 };
    const wrapper = mountChat();
    await warmReady();
    const btn = wrapper.find(".turn-nav-btn");
    expect(btn.text()).toBe("");
    expect(btn.find("svg").exists()).toBe(true);
    expect(btn.classes()).not.toContain("has-pct");

    tab.threadTokenUsage = null;
    await nextTick();
    await flushPromises();
    const btn2 = wrapper.find(".turn-nav-btn");
    expect(btn2.text()).toBe("");
    expect(btn2.find("svg").exists()).toBe(true);
    expect(btn2.classes()).not.toContain("has-pct");
    wrapper.unmount();
  });

  it("底边渲染上下文占用进度条（宽度=占用率，带语义 aria），无数据时不渲染", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    tab.threadTokenUsage = { used: 5000, window: 10000 };
    const wrapper = mountChat();
    await warmReady();
    const bar = wrapper.find(".chat-scroll-wrap .ctx-usage-bar");
    expect(bar.exists()).toBe(true);
    expect(bar.attributes("role")).toBe("progressbar");
    expect(bar.attributes("aria-valuenow")).toBe("50");
    expect(bar.attributes("aria-valuemax")).toBe("100");
    expect(bar.attributes("aria-label")).toContain("已用");
    expect(bar.attributes("aria-label")).toContain("最大");
    const fill = bar.find(".ctx-usage-bar-fill");
    expect(fill.exists()).toBe(true);
    expect(fill.attributes("style")).toContain("width: 50%");

    // 无上下文数据时不渲染底边进度条
    tab.threadTokenUsage = null;
    await nextTick();
    await flushPromises();
    expect(wrapper.find(".ctx-usage-bar").exists()).toBe(false);
    wrapper.unmount();
  });

  it("展开卡片头部渲染标题与 token/上下文/压缩顺序与文案", async () => {
    store.itemsByThread["t1"] = reactive(userThread(3));
    tab.threadTokenUsage = {
      used: 5000,
      window: 10000,
      input: 12000,
      output: 34000,
    };
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);
    const foot = wrapper.find(".turn-nav-foot");
    expect(foot.exists()).toBe(true);
    expect(foot.find(".turn-nav-title").exists()).toBe(false);
    const tokenEl = foot.find(".token-usage-chip");
    expect(tokenEl.findAll(".token-usage-ico")).toHaveLength(3);
    expect(tokenEl.findAll(".token-usage-part").map((x) => x.text())).toEqual([
      "12K",
      "34K",
    ]);
    expect(tokenEl.text()).toContain("12K");
    expect(tokenEl.text()).toContain("34K");
    expect(tokenEl.attributes("aria-label")).toContain("Token消耗");
    expect(tokenEl.attributes("aria-label")).toContain("输入");
    const capsule = foot.find(".ctx-control-capsule");
    expect(capsule.exists()).toBe(true);
    const info = capsule.find(".ctx-usage-text");
    expect(info.text()).toContain("5K");
    expect(info.text()).toContain("/");
    expect(info.text()).toContain("10K");
    expect(info.text()).not.toContain("已用");
    expect(info.text()).not.toContain("最大");
    expect(info.find("svg").exists()).toBe(true);
    expect(info.attributes("aria-label")).toContain("上下文窗口");
    expect(info.attributes("aria-label")).toContain("已用");
    const compact = capsule.find(".ctx-compact-btn");
    expect(compact.find("svg").exists()).toBe(true);
    expect(compact.text()).toBe("");
    // 胶囊内部顺序：用量文本 → 压缩按钮（分隔线由按钮左边框绘制）
    expect(
      capsule
        .findAll(".ctx-usage-text, .ctx-compact-btn")
        .map((x) => x.element.className),
    ).toEqual(["ctx-usage-text", "ctx-compact-btn"]);
    const order = foot.findAll(".token-usage-chip, .ctx-control-capsule");
    expect(order.map((x) => x.element.className)).toEqual([
      "token-usage-chip",
      "ctx-control-capsule",
    ]);
    wrapper.unmount();
  });

  it("点击压缩胶囊发起压缩并提示；压缩中按钮禁用，完成后恢复", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    tab.threadId = "t1";
    tab.threadTokenUsage = { used: 5000, window: 10000 };
    mockedInvoke.mockClear();
    let resolveInvoke!: (v: unknown) => void;
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "thread/compact/start") {
        return new Promise((resolve) => {
          resolveInvoke = resolve;
        });
      }
      return Promise.resolve({});
    });
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);
    const btn = wrapper.find(".ctx-compact-btn");
    expect((btn.element as HTMLButtonElement).disabled).toBe(false);
    await btn.trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/compact/start",
      params: { threadId: "t1" },
    });
    expect(
      (wrapper.find(".ctx-compact-btn").element as HTMLButtonElement).disabled,
    ).toBe(true);
    resolveInvoke({});
    await flushPromises();
    expect(
      (wrapper.find(".ctx-compact-btn").element as HTMLButtonElement).disabled,
    ).toBe(false);
    expect(setToast).toHaveBeenCalledWith("已开始压缩上下文");
    wrapper.unmount();
  });

  it("压缩失败时 toast 错误", async () => {
    store.itemsByThread["t1"] = reactive(userThread(2));
    tab.threadId = "t1";
    tab.threadTokenUsage = { used: 5000, window: 10000 };
    mockedInvoke.mockClear();
    mockedInvoke.mockRejectedValueOnce(new Error("压缩失败"));
    const wrapper = mountChat();
    await warmReady();
    await openNavCard(wrapper);
    await wrapper.find(".ctx-compact-btn").trigger("click");
    await flushPromises();
    expect(setToast).toHaveBeenCalledWith("压缩失败");
    wrapper.unmount();
  });
});
