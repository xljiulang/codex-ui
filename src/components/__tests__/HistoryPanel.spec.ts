import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    deleteThread: vi.fn(),
    togglePin: vi.fn(),
    openThread: vi.fn(),
    refreshThreads: vi.fn(),
    searchThreads: vi.fn(),
  };
});

import HistoryPanel from "../HistoryPanel.vue";
import {
  deleteThread,
  openThread,
  refreshThreads,
  searchThreads,
  store,
  togglePin,
} from "../../composables/useCodex";

const mockedDelete = vi.mocked(deleteThread);
const mockedTogglePin = vi.mocked(togglePin);
const mockedOpen = vi.mocked(openThread);
const mockedRefresh = vi.mocked(refreshThreads);
const mockedSearch = vi.mocked(searchThreads);

const now = Date.now() / 1000;
const threads = [
  { id: "t1", name: "会话一", preview: "预览一", createdAt: now, recencyAt: now },
  { id: "t2", name: null, preview: "仅预览", createdAt: now, recencyAt: now },
];

async function openCtxMenu(wrapper: VueWrapper, index = 0) {
  await wrapper.findAll(".history-item")[index].trigger("contextmenu", {
    clientX: 200,
    clientY: 200,
  });
}

async function clickCtxItem(wrapper: VueWrapper, label: string) {
  const btn = wrapper
    .findAll(".ctx-menu-item")
    .find((b) => b.text().trim() === label);
  expect(btn).toBeTruthy();
  await btn!.trigger("click");
}

function mockBasicHistory() {
  store.threads = threads.map((t) => ({ ...t }));
  store.loadingHistory = false;
}

describe("HistoryPanel 删除确认", () => {
  beforeEach(() => {
    mockBasicHistory();
    mockedDelete.mockClear();
    mockedDelete.mockResolvedValue(undefined);
  });

  it("右键菜单点删除弹出确认框，且不直接删除", async () => {
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    await clickCtxItem(wrapper, "删除会话");

    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    expect(wrapper.text()).toContain("确定删除会话「会话一」吗？此操作不可恢复。");
    expect(mockedDelete).not.toHaveBeenCalled();
  });

  it("确认框弹出后聚焦「删除」按钮", async () => {
    const wrapper = mount(HistoryPanel, { attachTo: document.body });
    await openCtxMenu(wrapper);
    await clickCtxItem(wrapper, "删除会话");
    await wrapper.vm.$nextTick();

    const danger = wrapper.find(".modal-foot .btn.danger");
    expect((danger.element as HTMLElement).isConnected).toBe(true);
    expect(document.activeElement).toBe(danger.element);
    wrapper.unmount();
  });

  it("点击「取消」关闭确认框且不删除", async () => {
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    await clickCtxItem(wrapper, "删除会话");
    const cancel = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "取消");
    await cancel!.trigger("click");

    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(mockedDelete).not.toHaveBeenCalled();
  });

  it("点击「删除」调用 deleteThread 并关闭确认框", async () => {
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    await clickCtxItem(wrapper, "删除会话");
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
    await openCtxMenu(wrapper);
    await clickCtxItem(wrapper, "删除会话");
    expect(wrapper.find(".modal-mask").exists()).toBe(true);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();

    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(mockedDelete).not.toHaveBeenCalled();
  });
});

