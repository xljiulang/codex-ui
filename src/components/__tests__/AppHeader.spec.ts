import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, pickAndOpenNewSession: vi.fn() };
});

import AppHeader from "../AppHeader.vue";
import { tooltipDirective } from "../../directives/tooltip";
import { pickAndOpenNewSession, store } from "../../composables/useCodex";

const mockedPickAndOpenNewSession = vi.mocked(pickAndOpenNewSession);

function mountHeader() {
  return mount(AppHeader, {
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("AppHeader 导航", () => {
  beforeEach(() => {
    store.showSettings = false;
    store.currentThreadId = null;
    store.currentThreadCwd = null;
    store.server.workspace = "";
    store.panelTab = "history";
    mockedPickAndOpenNewSession.mockClear();
  });

  it("点设置打开设置页", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(true);
  });

  it("设置页再点设置关闭设置，回到原对话且不新建/不中断", async () => {
    store.showSettings = true;
    store.currentThreadId = "t1";
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(false);
    expect(store.currentThreadId).toBe("t1");
    expect(mockedPickAndOpenNewSession).not.toHaveBeenCalled();
  });

  it("点新建会话：调用共享新建入口（含目录选择）", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(mockedPickAndOpenNewSession).toHaveBeenCalledTimes(1);
  });

  it("设置按钮无 active 态", async () => {
    store.showSettings = true;
    const wrapper = mountHeader();
    expect(wrapper.find('button[aria-label="设置"]').classes()).not.toContain(
      "active",
    );
  });
});

describe("AppHeader 图标布局", () => {
  it("工作目录按钮已移除，头部只剩新建会话与设置两个图标按钮", () => {
    const wrapper = mountHeader();
    expect(wrapper.find("button.brand-cwd").exists()).toBe(false);
    expect(wrapper.find(".cwd-group").exists()).toBe(false);
    const actions = wrapper.findAll(".header-actions > *");
    expect(actions.map((a) => a.attributes("aria-label"))).toEqual([
      "新建会话",
      "设置",
    ]);
  });
});
