import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import LoadingScreen from "../LoadingScreen.vue";

describe("LoadingScreen 启动加载动画", () => {
  it("渲染 logo、旋转容器与加载文案", () => {
    const w = mount(LoadingScreen);
    expect(w.find(".loading-screen__logo svg").exists()).toBe(true);
    expect(w.find(".loading-screen__logo svg .logo-c").exists()).toBe(true);
    expect(w.find(".loading-screen__text").text()).toBe("正在加载…");
  });

  it("加载文案带 role=status 供读屏播报", () => {
    const w = mount(LoadingScreen);
    expect(w.find('[role="status"]').exists()).toBe(true);
    expect(w.find('[role="status"]').text()).toBe("正在加载…");
  });
});