describe("HistoryPanel 置顶", () => {
  beforeEach(() => {
    mockBasicHistory();
    mockedTogglePin.mockClear();
    mockedTogglePin.mockResolvedValue(undefined);
  });

  it("未置顶会话不显示徽章，右键点置顶调用 togglePin(id, true)", async () => {
    const wrapper = mount(HistoryPanel);
    expect(wrapper.find(".pin-badge").exists()).toBe(false);
    await openCtxMenu(wrapper);
    await clickCtxItem(wrapper, "置顶");
    expect(mockedTogglePin).toHaveBeenCalledWith("t1", true);
  });

  it("置顶会话显示置顶图标，右键点取消置顶调用 togglePin(id, false)", async () => {
    store.threads = [
      { ...threads[0], isPinned: true },
      { ...threads[1] },
    ];
    const wrapper = mount(HistoryPanel);
    const badge = wrapper.find(".pin-badge");
    expect(badge.exists()).toBe(true);
    expect(badge.find("svg").exists()).toBe(true);
    expect(badge.attributes("aria-label")).toBe("已置顶");
    expect(badge.text()).not.toContain("置顶");
    // 置顶图标位于标题行最左侧，与标题同一行垂直居中
    expect(wrapper.find(".history-title-row :first-child").classes()).toContain(
      "pin-badge",
    );
    await openCtxMenu(wrapper);
    await clickCtxItem(wrapper, "取消置顶");
    expect(mockedTogglePin).toHaveBeenCalledWith("t1", false);
  });
});

describe("HistoryPanel 右键菜单", () => {
  beforeEach(() => {
    mockBasicHistory();
    store.searchActive = false;
    store.searchSnippets = {};
    mockedOpen.mockClear();
  });

  it("右键会话行显示四项菜单：打开/置顶/重命名/删除会话", async () => {
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    const labels = wrapper
      .findAll(".ctx-menu-item")
      .map((b) => b.text().trim());
    expect(labels).toEqual(["打开", "置顶", "重命名", "删除会话"]);
  });

  it("置顶会话右键菜单显示“取消置顶”", async () => {
    store.threads = [
      { ...threads[0], isPinned: true },
      { ...threads[1] },
    ];
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    const labels = wrapper
      .findAll(".ctx-menu-item")
      .map((b) => b.text().trim());
    expect(labels).toContain("取消置顶");
  });

  it("菜单每项都带图标，删除项保留 danger 类", async () => {
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items).toHaveLength(4);
    for (const it of items) {
      expect(it.find("svg").exists()).toBe(true);
    }
    expect(wrapper.findAll(".ctx-menu-item.danger")).toHaveLength(1);
  });

  it("点击“打开”调用 openThread 并关闭菜单", async () => {
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    await clickCtxItem(wrapper, "打开");
    expect(mockedOpen).toHaveBeenCalledWith("t1");
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
  });

  it("按 Escape 关闭菜单", async () => {
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    expect(wrapper.find(".ctx-menu").exists()).toBe(true);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();

    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
  });

  it("点击外部关闭菜单", async () => {
    const wrapper = mount(HistoryPanel);
    await openCtxMenu(wrapper);
    expect(wrapper.find(".ctx-menu").exists()).toBe(true);

    window.dispatchEvent(new MouseEvent("click"));
    await wrapper.vm.$nextTick();

    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
  });
});

describe("HistoryPanel 宽度调节", () => {
  beforeEach(() => {
    store.threads = [];
    store.loadingHistory = false;
  });

  it("向左拖拽加宽面板，并钳制在半个窗口宽度内", async () => {
    const wrapper = mount(HistoryPanel);
    const handle = wrapper.find(".history-resize-handle");
    await handle.trigger("pointerdown", { clientX: 500 });
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 300 }));
    await wrapper.vm.$nextTick();
    // 264 + (500 - 300) = 464，未超过 window.innerWidth / 2（happy-dom 默认 1024）
    expect(wrapper.find(".history-panel").attributes("style")).toContain(
      "width: 464px",
    );

    window.dispatchEvent(new PointerEvent("pointermove", { clientX: -1000 }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".history-panel").attributes("style")).toContain(
      "width: 512px",
    );

    window.dispatchEvent(new PointerEvent("pointerup"));
    wrapper.unmount();
  });

  it("向右拖拽不窄于默认宽度 264px", async () => {
    const wrapper = mount(HistoryPanel);
    const handle = wrapper.find(".history-resize-handle");
    await handle.trigger("pointerdown", { clientX: 100 });
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 5000 }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".history-panel").attributes("style")).toContain(
      "width: 264px",
    );

    window.dispatchEvent(new PointerEvent("pointerup"));
    wrapper.unmount();
  });
});

