import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import EmptyState from "../EmptyState.vue";

describe("EmptyState 空状态", () => {
  it("空闲提示输入消息", () => {
    const w = mount(EmptyState, { props: { busy: false } });
    expect(w.text()).toContain("输入消息开始新的会话");
  });

  it("忙碌提示正在处理请求", () => {
    const w = mount(EmptyState, { props: { busy: true } });
    expect(w.text()).toContain("正在处理请求…");
    expect(w.find(".empty-logo.busy").exists()).toBe(true);
  });
});
