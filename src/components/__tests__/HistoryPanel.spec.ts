import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, deleteThread: vi.fn(), togglePin: vi.fn() };
});

import HistoryPanel from "../HistoryPanel.vue";
import { deleteThread, togglePin, store } from "../../composables/useCodex";

const mockedDelete = vi.mocked(deleteThread);
const mockedTogglePin = vi.mocked(togglePin);

const now = Date.now() / 1000;
const threads = [
  { id: "t1", name: "会话一", preview: "预览一", createdAt: now, recencyAt: now },
  { id: "t2", name: null, preview: "仅预览", createdAt: now, recencyAt: now },
];

describe("HistoryPanel 删除确认", () => {
  beforeEach(() => {
    store.threads = threads.map((t) => ({ ...t }));
    store.loadingHistory = false;
    store.nextCursor = null;
    mockedDelete.mockClear();
    mockedDelete.mockResolvedValue(undefined);
  });

  it("点击 × 弹出确认框，且不直接删除", async () => {
    const wrapper = mount(HistoryPanel);
    await wrapper.find(".history-actions .act-btn.del").trigger("click");

    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    expect(wrapper.text()).toContain("确定删除会话「会话一」吗？此操作不可恢复。");
    expect(mockedDelete).not.toHaveBeenCalled();
  });

  it("确认框弹出后聚焦「删除」按钮", async () => {
    const wrapper = mount(HistoryPanel, { attachTo: document.body });
    await wrapper.find(".history-actions .act-btn.del").trigger("click");
    await wrapper.vm.$nextTick();

    const danger = wrapper.find(".modal-foot .btn.danger");
    expect((danger.element as HTMLElement).isConnected).toBe(true);
    expect(document.activeElement).toBe(danger.element);
    wrapper.unmount();
  });

  it("点击「取消」关闭确认框且不删除", async () => {
    const wrapper = mount(HistoryPanel);
    await wrapper.find(".history-actions .act-btn.del").trigger("click");
    const cancel = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "取消");
    await cancel!.trigger("click");

    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(mockedDelete).not.toHaveBeenCalled();
  });

  it("点击「删除」调用 deleteThread 并关闭确认框", async () => {
    const wrapper = mount(HistoryPanel);
    await wrapper.find(".history-actions .act-btn.del").trigger("click");
    const del = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "删除");
    await del!.trigger("click");

    expect(mockedDelete).toHaveBeenCalledTimes(1);
    expect(mockedDelete).toHaveBeenCalledWith("t1");
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
  });

  it("按 Escape 关闭确认框且不删除", async () => {
    const wrapper = mount(HistoryPanel);
    await wrapper.find(".history-actions .act-btn.del").trigger("click");
    expect(wrapper.find(".modal-mask").exists()).toBe(true);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();

    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(mockedDelete).not.toHaveBeenCalled();
  });
});

describe("HistoryPanel 置顶", () => {
  beforeEach(() => {
    store.threads = threads.map((t) => ({ ...t }));
    store.loadingHistory = false;
    store.nextCursor = null;
    mockedTogglePin.mockClear();
    mockedTogglePin.mockResolvedValue(undefined);
  });

  it("未置顶会话不显示徽章，点击图钉调用 togglePin(id, true)", async () => {
    const wrapper = mount(HistoryPanel);
    expect(wrapper.find(".pin-badge").exists()).toBe(false);
    await wrapper.find('[aria-label="固定置顶"]').trigger("click");
    expect(mockedTogglePin).toHaveBeenCalledWith("t1", true);
  });

  it("置顶会话显示“置顶”徽章，点击图钉调用 togglePin(id, false)", async () => {
    store.threads = [
      { ...threads[0], isPinned: true },
      { ...threads[1] },
    ];
    const wrapper = mount(HistoryPanel);
    expect(wrapper.find(".pin-badge").exists()).toBe(true);
    expect(wrapper.find(".pin-badge").text()).toContain("置顶");
    await wrapper.find('[aria-label="取消固定"]').trigger("click");
    expect(mockedTogglePin).toHaveBeenCalledWith("t1", false);
  });
});

describe("HistoryPanel 目录分组", () => {
  beforeEach(() => {
    store.threads = [];
    store.loadingHistory = false;
    store.nextCursor = null;
    store.searchActive = false;
    store.searchSnippets = {};
  });

  const dirThreads = [
    {
      id: "t1",
      name: "会话一",
      createdAt: now,
      recencyAt: 3,
      cwd: "D:\\codex\\codex-ui",
    },
    {
      id: "t2",
      name: "会话二",
      createdAt: now,
      recencyAt: 2,
      cwd: "D:\\codex\\codex-ui",
    },
    {
      id: "t3",
      name: "会话三",
      createdAt: now,
      recencyAt: 1,
      cwd: "D:\\codex\\codex-proxy",
    },
  ];

  it("相同 cwd 合并为目录行：显示目录名与数量，仅 1 条也建目录", async () => {
    store.threads = dirThreads.map((t) => ({ ...t }));
    const wrapper = mount(HistoryPanel);

    const folders = wrapper.findAll(".history-folder");
    expect(folders).toHaveLength(2);
    expect(folders[0].find(".folder-name").text()).toBe("codex-ui");
    expect(folders[0].find(".folder-count").text()).toBe("2");
    expect(folders[1].find(".folder-name").text()).toBe("codex-proxy");
    expect(folders[1].find(".folder-count").text()).toBe("1");
    // 默认收起：目录内会话均不可见
    expect(wrapper.findAll(".history-item")).toHaveLength(0);
  });

  it("目录默认收起，点击目录行展开/再点收起", async () => {
    store.threads = dirThreads.map((t) => ({ ...t }));
    const wrapper = mount(HistoryPanel);
    expect(wrapper.findAll(".history-item")).toHaveLength(0);
    expect(wrapper.findAll(".history-folder")[0].classes()).toContain("collapsed");

    await wrapper.findAll(".history-folder")[0].trigger("click");
    expect(wrapper.findAll(".history-item")).toHaveLength(2);
    expect(wrapper.findAll(".history-folder")[0].classes()).not.toContain("collapsed");

    await wrapper.findAll(".history-folder")[0].trigger("click");
    expect(wrapper.findAll(".history-item")).toHaveLength(0);
    expect(wrapper.findAll(".history-folder")[0].classes()).toContain("collapsed");
  });

  it("含置顶会话的目录排最前，置顶会话仍留在目录内", async () => {
    store.threads = [
      { ...dirThreads[2] },
      { ...dirThreads[0], isPinned: true },
      { ...dirThreads[1] },
    ];
    const wrapper = mount(HistoryPanel);

    const names = wrapper
      .findAll(".folder-name")
      .map((n) => n.text());
    expect(names).toEqual(["codex-ui", "codex-proxy"]);
    // 展开置顶目录后，置顶徽章仍显示在目录内的会话上
    await wrapper.findAll(".history-folder")[0].trigger("click");
    expect(wrapper.findAll(".pin-badge")).toHaveLength(1);
  });

  it("无 cwd 的会话平铺显示，不建目录", async () => {
    store.threads = [
      { id: "x", name: "平铺", createdAt: now, recencyAt: 5 },
      { id: "y", name: "目录内", createdAt: now, recencyAt: 4, cwd: "D:\\repo" },
    ];
    const wrapper = mount(HistoryPanel);

    expect(wrapper.findAll(".history-folder")).toHaveLength(1);
    // 目录默认收起，仅平铺会话可见
    expect(wrapper.findAll(".history-item")).toHaveLength(1);
    expect(wrapper.findAll(".history-item:not(.folder-item)")).toHaveLength(1);
  });
});
