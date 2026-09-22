import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockOpenDocsUrl = vi.hoisted(() => vi.fn());
vi.mock("../../lib/links", () => ({ openDocsUrl: mockOpenDocsUrl }));
const mockCopyText = vi.hoisted(() => vi.fn(async () => true));
vi.mock("../../lib/clipboard", () => ({ copyText: mockCopyText }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    applyCompatProxy: vi.fn(async () => ({ running: false, port: 18080 })),
    readCompatProxyStatus: vi.fn(), // 会被组件覆盖，见测试
    toggleCompatProxy: vi.fn(async () => true),
    setToast: vi.fn(),
    toastError: vi.fn(),
  };
});

import CompatProxySection from "../settings/CompatProxySection.vue";
import { tooltipDirective } from "../../directives/tooltip";
import { store } from "../../composables/useCodex";
import {
  applyCompatProxy,
  readCompatProxyStatus,
  setToast,
  toggleCompatProxy,
} from "../../composables/useCodex";

const mockedApply = vi.mocked(applyCompatProxy);
const mockedRead = vi.mocked(readCompatProxyStatus);
const mockedToggle = vi.mocked(toggleCompatProxy);
const mockedToast = vi.mocked(setToast);
const ZEN_DOCS_URL = "https://opencode.ai/zen";

