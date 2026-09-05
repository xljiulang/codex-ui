import { afterEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import AppSelect from "../AppSelect.vue";

const options = [
  { value: "a", label: "选项 A" },
  { value: "b", label: "选项 B" },
  { value: "c", label: "选项 C" },
];

function menuEl(): Element | null {
  return document.body.querySelector(".app-select-menu");
}

function menuOptions(): HTMLElement[] {
  return Array.from(
    document.body.querySelectorAll<HTMLElement>(".app-select-option"),
  );
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("AppSelect 自绘下拉", () => {
  it("按钮渲染当前选中项 label，初始收起", () => {
    const wrapper = mount(AppSelect, {
      props: { modelValue: "b", options },
    });
    const trigger = wrapper.find("button.app-select");
    expect(trigger.exists()).toBe(true);
    expect(trigger.find(".app-select-label").text()).toBe("选项 B");
    expect(trigger.attributes("aria-expanded")).toBe("false");
    wrapper.unmount();
  });

  it("点击展开弹出层：全部选项渲染、选中项标记、aria 展开", async () => {
    const wrapper = mount(AppSelect, {
      props: { modelValue: "b", options },
      attachTo: document.body,
    });
    await wrapper.find("button.app-select").trigger("click");
    await nextTick();
    const menu = menuEl();
    expect(menu).toBeTruthy();
    expect(triggerAriaExpanded(wrapper)).toBe("true");
    const opts = menuOptions();
    expect(opts.map((o) => o.textContent?.trim())).toEqual([
      "选项 A",
      "选项 B",
      "选项 C",
    ]);
    expect(opts[1].classList.contains("selected")).toBe(true);
    wrapper.unmount();
  });

  it("点击选项回传 update:modelValue 并收起弹层", async () => {
    const wrapper = mount(AppSelect, {
      props: { modelValue: "a", options },
      attachTo: document.body,
    });
    await wrapper.find("button.app-select").trigger("click");
    await nextTick();
    menuOptions()[2].click();
    await nextTick();
    expect(wrapper.emitted("update:modelValue")?.[0]).toEqual(["c"]);
    expect(menuEl()).toBeNull();
    wrapper.unmount();
  });

  it("Escape 关闭弹层", async () => {
    const wrapper = mount(AppSelect, {
      props: { modelValue: "a", options },
      attachTo: document.body,
    });
    await wrapper.find("button.app-select").trigger("click");
    await nextTick();
    expect(menuEl()).toBeTruthy();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(menuEl()).toBeNull();
    wrapper.unmount();
  });

  it("点击弹层外区域关闭", async () => {
    const wrapper = mount(AppSelect, {
      props: { modelValue: "a", options },
      attachTo: document.body,
    });
    await wrapper.find("button.app-select").trigger("click");
    await nextTick();
    expect(menuEl()).toBeTruthy();
    document.body.dispatchEvent(
      new MouseEvent("mousedown", { bubbles: true }),
    );
    await nextTick();
    expect(menuEl()).toBeNull();
    wrapper.unmount();
  });

  it("disabled 时不展开", async () => {
    const wrapper = mount(AppSelect, {
      props: { modelValue: "a", options, disabled: true },
      attachTo: document.body,
    });
    await wrapper.find("button.app-select").trigger("click");
    await nextTick();
    expect(menuEl()).toBeNull();
    wrapper.unmount();
  });

  it("无匹配值时显示 placeholder 占位", () => {
    const wrapper = mount(AppSelect, {
      props: { modelValue: "", options, placeholder: "请选择" },
    });
    const label = wrapper.find(".app-select-label");
    expect(label.text()).toBe("请选择");
    expect(label.classes()).toContain("is-placeholder");
    wrapper.unmount();
  });

  it("键盘 ArrowDown + Enter 选中下一项", async () => {
    const wrapper = mount(AppSelect, {
      props: { modelValue: "a", options },
      attachTo: document.body,
    });
    const trigger = wrapper.find("button.app-select");
    await trigger.trigger("keydown", { key: "ArrowDown" });
    await nextTick();
    expect(menuEl()).toBeTruthy();
    // 初始高亮为选中项 a → ArrowDown 移到 b → Enter 回传
    await trigger.trigger("keydown", { key: "ArrowDown" });
    await trigger.trigger("keydown", { key: "Enter" });
    await nextTick();
    expect(wrapper.emitted("update:modelValue")?.[0]).toEqual(["b"]);
    expect(menuEl()).toBeNull();
    wrapper.unmount();
  });
});

function triggerAriaExpanded(
  wrapper: ReturnType<typeof mount>,
): string | undefined {
  return wrapper.find("button.app-select").attributes("aria-expanded");
}
