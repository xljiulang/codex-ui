import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockCopyText = vi.hoisted(() => vi.fn(async () => true));
vi.mock("../../lib/clipboard", () => ({ copyText: mockCopyText }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod =
    await importOriginal<typeof import("../../composables/useCodex")>();
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

function mountSection() {
  return mount(CompatProxySection, { props: { active: true } });
}

describe("CompatProxySection", () => {
  beforeEach(() => {
    mockedRead.mockResolvedValue({ running: false, port: 18080 });
    mockedApply.mockClear();
    mockedToggle.mockClear();
    mockedToast.mockClear();
    mockCopyText.mockClear();
    mockCopyText.mockResolvedValue(true);
    // 用例间会互相污染：显式把代理开关复位为「未启用」
    store.settings.compat_proxy_enabled = false;
    store.settings.compat_proxy_base_url = "";
  });

  it("渲染标题、说明与运行状态", async () => {
    mockedRead.mockResolvedValue({ running: true, port: 18080 });
    const wrapper = mountSection();
    await flushPromises();
    expect(wrapper.find(".settings-section-title").text()).toContain(
      "兼容代理",
    );
    expect(wrapper.find(".compat-proxy-head-main h3").text()).toBe(
      "兼容代理服务",
    );
    expect(wrapper.text()).toContain("Responses");
    expect(wrapper.text()).toContain("OpenAI 兼容上游");
    expect(wrapper.text()).toContain("通用协议转换");
  });

  it("不再展示客户端身份开关或旧文档链接", () => {
    const wrapper = mountSection();
    expect(wrapper.find(".compat-proxy-docs-link").exists()).toBe(false);
    expect(wrapper.find("#compat-proxy-identity").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("补齐 2.2.0 请求头");
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
    // 统一胶囊徽章：基类 + 语义档（已停止属正常态 → 中性）
    expect(badge.classes()).toContain("badge");
    expect(badge.classes()).toContain("badge-neutral");
    expect(wrapper.find(".compat-proxy-base-url-capsule").exists()).toBe(false);
  });

  it("启用后显示「已启动」徽章与 base_url 胶囊", () => {
    store.settings.compat_proxy_enabled = true;
    store.settings.compat_proxy_base_url = "https://api.example.com/v1";
    const wrapper = mountSection();
    const badge = wrapper.find(".compat-proxy-status-badge");
    expect(badge.text()).toBe("已启动");
    expect(badge.classes()).toContain("badge");
    expect(badge.classes()).toContain("badge-accent");
    const capsule = wrapper.find(".compat-proxy-base-url-capsule");
    expect(capsule.exists()).toBe(true);
    expect(capsule.text()).toContain("http://127.0.0.1:18080/v1");
  });

  it("点击胶囊复制 base_url 并提示", async () => {
    store.settings.compat_proxy_enabled = true;
    store.settings.compat_proxy_base_url = "https://api.example.com/v1";
    const wrapper = mountSection();
    await wrapper.find(".compat-proxy-base-url-capsule").trigger("click");
    expect(mockCopyText).toHaveBeenCalledWith("http://127.0.0.1:18080/v1");
    expect(mockedToast).toHaveBeenCalledWith("已复制 base_url");
  });

  it("复制失败时提示手动复制", async () => {
    store.settings.compat_proxy_enabled = true;
    store.settings.compat_proxy_base_url = "https://api.example.com/v1";
    mockCopyText.mockResolvedValue(false);
    const wrapper = mountSection();
    await wrapper.find(".compat-proxy-base-url-capsule").trigger("click");
    expect(mockedToast).toHaveBeenCalledWith("复制失败，请手动选择复制");
  });

  it("本地 provider 的 base_url 提示跟随上游地址的路径", async () => {
    store.settings.compat_proxy_enabled = true;
    const wrapper = mountSection();
    const value = () => wrapper.find(".compat-proxy-base-url-capsule");
    expect(value().exists()).toBe(false);
    const urlInput = wrapper.find(".compat-proxy-url-input");
    await urlInput.setValue("https://api.example.com/v1");
    expect(value().text()).toContain("http://127.0.0.1:18080/v1");
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

  it("收尾助推是唯一行为开关且默认勾选", () => {
    const wrapper = mountSection();
    const texts = wrapper
      .findAll(".compat-proxy-checkbox-row label")
      .map((l) => l.text());
    expect(texts).toHaveLength(1);
    expect(texts[0]).toContain("回合收尾约束和助推");
    expect(texts[0]).toContain("模型空转收尾时自动续跑");
    const boxes = wrapper.findAll(".compat-proxy-checkbox-row input");
    expect(boxes).toHaveLength(1);
    for (const box of boxes) {
      expect((box.element as HTMLInputElement).checked).toBe(true);
    }
  });

  it("行为开关使用左侧复选框", () => {
    const wrapper = mountSection();
    const rows = wrapper.findAll(".compat-proxy-checkbox-row");
    expect(rows).toHaveLength(1);
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

  it("保存时提交地址与收尾助推值", async () => {
    const wrapper = mountSection();
    await wrapper
      .find(".compat-proxy-url-input")
      .setValue("https://custom.example.com/v1");
    const boxes = wrapper.findAll(".compat-proxy-checkbox-row input");
    await boxes[0].setValue(false);
    const btn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("保存"));
    await btn!.trigger("click");
    expect(mockedApply).toHaveBeenCalledWith(
      {
        enabled: false,
        port: 18080,
        baseUrl: "https://custom.example.com/v1",
        nudgeEnabled: false,
      },
      true,
    );
  });

  it("端口非法时提示错误", async () => {
    const wrapper = mountSection();
    const input = wrapper.find("input[type='number']");
    await input.setValue("80");
    const btn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("保存"));
    await btn!.trigger("click");
    expect(wrapper.find(".compat-proxy-error").exists()).toBe(true);
    expect(mockedToggle).not.toHaveBeenCalled();
    expect(mockedApply).not.toHaveBeenCalled();
  });

  it("地址为空时拒绝开启代理", async () => {
    const wrapper = mountSection();
    const checkbox = wrapper.find(
      ".model-config-head-actions input[type='checkbox']",
    );
    await checkbox.setValue(true);
    await checkbox.trigger("change");
    expect(mockedToggle).not.toHaveBeenCalled();
    expect(mockedToast).toHaveBeenCalledWith("请先填写模型提供方的 base_url");
  });

  it("填写地址后开启开关调用 toggleCompatProxy", async () => {
    const wrapper = mountSection();
    await wrapper
      .find(".compat-proxy-url-input")
      .setValue("https://api.example.com/v1");
    const checkbox = wrapper.find(
      ".model-config-head-actions input[type='checkbox']",
    );
    await checkbox.setValue(true);
    await checkbox.trigger("change");
    expect(mockedToggle).toHaveBeenCalledWith({
      enabled: true,
      port: 18080,
      baseUrl: "https://api.example.com/v1",
      nudgeEnabled: true,
    });
  });

  it("地址为空时仍允许关闭代理", async () => {
    store.settings.compat_proxy_enabled = true;
    const wrapper = mountSection();
    const checkbox = wrapper.find(
      ".model-config-head-actions input[type='checkbox']",
    );
    await checkbox.setValue(false);
    await checkbox.trigger("change");
    expect(mockedToggle).toHaveBeenCalledWith({
      enabled: false,
      port: 18080,
      baseUrl: "",
      nudgeEnabled: true,
    });
  });

  it("保存合法端口与 API 地址调用 applyCompatProxy", async () => {
    const wrapper = mountSection();
    const input = wrapper.find("input[type='number']");
    await input.setValue("19090");
    const urlInput = wrapper.find(".compat-proxy-url-input");
    await urlInput.setValue("https://custom.example.com/v1");
    const btn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("保存"));
    await btn!.trigger("click");
    expect(mockedApply).toHaveBeenCalledWith(
      {
        enabled: false,
        port: 19090,
        baseUrl: "https://custom.example.com/v1",
        nudgeEnabled: true,
      },
      true,
    );
  });
});