function mountSection() {
  return mount(CompatProxySection, {
    props: { active: true },
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("CompatProxySection", () => {
  beforeEach(() => {
    mockedRead.mockResolvedValue({ running: false, port: 18080 });
    mockedApply.mockClear();
    mockedToggle.mockClear();
    mockedToast.mockClear();
    mockOpenDocsUrl.mockClear();
    mockCopyText.mockClear();
    mockCopyText.mockResolvedValue(true);
    // 用例间会互相污染：显式把代理开关复位为「未启用」
    store.settings.compat_proxy_enabled = false;
  });

  it("渲染标题、说明与运行状态", async () => {
    mockedRead.mockResolvedValue({ running: true, port: 18080 });
    const wrapper = mountSection();
    await flushPromises();
    expect(wrapper.find(".settings-section-title").text()).toContain("兼容代理");
    expect(wrapper.find(".compat-proxy-head-main h3").text()).toBe(
      "兼容代理服务",
    );
    // 说明文案含 Responses，并点出默认上游是 OpenCode Zen（可换成任意兼容端点）
    expect(wrapper.text()).toContain("Responses");
    expect(wrapper.text()).toContain("OpenCode Zen");
    expect(wrapper.text()).toContain("OpenAI 兼容上游");
  });

  it("顶部说明里的「Zen 免费模型」是文档链接", async () => {
    const wrapper = mountSection();
    const link = wrapper.find(".settings-section-desc .compat-proxy-docs-link");
    expect(link.text()).toBe("Zen 免费模型");
    expect(link.attributes("href")).toBe(ZEN_DOCS_URL);
    await link.trigger("click");
    expect(mockOpenDocsUrl).toHaveBeenCalledWith(ZEN_DOCS_URL);
  });

  it("接入配置框整块删除（含旧字段提示）", () => {
    const wrapper = mountSection();
    expect(wrapper.find(".compat-proxy-config-hint").exists()).toBe(false);
    expect(wrapper.find(".compat-proxy-config-row").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("字段按下图填写");
    expect(wrapper.text()).not.toContain("experimental_bearer_token");
  });

  it("状态徽章常驻：未启用时显示「已停止」且无胶囊", () => {
    const wrapper = mountSection();
    const badge = wrapper.find(".compat-proxy-status-badge");
    expect(badge.text()).toBe("已停止");
    expect(badge.classes()).toContain("is-stopped");
    expect(wrapper.find(".compat-proxy-base-url-capsule").exists()).toBe(false);
  });

  it("启用后显示「已启动」徽章与 base_url 胶囊", () => {
    store.settings.compat_proxy_enabled = true;
    const wrapper = mountSection();
    const badge = wrapper.find(".compat-proxy-status-badge");
    expect(badge.text()).toBe("已启动");
    expect(badge.classes()).toContain("is-running");
    const capsule = wrapper.find(".compat-proxy-base-url-capsule");
    expect(capsule.exists()).toBe(true);
    expect(capsule.text()).toContain("http://127.0.0.1:18080/zen/v1");
  });

  it("点击胶囊复制 base_url 并提示", async () => {
    store.settings.compat_proxy_enabled = true;
    const wrapper = mountSection();
    await wrapper.find(".compat-proxy-base-url-capsule").trigger("click");
    expect(mockCopyText).toHaveBeenCalledWith("http://127.0.0.1:18080/zen/v1");
    expect(mockedToast).toHaveBeenCalledWith("已复制 base_url");
  });

  it("复制失败时提示手动复制", async () => {
    store.settings.compat_proxy_enabled = true;
    mockCopyText.mockResolvedValue(false);
    const wrapper = mountSection();
    await wrapper.find(".compat-proxy-base-url-capsule").trigger("click");
    expect(mockedToast).toHaveBeenCalledWith("复制失败，请手动选择复制");
  });

  it("本地 provider 的 base_url 提示跟随上游地址的路径", async () => {
    store.settings.compat_proxy_enabled = true;
    const wrapper = mountSection();
    // 默认上游 https://opencode.ai/zen/v1 → 本机地址 + /zen/v1
    const value = () => wrapper.find(".compat-proxy-base-url-capsule");
    expect(value().text()).toContain("http://127.0.0.1:18080/zen/v1");
    const urlInput = wrapper.find("input[type='text']");
    await urlInput.setValue("https://api.deepseek.com/");
    expect(value().text()).toContain("http://127.0.0.1:18080");
    expect(value().text()).not.toContain("http://127.0.0.1:18080/zen");
    await urlInput.setValue("https://custom.example.com/zen/v2/");
    expect(value().text()).toContain("http://127.0.0.1:18080/zen/v2");
  });

  it("上游地址字段文案为「模型提供方的 base_url」", () => {
    const wrapper = mountSection();
    const labels = wrapper.findAll("label").map((l) => l.text());
    expect(labels).toContain("模型提供方的 base_url");
    expect(labels).not.toContain("转发目标地址");
  });

  it("两个行为开关的文案为「名称（额外说明）」且默认勾选", () => {
    const wrapper = mountSection();
    const texts = wrapper
      .findAll(".compat-proxy-checkbox-row label")
      .map((l) => l.text());
    expect(texts[0]).toContain("回合收尾约束和助推");
    expect(texts[0]).toContain("模型空转收尾时自动续跑");
    expect(texts[1]).toContain("OpenCode 客户端身份");
    expect(texts[1]).toContain("补齐与 OpenCode 一致的请求头和工具集");
    // 身份开关不再按上游分流：括号说明里不该再出现「对 opencode 上游」这类前提
    expect(texts[1]).not.toContain("opencode 上游");
    const boxes = wrapper.findAll(".compat-proxy-checkbox-row input");
    expect(boxes).toHaveLength(2);
    for (const box of boxes) {
      expect((box.element as HTMLInputElement).checked).toBe(true);
    }
  });

  it("两个行为开关用的是左侧复选框，不再是右侧 switch", () => {
    const wrapper = mountSection();
    const rows = wrapper.findAll(".compat-proxy-checkbox-row");
    expect(rows).toHaveLength(2);
    for (const row of rows) {
      const input = row.find("input[type='checkbox']");
      expect(input.exists()).toBe(true);
      // 复选框在左：它是行内第一个子节点，label 紧随其后
      expect(row.element.firstElementChild).toBe(input.element);
      expect(input.element.nextElementSibling?.tagName).toBe("LABEL");
      // 行内不再有 switch 药丸
      expect(row.find(".switch").exists()).toBe(false);
      expect(row.find(".switch-track").exists()).toBe(false);
    }
  });

  it("保存时把两个行为开关当前值一并提交", async () => {
    const wrapper = mountSection();
    const boxes = wrapper.findAll(".compat-proxy-checkbox-row input");
    await boxes[0].setValue(false);
    await boxes[1].setValue(false);
    const btn = wrapper.findAll("button").find((b) => b.text().includes("保存"));
    await btn!.trigger("click");
    expect(mockedApply).toHaveBeenCalledWith(
      {
        enabled: false,
        port: 18080,
        baseUrl: "https://opencode.ai/zen/v1",
        nudgeEnabled: false,
        identityEnabled: false,
      },
      true,
    );
  });

  it("端口非法时提示错误", async () => {
    const wrapper = mountSection();
    const input = wrapper.find("input[type='number']");
    await input.setValue("80");
    const btn = wrapper.findAll("button").find((b) => b.text().includes("保存"));
    await btn!.trigger("click");
    expect(wrapper.find(".compat-proxy-error").exists()).toBe(true);
    expect(mockedToggle).not.toHaveBeenCalled();
    expect(mockedApply).not.toHaveBeenCalled();
  });

  it("开启开关时调用 toggleCompatProxy", async () => {
    const wrapper = mountSection();
    const checkbox = wrapper.find(
      ".model-config-head-actions input[type='checkbox']",
    );
    await checkbox.setValue(true);
    await checkbox.trigger("change");
    // 默认端口 18080 通过校验，默认 base_url
    expect(mockedToggle).toHaveBeenCalledWith(
      {
        enabled: true,
        port: 18080,
        baseUrl: "https://opencode.ai/zen/v1",
        nudgeEnabled: true,
        identityEnabled: true,
      },
    );
  });

  it("保存合法端口与 API 地址调用 applyCompatProxy", async () => {
    const wrapper = mountSection();
    const input = wrapper.find("input[type='number']");
    await input.setValue("19090");
    const urlInput = wrapper.find("input[type='text']");
    await urlInput.setValue("https://custom.example.com/v1");
    const btn = wrapper.findAll("button").find((b) => b.text().includes("保存"));
    await btn!.trigger("click");
    expect(mockedApply).toHaveBeenCalledWith(
      {
        enabled: false,
        port: 19090,
        baseUrl: "https://custom.example.com/v1",
        nudgeEnabled: true,
        identityEnabled: true,
      },
      true,
    );
  });
});
