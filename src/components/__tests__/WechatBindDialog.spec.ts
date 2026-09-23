import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    wechatBindLoginStart: vi.fn().mockResolvedValue(undefined),
    wechatUnbind: vi.fn().mockResolvedValue(undefined),
    wechatCancelBind: vi.fn().mockResolvedValue(undefined),
    refreshWeChatState: vi.fn().mockResolvedValue(undefined),
    setToast: vi.fn(),
  };
});
vi.mock("qrcode", () => ({
  toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,QR"),
}));

import { store } from "../../composables/useCodex";
import {
  wechatCancelBind,
  wechatBindLoginStart,
  wechatUnbind,
} from "../../composables/useCodex";
import WechatBindDialog from "../WechatBindDialog.vue";

const mockedBind = vi.mocked(wechatBindLoginStart);
const mockedUnbind = vi.mocked(wechatUnbind);
const mockedCancel = vi.mocked(wechatCancelBind);

const thread = { id: "t1", name: "会话一", createdAt: 0, recencyAt: 0 };

function mountDialog() {
  return mount(WechatBindDialog, { props: { thread } });
}

describe("WechatBindDialog", () => {
  beforeEach(() => {
    store.wechat = null;
    mockedBind.mockClear();
    mockedUnbind.mockClear();
    mockedCancel.mockClear();
  });

  it("打开弹窗即自动调用 bind_login_start（直达二维码）", async () => {
    mountDialog();
    await flushPromises();
    expect(mockedBind).toHaveBeenCalledWith("t1");
  });

  it("已绑定态：解除后自动重新扫码并继续展示二维码", async () => {
    store.wechat = {
      running: true,
      connection: "connected",
      detail: null,
      qrContent: null,
      pendingThreadId: null,
      busy: false,
      bindings: [
        { threadId: "t1", accountId: "bot-1", connection: "connected" },
      ],
    };
    mockedUnbind.mockImplementationOnce(async () => {
      // 模拟后端解除后的快照：绑定移除、无 pending → 前端应自动重新发起扫码
      store.wechat = {
        running: true,
        connection: "offline",
        detail: null,
        qrContent: null,
        pendingThreadId: null,
        busy: false,
        bindings: [],
      };
    });
    const wrapper = mountDialog();
    await flushPromises();
    expect(wrapper.text()).toContain("账号：bot-1");
    expect(wrapper.find(".wechat-unbind-btn").exists()).toBe(true);
    expect(mockedBind).not.toHaveBeenCalled();
    await wrapper.find(".wechat-unbind-btn").trigger("click");
    await flushPromises();
    expect(mockedUnbind).toHaveBeenCalledWith("t1");
    expect(mockedBind).toHaveBeenCalledWith("t1");
  });

  it("本会话扫码中：渲染二维码并展示等待扫码", async () => {
    store.wechat = {
      running: true,
      connection: "awaiting_qr",
      detail: null,
      qrContent: "http://weixin/abc",
      pendingThreadId: "t1",
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
      busy: false,
      bindings: [],
    };
    const wrapper = mountDialog();
    expect(wrapper.find(".wechat-qr-wrap img").exists()).toBe(false);
    expect(wrapper.text()).toContain("正在生成二维码，请稍候…");
  });

  it("其它会话扫码中：提示等待且不再自动发起", async () => {
    store.wechat = {
      running: true,
      connection: "awaiting_qr",
      detail: null,
      qrContent: "http://weixin/abc",
      pendingThreadId: "t9",
      busy: false,
      bindings: [],
    };
    const wrapper = mountDialog();
    await flushPromises();
    expect(wrapper.find(".wechat-qr-wrap img").exists()).toBe(false);
    expect(wrapper.text()).toContain("已有其他会话正在绑定微信");
    expect(mockedBind).not.toHaveBeenCalled();
  });

  it("非扫码中关闭：直接 close 且不取消", async () => {
    const wrapper = mountDialog();
    await flushPromises();
    const footBtn = wrapper.findAll(".modal-foot .btn").find((b) => b.text().trim() === "关闭");
    expect(footBtn).toBeTruthy();
    await footBtn!.trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
    expect(mockedCancel).not.toHaveBeenCalled();
  });

  it("本会话扫码中关闭：先取消再 close", async () => {
    store.wechat = {
      running: true,
      connection: "awaiting_qr",
      detail: null,
      qrContent: "http://weixin/abc",
      pendingThreadId: "t1",
      busy: false,
      bindings: [],
    };
    const wrapper = mountDialog();
    await flushPromises();
    const footBtn = wrapper.findAll(".modal-foot .btn").find((b) => b.text().trim() === "关闭");
    await footBtn!.trigger("click");
    await flushPromises();
    expect(mockedCancel).toHaveBeenCalled();
    expect(wrapper.emitted("close")).toHaveLength(1);
  });
});
