import { describe, expect, it, vi, afterEach } from "vitest";
import { defineComponent, h, ref } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import { useContextMenu } from "../useContextMenu";

vi.mock("../../lib/links", () => ({
  openLink: vi.fn(),
}));

const Host = defineComponent({
  setup() {
    const { ctxMenu } = useContextMenu();
    const text = ref("hello");
    return () =>
      h("div", [
        h("textarea", { value: text.value }),
        h("a", { href: "https://example.com" }, "链接"),
        ctxMenu.value
          ? h(
              "div",
              {
                class: "ctx-menu",
                style: {
                  left: ctxMenu.value.x + "px",
                  top: ctxMenu.value.y + "px",
                },
              },
              ctxMenu.value.items.map((it) =>
                h(
                  "button",
                  { class: "ctx-menu-item", onClick: () => it.action() },
                  it.label,
                ),
              ),
            )
          : null,
      ]);
  },
});

const HostAlwaysCopy = defineComponent({
  setup() {
    const { ctxMenu } = useContextMenu(true, true);
    return () =>
      h("div", [
        h("span", { id: "plain" }, "普通文本行"),
        ctxMenu.value
          ? h(
              "div",
              { class: "ctx-menu" },
              ctxMenu.value.items.map((it) =>
                h("button", { class: "ctx-menu-item" }, it.label),
              ),
            )
          : null,
      ]);
  },
});

function mountHost() {
  return mount(Host, { attachTo: document.body });
}

describe("useContextMenu 自定义右键菜单", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("可编辑元素右键不拦截（交给原生菜单处理粘贴）", async () => {
    const wrapper = mountHost();
    await flushPromises();
    const ev = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 50,
      clientY: 50,
    });
    wrapper.find("textarea").element.dispatchEvent(ev);
    await flushPromises();
    expect(ev.defaultPrevented).toBe(false);
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
  });

  it("链接右键显示打开链接与复制链接地址", async () => {
    const wrapper = mountHost();
    await flushPromises();
    const ev = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 50,
      clientY: 50,
    });
    wrapper.find("a").element.dispatchEvent(ev);
    await flushPromises();
    expect(ev.defaultPrevented).toBe(true);
    const menu = wrapper.find(".ctx-menu");
    expect(menu.exists()).toBe(true);
    expect(menu.text()).toContain("打开链接");
    expect(menu.text()).toContain("复制链接地址");
  });

  it("点击外部关闭菜单", async () => {
    const wrapper = mountHost();
    await flushPromises();
    wrapper
      .find("a")
      .element.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          clientX: 50,
          clientY: 50,
        }),
      );
    await flushPromises();
    expect(wrapper.find(".ctx-menu").exists()).toBe(true);
    window.dispatchEvent(new MouseEvent("click"));
    await flushPromises();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
  });

  it("alwaysCopy 时非编辑区右键也提供复制", async () => {
    const wrapper = mount(HostAlwaysCopy, { attachTo: document.body });
    await flushPromises();
    const ev = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 50,
      clientY: 50,
    });
    wrapper.find("#plain").element.dispatchEvent(ev);
    await flushPromises();
    expect(ev.defaultPrevented).toBe(true);
    const menu = wrapper.find(".ctx-menu");
    expect(menu.exists()).toBe(true);
    expect(menu.text()).toContain("复制");
  });
});
