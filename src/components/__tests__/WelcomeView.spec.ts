import { describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import WelcomeView from "../WelcomeView.vue";

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(async () => "1.2.3"),
}));

function mountView() {
  return mount(WelcomeView);
}

describe("WelcomeView 欢迎介绍页", () => {
  it("品牌区展示欢迎语、使用指引与应用版本", async () => {
    const wrapper = mountView();
    await flushPromises();
    expect(wrapper.find(".welcome-title").text()).toContain("欢迎使用 Codex-UI");
    expect(wrapper.find(".welcome-tagline").text()).toContain("Codex CLI");
    expect(wrapper.find(".welcome-guide").text()).toContain("新建会话");
    expect(wrapper.find(".welcome-version").text()).toBe("v1.2.3");
    wrapper.unmount();
  });

  it("主功能卡片覆盖微信远控/资源/Git/日志防膨胀/插件市场/多会话/终端", () => {
    const wrapper = mountView();
    const titles = wrapper.findAll(".feature-card h3").map((h) => h.text());
    expect(titles.join("|")).toContain("微信远控");
    expect(titles.join("|")).toContain("对话内动态工具");
    expect(titles.join("|")).toContain("资源管理");
    expect(titles.join("|")).toContain("Git");
    expect(titles.join("|")).toContain("日志防膨胀");
    expect(titles.join("|")).toContain("内置双插件市场");
    expect(titles.join("|")).toContain("多会话并行标签");
    expect(titles.join("|")).toContain("终端");
    const wechatCard = wrapper
      .findAll(".feature-card")
      .find((c) => c.text().includes("微信远控"))!;
    expect(wechatCard.exists()).toBe(true);
    expect(wechatCard.find(".feature-badge").text()).toContain("实验性");
    expect(wechatCard.text()).toContain("微信接入");
    expect(wechatCard.text()).toContain("仅限工作目录的完全访问");
    expect(wechatCard.classes()).toEqual(["feature-card"]);
    wrapper.unmount();
  });

  it("对话内动态工具卡片：说明两个 codexui 工具及其微信远控必要性", () => {
    const wrapper = mountView();
    const cards = wrapper.findAll(".feature-card");
    const toolCard = cards.find((c) => c.text().includes("对话内动态工具"))!;
    expect(toolCard.exists()).toBe(true);
    expect(toolCard.find(".feature-badge").text()).toContain("实验性");
    const text = toolCard.text();
    expect(text).toContain("codexui_get_usage");
    expect(text).toContain("codexui_compact_context");
    expect(text).toContain("上下文窗口占用");
    expect(text).toContain("微信");
    expect(text).toContain("新建会话");
    expect(text).toContain("历史/分叉线程无");
    wrapper.unmount();
  });

  it("补充亮点与页脚说明存在", () => {
    const wrapper = mountView();
    const highlights = wrapper.findAll(".highlight-item").map((h) => h.text());
    expect(highlights.length).toBeGreaterThanOrEqual(8);
    expect(highlights.join("|")).not.toContain("系统托盘常驻");
    expect(highlights.join("|")).toContain("权限");
    expect(wrapper.find(".welcome-foot").text()).toContain("0.149.x");
    wrapper.unmount();
  });
});
