import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockSetToast = vi.hoisted(() => vi.fn());
const mockToastError = vi.hoisted(() => (e: unknown) => String(e));
const mockStore = vi.hoisted(() => ({}));

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<
    typeof import("../../composables/useCodex")
  >();
  return {
    ...mod,
    setToast: mockSetToast,
    toastError: mockToastError,
    store: mockStore,
  };
});

const mockList = vi.hoisted(() => vi.fn());
const mockCreate = vi.hoisted(() => vi.fn());
const mockApply = vi.hoisted(() => vi.fn());
const mockDelete = vi.hoisted(() => vi.fn());
const mockOpen = vi.hoisted(() => vi.fn());

vi.mock("../../composables/useModelSnapshots", () => ({
  listModelSnapshots: mockList,
  createModelSnapshot: mockCreate,
  applyModelSnapshot: mockApply,
  deleteModelSnapshot: mockDelete,
  openModelSnapshot: mockOpen,
}));

import ModelSnapshotMenu from "../ModelSnapshotMenu.vue";
import { tooltipDirective } from "../../directives/tooltip";
import { ICON_CLOSE, ICON_OPEN, ICON_RESTORE } from "../../lib/icons";

function mountMenu() {
  return mount(ModelSnapshotMenu, {
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("ModelSnapshotMenu", () => {
  beforeEach(() => {
    mockList.mockReset().mockResolvedValue(["a", "b"]);
    mockCreate.mockReset();
    mockApply.mockReset();
    mockDelete.mockReset();
    mockOpen.mockReset();
    mockSetToast.mockReset();
  });

  it("点模型快照按钮展开菜单并拉取快照列表", async () => {
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="模型快照"]').trigger("click");
    await flushPromises();
    expect(mockList).toHaveBeenCalledOnce();
    expect(wrapper.find(".model-snapshot-title").text()).toContain("模型快照");
    expect(wrapper.text()).toContain("a");
    expect(wrapper.text()).toContain("b");
  });

  it("触发按钮为纯图标（外框 + M，无文字）", async () => {
    const wrapper = mountMenu();
    const btn = wrapper.find('button[aria-label="模型快照"]');
    expect(btn.find("img").exists()).toBe(false);
    const icon = btn.find(".model-snapshot-brace");
    expect(icon.exists()).toBe(true);
    // 4 个直角边框 + 1 个 M
    expect(icon.findAll("path").length).toBe(5);
    expect(
      icon.findAll("path").every((p) => p.attributes("fill") === "none"),
    ).toBe(true);
    expect(btn.text().trim()).toBe("");
  });

  it("点还原按钮还原模型快照", async () => {
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="模型快照"]').trigger("click");
    await flushPromises();
    const restore = wrapper.find('button[aria-label="还原模型快照a"]');
    expect(restore.find("path").attributes("d")).toBe(ICON_RESTORE);
    await restore.trigger("click");
    await flushPromises();
    expect(mockApply).toHaveBeenCalledWith("a");
  });

  it("点 × 删除模型快照", async () => {
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="模型快照"]').trigger("click");
    await flushPromises();
    const del = wrapper.find('button[aria-label="删除模型快照b"]');
    expect(del.find("path").attributes("d")).toBe(ICON_CLOSE);
    const row = del.element.closest(".model-snapshot-row") as HTMLElement;
    const restore = row.querySelector('button[aria-label="还原模型快照b"]');
    const openBtn = row.querySelector('button[aria-label="打开模型快照b"]');
    expect(restore).toBeTruthy();
    expect(openBtn).toBeTruthy();
    await del.trigger("click");
    await flushPromises();
    expect(mockDelete).toHaveBeenCalledWith("b");
  });

  it("点打开按钮打开模型快照 JSON", async () => {
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="模型快照"]').trigger("click");
    await flushPromises();
    const openBtn = wrapper.find('button[aria-label="打开模型快照b"]');
    expect(openBtn.find("path").attributes("d")).toBe(ICON_OPEN);
    await openBtn.trigger("click");
    await flushPromises();
    expect(mockOpen).toHaveBeenCalledWith("b");
  });

  it("底部新建：输入名字并点新建创建模型快照", async () => {
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="模型快照"]').trigger("click");
    await flushPromises();
    const input = wrapper.find(".git-branch-input");
    expect(input.attributes("placeholder")).toBe("新建模型快照");
    await input.setValue("dev");
    await wrapper.find(".git-branch-create-btn").trigger("click");
    await flushPromises();
    expect(mockCreate).toHaveBeenCalledWith("dev");
    expect(wrapper.find(".model-snapshot-menu").exists()).toBe(true);
  });
});
