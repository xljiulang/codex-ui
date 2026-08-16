import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import PermissionMenu from "../PermissionMenu.vue";
import { store } from "../../composables/useCodex";

describe("PermissionMenu 权限模式菜单", () => {
  beforeEach(() => {
    store.permissionMode = "ask-for-approval";
  });

  it("渲染三种模式并高亮当前模式", () => {
    store.permissionMode = "full-access";
    const w = mount(PermissionMenu);
    expect(w.findAll(".mode-menu-item")).toHaveLength(3);
    expect(w.text()).toContain("请求批准");
    expect(w.text()).toContain("帮我批准");
    expect(w.text()).toContain("完全访问权限");
    expect(w.find(".mode-menu-item.selected").text()).toContain("完全访问权限");
  });

  it("点击切换权限模式并关闭", async () => {
    const w = mount(PermissionMenu);
    await w.findAll(".mode-menu-item")[2].trigger("click");
    expect(store.permissionMode).toBe("full-access");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("切换权限模式并关闭", async () => {
    const w = mount(PermissionMenu);
    await w.findAll(".mode-menu-item")[1].trigger("click");
    expect(store.permissionMode).toBe("help-me-approve");
    expect(w.emitted("close")).toBeTruthy();
  });
});
