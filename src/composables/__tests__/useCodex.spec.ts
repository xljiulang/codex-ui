import { describe, expect, it, beforeEach } from "vitest";
import { vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

import { invoke } from "@tauri-apps/api/core";
import {
  ensureThreadPlugins,
  sendPrompt,
  store,
  toastError,
} from "../useCodex";

const mockedInvoke = vi.mocked(invoke);

const PLUGINS_RESPONSE = {
  marketplaces: [
    {
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          installed: true,
          enabled: true,
          source: { path: "C:/x/documents" },
          interface: {
            displayName: "Documents",
            shortDescription: "文档处理",
            composerIcon: "C:/x/documents/icon.png",
            composerIconUrl: null,
            brandColor: "#2563EB",
          },
        },
        {
          id: "pdf@openai-primary-runtime",
          name: "pdf",
          installed: true,
          enabled: true,
          source: { path: "C:/x/pdf" },
          interface: { displayName: "PDF" },
        },
        {
          id: "disabled@openai-curated",
          name: "disabled",
          installed: false,
          enabled: false,
          source: { path: "C:/x/disabled" },
          interface: { displayName: "Disabled" },
        },
      ],
    },
    {
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          installed: true,
          enabled: true,
          source: { path: "C:/x/dup" },
          interface: { displayName: "Duplicate" },
        },
      ],
    },
  ],
};

describe("toastError 错误提示提取", () => {
  it("优先提取 error.error.message / error.message，回退 String(e)", () => {
    expect(toastError({ error: { message: "服务端错误" } })).toBe("服务端错误");
    expect(toastError({ message: "普通错误" })).toBe("普通错误");
    expect(toastError("raw string")).toBe("raw string");
    expect(toastError(42)).toBe("42");
    expect(toastError(null)).toBe("null");
  });
});

describe("队列模式下发送提示", () => {
  beforeEach(() => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.settings.followup_mode = "queue";
    store.toast = "";
    store.followupQueue.splice(0);
    store.attachments.splice(0);
  });

  it("回合进行中且为队列模式：消息入队并提示", async () => {
    await sendPrompt("第二条消息");
    expect(store.followupQueue).toHaveLength(1);
    expect(store.followupQueue[0].text).toBe("第二条消息");
    expect(store.toast).toContain("已加入队列");
  });
});

describe("ensureThreadPlugins 对话级插件缓存", () => {
  beforeEach(() => {
    store.threadPlugins = {};
    mockedInvoke.mockReset();
  });

  it("为对话归一化 plugin/list：过滤未安装/禁用插件并按 id 去重", async () => {
    mockedInvoke.mockResolvedValue(PLUGINS_RESPONSE);
    await ensureThreadPlugins("t1");
    expect(store.threadPlugins["t1"]).toEqual({
      loaded: true,
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          displayName: "Documents",
          description: "文档处理",
          path: "C:/x/documents",
          iconPath: "C:/x/documents/icon.png",
          iconUrl: "",
          brandColor: "#2563EB",
        },
        {
          id: "pdf@openai-primary-runtime",
          name: "pdf",
          displayName: "PDF",
          description: "",
          path: "C:/x/pdf",
          iconPath: "",
          iconUrl: "",
          brandColor: "",
        },
      ],
    });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/list",
      params: {},
    });
    const skillsCalls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(skillsCalls).toHaveLength(0);
  });

  it("同对话复用缓存不重复请求，不同对话各自缓存", async () => {
    mockedInvoke.mockResolvedValue(PLUGINS_RESPONSE);
    await ensureThreadPlugins("t1");
    await ensureThreadPlugins("t1"); // 同对话复用
    let calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/list",
    );
    expect(calls).toHaveLength(1);

    await ensureThreadPlugins("t2"); // 新对话预加载
    await ensureThreadPlugins("t1"); // 仍复用
    calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/list",
    );
    expect(calls).toHaveLength(2);
  });

  it("plugin/list 失败时该对话缓存为空且不回退 skills/list", async () => {
    mockedInvoke.mockRejectedValue(new Error("boom"));
    await ensureThreadPlugins("t1");
    expect(store.threadPlugins["t1"]).toEqual({
      plugins: [],
      loaded: false,
    });
    const skillsCalls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(skillsCalls).toHaveLength(0);
  });
});
