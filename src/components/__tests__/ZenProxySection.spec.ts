import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
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

describe("ZenProxySection", () => {
  beforeEach(() => {
    mockedRead.mockResolvedValue({ running: false, port: 18080 });
    mockedApply.mockClear();
    mockedToggle.mockClear();
    mockedToast.mockClear();
  });

  it("渲染标题、说明与运行状态", async () => {
    mockedRead.mockResolvedValue({ running: true, port: 18080 });
    const wrapper = mount(ZenProxySection, { props: { active: true } });
    await flushPromises();
    expect(wrapper.find(".settings-section-title").text()).toContain("Zen 本地代理");
    expect(wrapper.text()).toContain("运行中");
    expect(wrapper.find(".zen-proxy-port-badge").text()).toContain("18080");
    // 说明文案含 Responses 与 Zen
    expect(wrapper.text()).toContain("Responses");
    expect(wrapper.text()).toContain("OpenCode Zen");
  });

  it("端口非法时提示错误", async () => {
    const wrapper = mount(ZenProxySection, { props: { active: true } });
    const input = wrapper.find("input[type='number']");
    await input.setValue("80");
    const btn = wrapper.findAll("button").find((b) => b.text().includes("应用"));
    await btn!.trigger("click");
    expect(wrapper.find(".zen-proxy-error").exists()).toBe(true);
    expect(mockedToggle).not.toHaveBeenCalled();
    expect(mockedApply).not.toHaveBeenCalled();
  });

  it("开启开关时调用 toggleZenProxy", async () => {
    const wrapper = mount(ZenProxySection, { props: { active: true } });
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

  it("应用合法端口与 API 地址调用 applyZenProxy", async () => {
    const wrapper = mount(ZenProxySection, { props: { active: true } });
    const input = wrapper.find("input[type='number']");
    await input.setValue("19090");
    const urlInput = wrapper.find("input[type='text']");
    await urlInput.setValue("https://custom.example.com/v1");
    const btn = wrapper.findAll("button").find((b) => b.text().includes("应用"));
    await btn!.trigger("click");
    expect(mockedApply).toHaveBeenCalledWith(
      false,
      19090,
      "https://custom.example.com/v1",
      true,
    );
  });
});
