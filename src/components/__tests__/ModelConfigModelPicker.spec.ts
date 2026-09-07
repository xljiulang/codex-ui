import { afterEach, describe, expect, it } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import ModelConfigModelPicker from "../ModelConfigModelPicker.vue";

let wrapper: VueWrapper | null = null;

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
});

const OPTIONS = ["gpt-5.2-codex", "deepseek-v4-flash", "claude-3-7"];

function mountPicker(props: Record<string, unknown> = {}) {
  wrapper = mount(ModelConfigModelPicker, {
    props: {
      modelValue: "",
      options: OPTIONS,
      ...props,
    },
  });
  return wrapper;
}

describe("ModelConfigModelPicker 模型候选下拉", () => {
  it("输入框禁用浏览器自动填充（规避「保存的信息」下拉）", () => {
    const w = mountPicker();
    expect(w.find("input").attributes("autocomplete")).toBe("off");
  });

  it("点击输入框才展开，列表包含全部候选", async () => {
    const w = mountPicker();
    expect(w.find(".popup-menu").exists()).toBe(false);
    await w.find("input").trigger("click");
    expect(w.find(".popup-menu").exists()).toBe(true);
    const btns = w.findAll(".option-btn");
    expect(btns.length).toBe(OPTIONS.length);
    for (const opt of OPTIONS) {
      expect(w.text()).toContain(opt);
    }
  });

  it("输入框已有值或键入部分字符时，列表仍包含全部候选（不过滤）", async () => {
    const w = mountPicker({ modelValue: "deepseek-v4-flash" });
    const input = w.find("input");
    await input.setValue("d");
    await input.trigger("click");
    const btns = w.findAll(".option-btn");
    expect(btns.length).toBe(OPTIONS.length);
  });

  it("点击候选后 emit update:modelValue 为其值并收起", async () => {
    const w = mountPicker();
    await w.find("input").trigger("click");
    const btns = w.findAll(".option-btn");
    await btns[1].trigger("click");
    const emitted = w.emitted("update:modelValue");
    expect(emitted).toBeTruthy();
    expect(emitted![0]).toEqual([OPTIONS[1]]);
    expect(w.find(".popup-menu").exists()).toBe(false);
  });

  it("候选为空时显示无可选模型", async () => {
    const w = mountPicker({ options: [] });
    await w.find("input").trigger("click");
    expect(w.find(".popup-menu").exists()).toBe(true);
    expect(w.text()).toContain("无可选模型");
  });

  it("disabled 时点击输入框不展开", async () => {
    const w = mountPicker({ disabled: true });
    await w.find("input").trigger("click");
    expect(w.find(".popup-menu").exists()).toBe(false);
  });
});
