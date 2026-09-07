import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockSetToast = vi.hoisted(() => vi.fn());
const mockToastError = vi.hoisted(() => (e: unknown) => String(e));
const mockStore = vi.hoisted(() => ({
  settings: { active_config: null as string | null },
}));

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

vi.mock("../../composables/useConfigProfiles", () => ({
  listConfigProfiles: mockList,
  createConfigProfile: mockCreate,
  applyConfigProfile: mockApply,
  deleteConfigProfile: mockDelete,
}));

import ConfigSwitchMenu from "../ConfigSwitchMenu.vue";
import { tooltipDirective } from "../../directives/tooltip";
import { ICON_CLOSE } from "../../lib/icons";

function mountMenu() {
  return mount(ConfigSwitchMenu, {
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("ConfigSwitchMenu", () => {
  beforeEach(() => {
    mockList.mockReset().mockResolvedValue(["a", "b"]);
    mockCreate.mockReset();
    mockApply.mockReset();
    mockDelete.mockReset();
    mockSetToast.mockReset();
    mockStore.settings.active_config = null;
  });

  it("点配置快照按钮展开菜单并拉取配置列表", async () => {
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="配置快照"]').trigger("click");
    await flushPromises();
    expect(mockList).toHaveBeenCalledOnce();
    expect(wrapper.find(".config-menu-title").text()).toContain("配置快照");
    expect(wrapper.text()).toContain("a");
    expect(wrapper.text()).toContain("b");
  });

  it("触发按钮为纯图标（内联单色 {} 花括号，无文字）", async () => {
    const wrapper = mountMenu();
    const btn = wrapper.find('button[aria-label="配置快照"]');
    // 不再使用彩色 CSS 文件类型 <img>，改为内联单色 <svg>
    expect(btn.find("img").exists()).toBe(false);
    const brace = btn.find(".config-brace");
    expect(brace.exists()).toBe(true);
    expect(brace.findAll("path").length).toBe(2);
    expect(brace.findAll("path").every((p) => p.attributes("fill") === "none")).toBe(
      true,
    );
    expect(btn.text().trim()).toBe("");
  });

  it("点 ✓ 应用配置", async () => {
    mockStore.settings.active_config = "a";
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="配置快照"]').trigger("click");
    await flushPromises();
    await wrapper.find('button[aria-label="应用配置a"]').trigger("click");
    await flushPromises();
    expect(mockApply).toHaveBeenCalledWith("a");
    expect(mockSetToast).not.toHaveBeenCalledWith(
      expect.stringContaining("已删除"),
    );
  });

  it("点 × 删除配置（复用 git 分支删除图标 ICON_CLOSE）", async () => {
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="配置快照"]').trigger("click");
    await flushPromises();
    const del = wrapper.find('button[aria-label="删除配置b"]');
    expect(del.find("path").attributes("d")).toBe(ICON_CLOSE);
    // 同一行内 ✓ 应用按钮需在 × 删除按钮之前（name → ✓ → ×）
    const row = del.element.closest(".git-branch-menu-item") as HTMLElement;
    const apply = row.querySelector('button[aria-label="应用配置b"]');
    expect(apply).toBeTruthy();
    expect(
      Array.prototype.indexOf.call(
        row.children,
        apply,
      ),
    ).toBeLessThan(
      Array.prototype.indexOf.call(row.children, del.element),
    );
    await del.trigger("click");
    await flushPromises();
    expect(mockDelete).toHaveBeenCalledWith("b");
  });

  it("底部新建：输入名字并点新建创建配置", async () => {
    const wrapper = mountMenu();
    await wrapper.find('button[aria-label="配置快照"]').trigger("click");
    await flushPromises();
    const input = wrapper.find(".git-branch-input");
    expect(input.attributes("placeholder")).toBe("新建配置快照");
    await input.setValue("dev");
    await wrapper.find(".git-branch-create-btn").trigger("click");
    await flushPromises();
    expect(mockCreate).toHaveBeenCalledWith("dev");
    // 新建成功后菜单保持打开，便于直接看到新快照被勾选
    expect(wrapper.find(".config-profiles-menu").exists()).toBe(true);
  });
});
