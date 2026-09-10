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

import ModelSnapshotCard from "../settings/ModelSnapshotCard.vue";
import { tooltipDirective } from "../../directives/tooltip";
import {
  ICON_DELETE,
  ICON_OPEN,
  ICON_PLUS,
  ICON_RESTORE,
} from "../../lib/icons";

function mountCard(active = true) {
  return mount(ModelSnapshotCard, {
    props: { active },
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("ModelSnapshotCard 模型快照卡片", () => {
  beforeEach(() => {
    mockList.mockReset().mockResolvedValue(["a", "b"]);
    mockCreate.mockReset().mockResolvedValue(undefined);
    mockApply.mockReset().mockResolvedValue(undefined);
    mockDelete.mockReset().mockResolvedValue(undefined);
    mockOpen.mockReset().mockResolvedValue(undefined);
    mockSetToast.mockReset();
  });

  it("激活时拉取列表并渲染卡片头与快照行", async () => {
    const wrapper = mountCard();
    await flushPromises();

    expect(mockList).toHaveBeenCalledOnce();
    expect(wrapper.find(".model-config-card-head h3").text()).toBe("模型快照");
    const rows = wrapper.findAll(".model-snapshot-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].find(".model-snapshot-name").text()).toBe("a");
    expect(rows[1].find(".model-snapshot-name").text()).toBe("b");
    // 卡片头两个按钮均为纯图标
    const add = wrapper.find('button[aria-label="新建模型快照"]');
    expect(add.find("path").attributes("d")).toBe(ICON_PLUS);
    expect(add.text().trim()).toBe("");
    expect(wrapper.find(".model-config-reload-btn").exists()).toBe(true);
  });

  it("未激活时不拉取列表；空列表显示空态", async () => {
    const inactive = mountCard(false);
    await flushPromises();
    expect(mockList).not.toHaveBeenCalled();
    inactive.unmount();

    mockList.mockResolvedValue([]);
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.find(".plugin-empty").text()).toBe("暂无模型快照");
    wrapper.unmount();
  });

  it("还原：调用 apply 并 emit applied", async () => {
    const wrapper = mountCard();
    await flushPromises();
    const restore = wrapper.find('button[aria-label="还原模型快照a"]');
    expect(restore.find("path").attributes("d")).toBe(ICON_RESTORE);

    await restore.trigger("click");
    await flushPromises();
    expect(mockApply).toHaveBeenCalledWith("a");
    expect(wrapper.emitted("applied")).toHaveLength(1);
    wrapper.unmount();
  });

  it("还原失败：不 emit applied", async () => {
    mockApply.mockRejectedValue(new Error("boom"));
    const wrapper = mountCard();
    await flushPromises();
    await wrapper.find('button[aria-label="还原模型快照a"]').trigger("click");
    await flushPromises();
    expect(wrapper.emitted("applied")).toBeUndefined();
    expect(mockSetToast).toHaveBeenCalled();
    wrapper.unmount();
  });

  it("删除与打开：调用对应函数并带确认/图标", async () => {
    const wrapper = mountCard();
    await flushPromises();

    const del = wrapper.find('button[aria-label="删除模型快照b"]');
    expect(del.find("path").attributes("d")).toBe(ICON_DELETE);
    await del.trigger("click");
    await flushPromises();
    expect(mockDelete).toHaveBeenCalledWith("b");
    // 删除后刷新列表
    expect(mockList).toHaveBeenCalledTimes(2);

    const open = wrapper.find('button[aria-label="打开模型快照b"]');
    expect(open.find("path").attributes("d")).toBe(ICON_OPEN);
    await open.trigger("click");
    await flushPromises();
    expect(mockOpen).toHaveBeenCalledWith("b");
    wrapper.unmount();
  });

  it("新建：头按钮打开弹窗，输入名称后创建并刷新列表", async () => {
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(false);

    await wrapper.find('button[aria-label="新建模型快照"]').trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    expect(wrapper.find(".modal-title").text()).toBe("新建模型快照");

    const input = wrapper.find("#model-snapshot-name");
    expect(input.attributes("placeholder")).toBe("如 dev");
    // 空名称时创建按钮禁用
    const confirm = wrapper.find(".model-snapshot-create-btn");
    expect(confirm.text()).toBe("确定");
    expect(confirm.attributes("disabled")).toBeDefined();

    await input.setValue("dev");
    await wrapper.find(".model-snapshot-create-btn").trigger("click");
    await flushPromises();
    expect(mockCreate).toHaveBeenCalledWith("dev");
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(mockList).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });
});
