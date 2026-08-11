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
