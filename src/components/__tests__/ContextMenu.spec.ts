import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import ContextMenu from "../ContextMenu.vue";

const ITEMS = [
  { label: "复制", icon: "M10 0H0v10h10z", action: () => {} },
  {
    label: "删除",
    icon: "M10 0H0v10h10z",
    danger: true,
    action: () => {},
  },
];

function mountMenu() {
  return mount(ContextMenu, { props: { items: ITEMS, x: 0, y: 0 } });
}

describe("ContextMenu 复用弹层菜单项基线类", () => {
  it("菜单项按钮同时使用 .ctx-menu-item 与共享基线 .popup-menu-item", () => {
    const wrapper = mountMenu();
    const btns = wrapper.findAll(".ctx-menu-item");
    expect(btns.length).toBe(ITEMS.length);
    for (const b of btns) {
      expect(b.classes()).toContain("popup-menu-item");
    }
  });

  it("danger 项保留 danger 条件类", () => {
    const wrapper = mountMenu();
    const del = wrapper.findAll(".ctx-menu-item")[1];
    expect(del.classes()).toContain("danger");
  });
});
