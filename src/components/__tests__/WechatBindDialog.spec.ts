import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    wechatBindLoginStart: vi.fn().mockResolvedValue(undefined),
    wechatUnbind: vi.fn().mockResolvedValue(undefined),
    refreshWeChatState: vi.fn().mockResolvedValue(undefined),
    setToast: vi.fn(),
  };
});
vi.mock("qrcode", () => ({
  toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,QR"),
}));

import { store } from "../../composables/useCodex";
import {
  wechatBindLoginStart,
  wechatUnbind,
} from "../../composables/useCodex";
import WechatBindDialog from "../WechatBindDialog.vue";

const mockedBind = vi.mocked(wechatBindLoginStart);
const mockedUnbind = vi.mocked(wechatUnbind);

const thread = { id: "t1", name: "会话一", createdAt: 0, recencyAt: 0 };

function mountDialog() {
  return mount(WechatBindDialog, { props: { thread } });
}

describe("WechatBindDialog", () => {
  beforeEach(() => {
    store.wechat = null;
    mockedBind.mockClear();
    mockedUnbind.mockClear();
  });

  it("未绑定态展示扫码绑定按钮并调用 bind_login_start", async () => {
    const wrapper = mountDialog();
    expect(wrapper.find(".wechat-bind-btn").exists()).toBe(true);
    await wrapper.find(".wechat-bind-btn").trigger("click");
    await flushPromises();
    expect(mockedBind).toHaveBeenCalledWith("t1");
  });

  it("已绑定态展示账号信息与解除绑定按钮", async () => {
    store.wechat = {
      running: true,
      connection: "connected",
      detail: null,
      qrContent: null,
      pendingThreadId: null,
      queued: 0,
      busy: false,
      bindings: [
        { threadId: "t1", accountId: "bot-1", connection: "connected" },
      ],
    };
    const wrapper = mountDialog();
    expect(wrapper.text()).toContain("账号：bot-1");
    expect(wrapper.find(".wechat-unbind-btn").exists()).toBe(true);
    await wrapper.find(".wechat-unbind-btn").trigger("click");
    await flushPromises();
    expect(mockedUnbind).toHaveBeenCalledWith("t1");
  });

  it("本会话扫码中：渲染二维码并展示等待扫码", async () => {
    store.wechat = {
      running: true,
      connection: "awaiting_qr",
      detail: null,
      qrContent: "http://weixin/abc",
      pendingThreadId: "t1",
      queued: 0,
      busy: false,
      bindings: [],
    };
    const wrapper = mountDialog();
    await flushPromises();
    const img = wrapper.find(".wechat-qr-wrap img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("data:image/png;base64,QR");
    expect(wrapper.text()).toContain("请使用要绑定的微信扫码并确认授权");
  });

  it("本会话 pending 但二维码尚未到达：提示正在生成", async () => {
    store.wechat = {
      running: true,
      connection: "starting",
      detail: null,
      qrContent: null,
      pendingThreadId: "t1",
      queued: 0,
      busy: false,
      bindings: [],
    };
    const wrapper = mountDialog();
    expect(wrapper.find(".wechat-qr-wrap img").exists()).toBe(false);
    expect(wrapper.text()).toContain("正在生成二维码，请稍候…");
  });

  it("其它会话扫码中：提示等待且绑定按钮禁用", async () => {
    store.wechat = {
      running: true,
      connection: "awaiting_qr",
      detail: null,
      qrContent: "http://weixin/abc",
      pendingThreadId: "t9",
      queued: 0,
      busy: false,
      bindings: [],
    };
    const wrapper = mountDialog();
    expect(wrapper.find(".wechat-qr-wrap img").exists()).toBe(false);
    expect(wrapper.text()).toContain("已有其他会话正在绑定微信");
    expect(
      (wrapper.find(".wechat-bind-btn").element as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("关闭按钮触发 close 事件", async () => {
    const wrapper = mountDialog();
    const footBtn = wrapper.findAll(".modal-foot .btn").find((b) => b.text().trim() === "关闭");
    expect(footBtn).toBeTruthy();
    await footBtn!.trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });
});
