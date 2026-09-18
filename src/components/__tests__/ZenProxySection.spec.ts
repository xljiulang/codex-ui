import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockOpenDocsUrl = vi.hoisted(() => vi.fn());
const mockCopyText = vi.hoisted(() => vi.fn());
vi.mock("../../lib/clipboard", () => ({ copyText: mockCopyText }));
vi.mock("../../lib/links", () => ({ openDocsUrl: mockOpenDocsUrl }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    applyZenProxy: vi.fn(async () => ({ running: false, port: 18080 })),
    readZenProxyStatus: vi.fn(), // 会被组件覆盖，见测试
    toggleZenProxy: vi.fn(async () => true),
    setToast: vi.fn(),
    toastError: vi.fn(),
  };
});

import ZenProxySection from "../settings/ZenProxySection.vue";
import { tooltipDirective } from "../../directives/tooltip";
import {
  applyZenProxy,
  readZenProxyStatus,
  setToast,
  toggleZenProxy,
} from "../../composables/useCodex";

const mockedApply = vi.mocked(applyZenProxy);
const mockedRead = vi.mocked(readZenProxyStatus);
const mockedToggle = vi.mocked(toggleZenProxy);
const mockedToast = vi.mocked(setToast);
const ZEN_PRICING_DOCS_URL =
  "https://open-code.ai/zh/docs/zen#%E5%AE%9A%E4%BB%B7";

function mountSection() {
  return mount(ZenProxySection, {
    props: { active: true },
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("ZenProxySection", () => {
  beforeEach(() => {
    mockedRead.mockResolvedValue({ running: false, port: 18080 });
    mockedApply.mockClear();
    mockedToggle.mockClear();
    mockedToast.mockClear();
    mockOpenDocsUrl.mockClear();
    mockCopyText.mockReset();
    mockCopyText.mockResolvedValue(true);
  });

  it("渲染标题、说明与运行状态", async () => {
    mockedRead.mockResolvedValue({ running: true, port: 18080 });
    const wrapper = mountSection();
    await flushPromises();
    expect(wrapper.find(".settings-section-title").text()).toContain("Zen 代理");
    expect(wrapper.find(".zen-proxy-head-main h3").text()).toBe(
      "Zen 本地代理服务",
    );
    // 说明文案含 Responses 与 Zen
    expect(wrapper.text()).toContain("Responses");
    expect(wrapper.text()).toContain("OpenCode Zen");
  });

  it("配置块中的 Zen 免费模型链接到定价文档", async () => {
    const wrapper = mountSection();
    const row = wrapper.findAll(".zen-proxy-config-row")[2];
    const link = row.find(".zen-proxy-docs-link");
    expect(link.text()).toBe("Zen 免费模型");
    expect(link.attributes("href")).toBe(ZEN_PRICING_DOCS_URL);
    await link.trigger("click");
    expect(mockOpenDocsUrl).toHaveBeenCalledWith(ZEN_PRICING_DOCS_URL);
  });

  it("配置块渲染 experimental_bearer_token：指向「API Key」链接", async () => {
    const wrapper = mountSection();
    const rows = wrapper.findAll(".zen-proxy-config-row");
    const tokenRow = rows[1];
    expect(tokenRow.find(".zen-proxy-config-field").text()).toBe(
      "experimental_bearer_token",
    );
    const keyLink = tokenRow.find(".zen-proxy-docs-link");
    expect(keyLink.text()).toBe("ApiKey");
    expect(tokenRow.text()).toContain("public");
    const publicCode = tokenRow.find(".zen-proxy-config-code");
    expect(publicCode.exists()).toBe(true);
    expect(publicCode.text()).toBe("public");
  });

  it("配置块 base_url 行提供复制按钮并调用 copyText", async () => {
    const wrapper = mountSection();
    const copyBtn = wrapper.find(".zen-proxy-config-copy");
    expect(copyBtn.exists()).toBe(true);
    await copyBtn.trigger("click");
    expect(mockCopyText).toHaveBeenCalledWith(
      "http://127.0.0.1:18080/zen/v1",
    );
    expect(copyBtn.text()).toContain("已复制");
  });

  it("本地 provider 的 base_url 提示跟随上游地址的路径", async () => {
    const wrapper = mountSection();
    // 默认上游 https://opencode.ai/zen/v1 → 本机地址 + /zen/v1
    const value = () =>
      wrapper.find(".zen-proxy-config-value.zen-proxy-config-code");
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

  it("端口非法时提示错误", async () => {
    const wrapper = mountSection();
    const input = wrapper.find("input[type='number']");
    await input.setValue("80");
    const btn = wrapper.findAll("button").find((b) => b.text().includes("保存"));
    await btn!.trigger("click");
    expect(wrapper.find(".zen-proxy-error").exists()).toBe(true);
    expect(mockedToggle).not.toHaveBeenCalled();
    expect(mockedApply).not.toHaveBeenCalled();
  });

  it("开启开关时调用 toggleZenProxy", async () => {
    const wrapper = mountSection();
    const checkbox = wrapper.find("input[type='checkbox']");
    await checkbox.setValue(true);
    await checkbox.trigger("change");
    // 默认端口 18080 通过校验，默认 base_url
    expect(mockedToggle).toHaveBeenCalledWith(
      true,
      18080,
      "https://opencode.ai/zen/v1",
    );
  });

  it("保存合法端口与 API 地址调用 applyZenProxy", async () => {
    const wrapper = mountSection();
    const input = wrapper.find("input[type='number']");
    await input.setValue("19090");
    const urlInput = wrapper.find("input[type='text']");
    await urlInput.setValue("https://custom.example.com/v1");
    const btn = wrapper.findAll("button").find((b) => b.text().includes("保存"));
    await btn!.trigger("click");
    expect(mockedApply).toHaveBeenCalledWith(
      false,
      19090,
      "https://custom.example.com/v1",
      true,
    );
  });
});