describe("HistoryPanel 搜索刷新", () => {
  beforeEach(() => {
    mockBasicHistory();
    store.searchActive = false;
    store.searchSnippets = {};
    mockedRefresh.mockClear();
    mockedSearch.mockClear();
  });

  it("普通态点击刷新调用 refreshThreads", async () => {
    const wrapper = mount(HistoryPanel);
    mockedRefresh.mockClear();
    await wrapper.find('button[aria-label="刷新"]').trigger("click");
    expect(mockedRefresh).toHaveBeenCalledTimes(1);
    expect(mockedSearch).not.toHaveBeenCalled();
  });

  it("搜索态点击刷新重新执行当前搜索词", async () => {
    store.searchActive = true;
    const wrapper = mount(HistoryPanel);
    mockedRefresh.mockClear();
    mockedSearch.mockClear();
    await wrapper.find(".history-search").setValue("codex");
    await wrapper.find('button[aria-label="刷新"]').trigger("click");
    expect(mockedSearch).toHaveBeenCalledWith("codex");
    expect(mockedRefresh).not.toHaveBeenCalled();
  });

  it("搜索框与刷新按钮组成胶囊组", () => {
    const wrapper = mount(HistoryPanel);
    const group = wrapper.find(".history-search-group");
    expect(group.find("input.history-search").exists()).toBe(true);
    expect(group.find('button[aria-label="刷新"]').exists()).toBe(true);
  });
});

describe("HistoryPanel 目录分组", () => {
  beforeEach(() => {
    store.threads = [];
    store.loadingHistory = false;
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
    // 目录按名称 A-Z：codex-proxy 在 codex-ui 之前
    expect(folders[0].find(".folder-name").text()).toBe("codex-proxy");
    expect(folders[0].find(".folder-count").text()).toBe("1");
    expect(folders[1].find(".folder-name").text()).toBe("codex-ui");
    expect(folders[1].find(".folder-count").text()).toBe("2");
    // 默认收起：目录内会话均不可见
    expect(wrapper.findAll(".history-item")).toHaveLength(0);
  });

  it("目录默认收起，点击目录行展开/再点收起", async () => {
    store.threads = dirThreads.map((t) => ({ ...t }));
    const wrapper = mount(HistoryPanel);
    expect(wrapper.findAll(".history-item")).toHaveLength(0);
    expect(wrapper.findAll(".history-folder")[0].classes()).toContain("collapsed");

    await wrapper.findAll(".history-folder")[1].trigger("click");
    expect(wrapper.findAll(".history-item")).toHaveLength(2);
    expect(wrapper.findAll(".history-folder")[1].classes()).not.toContain("collapsed");

    await wrapper.findAll(".history-folder")[1].trigger("click");
    expect(wrapper.findAll(".history-item")).toHaveLength(0);
    expect(wrapper.findAll(".history-folder")[1].classes()).toContain("collapsed");
  });

  it("目录按名称 A-Z 排序，置顶会话仍留在目录内", async () => {
    store.threads = [
      { ...dirThreads[2] },
      { ...dirThreads[0], isPinned: true },
      { ...dirThreads[1] },
    ];
    const wrapper = mount(HistoryPanel);

    const names = wrapper
      .findAll(".folder-name")
      .map((n) => n.text());
    expect(names).toEqual(["codex-proxy", "codex-ui"]);
    // 展开 codex-ui（第二个目录）后，置顶图标仍显示在目录内的会话上
    await wrapper.findAll(".history-folder")[1].trigger("click");
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

  it("目录行带文件夹图标", async () => {
    store.threads = dirThreads.map((t) => ({ ...t }));
    const wrapper = mount(HistoryPanel);
    expect(wrapper.findAll(".history-folder .folder-icon svg")).toHaveLength(2);
  });
});
