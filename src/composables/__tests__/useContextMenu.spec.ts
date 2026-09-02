import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { defineComponent, h, ref } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import { copyText } from "../../lib/clipboard";
import { useContextMenu } from "../useContextMenu";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../lib/links", () => ({
  openLink: vi.fn(),
}));
vi.mock("../../lib/clipboard", () => ({ copyText: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);
const mockedCopy = vi.mocked(copyText);

const Host = defineComponent({
  setup() {
    const { ctxMenu } = useContextMenu();
    const text = ref("hello");
    return () =>
      h("div", [
        h("div", { class: "host-content" }, [
          h("textarea", {
            value: text.value,
            onInput: (e: Event) => {
              text.value = (e.target as HTMLTextAreaElement).value;
            },
          }),
          h("a", { href: "https://example.com" }, "链接"),
        ]),
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

const HostControls = defineComponent({
  setup() {
    const { ctxMenu } = useContextMenu();
    return () =>
      h("div", [
        h("input", { type: "checkbox", id: "cb" }),
        h("select", { id: "sel" }, [h("option", {}, "a")]),
        ctxMenu.value
          ? h(
              "div",
              { class: "ctx-menu" },
              ctxMenu.value.items.map((it) => h("button", {}, it.label)),
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

  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue("");
    mockedCopy.mockReset();
    mockedCopy.mockResolvedValue(true);
  });

  it("文本输入控件右键被拦截并弹出编辑菜单", async () => {
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
    expect(ev.defaultPrevented).toBe(true);
    const menu = wrapper.find(".ctx-menu");
    expect(menu.exists()).toBe(true);
    expect(menu.text()).toContain("粘贴");
    expect(menu.text()).toContain("全选");
    expect(menu.text()).not.toContain("剪切");
    expect(menu.text()).not.toContain("复制");
  });

  it("存在选区时显示剪切/复制，点复制调用 copyText", async () => {
    const wrapper = mountHost();
    await flushPromises();
    const ta = wrapper.find("textarea").element as HTMLTextAreaElement;
    ta.focus();
    ta.setSelectionRange(0, 2);
    wrapper
      .find("textarea")
      .element.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          clientX: 50,
          clientY: 50,
        }),
      );
    await flushPromises();
    const menu = wrapper.find(".ctx-menu");
    expect(menu.text()).toContain("剪切");
    expect(menu.text()).toContain("复制");
    const btns = wrapper.findAll(".ctx-menu-item");
    await btns.find((b) => b.text() === "复制")!.trigger("click");
    expect(mockedCopy).toHaveBeenCalledWith("he");
  });

  it("点粘贴经 clipboard_read_text 读取并插入光标", async () => {
    mockedInvoke.mockResolvedValue("pasted");
    const wrapper = mountHost();
    await flushPromises();
    const ta = wrapper.find("textarea").element as HTMLTextAreaElement;
    ta.setSelectionRange(0, 0);
    wrapper
      .find("textarea")
      .element.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          clientX: 50,
          clientY: 50,
        }),
      );
    await flushPromises();
    const btns = wrapper.findAll(".ctx-menu-item");
    await btns.find((b) => b.text() === "粘贴")!.trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("clipboard_read_text");
    const cur = wrapper.find("textarea").element as HTMLTextAreaElement;
    expect(cur.value).toContain("pasted");
  });

  it("checkbox/select 右键不弹编辑菜单（放行原生）", async () => {
    const wrapper = mount(HostControls, { attachTo: document.body });
    await flushPromises();
    for (const sel of ["#cb", "#sel"]) {
      const el = wrapper.find(sel).element as HTMLElement;
      const ev = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 10,
        clientY: 10,
      });
      el.dispatchEvent(ev);
      await flushPromises();
      expect(ev.defaultPrevented).toBe(false);
      expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    }
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
