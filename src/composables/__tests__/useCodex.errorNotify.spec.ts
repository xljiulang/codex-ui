// codex 错误 → Windows 通知：前端触发层的门槛（设置/会话归属）与文案拼装
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  codexErrorNoticeBody,
  codexErrorNoticeTitle,
  flattenNoticeText,
  notifyCodexError,
  sessionTitleForThread,
} from "../useCodex/errorNotify";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { backgroundThreadIds, store } from "../useCodex/store";
import { defaultSettings } from "../useCodex/types";
import { makeSessionTab, tabs } from "./useCodexTestHarness";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

/** 取出本次 notify_codex_error 调用参数（未调用返回 undefined） */
function notifyArgs() {
  const call = mockedInvoke.mock.calls.find(([cmd]) => cmd === "notify_codex_error");
  return call?.[1] as
    | { title: string; body: string; threadId: string; turnId?: string | null }
    | undefined;
}

beforeEach(() => {
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation(() => Promise.resolve(undefined));
  vi.mocked(listen).mockReset();
  __resetSessionTabsForTest();
  store.settings = defaultSettings();
  store.threads = [];
});

describe("flattenNoticeText", () => {
  it("换行/制表/连续空白压成单空格并去首尾空白", () => {
    expect(flattenNoticeText("第一行\n第二行\t尾部  ", 200)).toBe(
      "第一行 第二行 尾部",
    );
    expect(flattenNoticeText("   ", 200)).toBe("");
  });

  it("按字符截断并追加省略号（按 Unicode 码点计数）", () => {
    expect(flattenNoticeText("错".repeat(205), 200).length).toBe(201);
    expect(flattenNoticeText("错".repeat(205), 200).endsWith("…")).toBe(true);
    expect(flattenNoticeText("短", 200)).toBe("短");
  });
});

describe("codexErrorNoticeBody", () => {
  it("走友好中文映射（结构化 codexErrorInfo 优先）", () => {
    expect(
      codexErrorNoticeBody("raw message", "contextWindowExceeded"),
    ).toBe("上下文已超出模型窗口");
    expect(
      codexErrorNoticeBody(
        "cannot resume running thread thr_9 with history while it is already running",
      ),
    ).toBe("该会话正被占用（已有回合在运行或其它进程持有），请稍后再试");
  });

  it("未命中映射时保留原文", () => {
    expect(codexErrorNoticeBody("some future error")).toBe("some future error");
  });
});

describe("codexErrorNoticeTitle / sessionTitleForThread", () => {
  it("打开标签有名称时带会话名", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "修复登录" }));
    expect(sessionTitleForThread("t1")).toBe("修复登录");
    expect(codexErrorNoticeTitle("t1")).toBe("Codex 错误 · 修复登录");
  });

  it("标签无名称时回退历史摘要（name → preview）", () => {
    tabs.push(makeSessionTab("s1", "t1"));
    store.threads = [
      { id: "t1", name: null, preview: "摘要标题", createdAt: 0 },
    ];
    expect(codexErrorNoticeTitle("t1")).toBe("Codex 错误 · 摘要标题");
  });

  it("取不到会话名时只用固定标题（不显示「新建会话」占位）", () => {
    tabs.push(makeSessionTab("s1", "t1"));
    expect(sessionTitleForThread("t1")).toBe("");
    expect(codexErrorNoticeTitle("t1")).toBe("Codex 错误");
  });

  it("会话名超长时截断到 40 字符", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "标".repeat(60) }));
    const title = codexErrorNoticeTitle("t1");
    expect(title).toBe(`Codex 错误 · ${"标".repeat(40)}…`);
  });
});

describe("notifyCodexError", () => {
  it("命中前台/无会话/空正文以外的一切情况：带会话名与回合 id 调后端命令", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "修复登录" }));
    notifyCodexError({
      message:
        "cannot resume running thread thr_9 with history while it is already running",
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(notifyArgs()).toEqual({
      title: "Codex 错误 · 修复登录",
      body: "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试",
      threadId: "t1",
      turnId: "turn-1",
    });
  });

  it("无回合 id 时传 null（后端按正文去重）", () => {
    notifyCodexError({ message: "boom", threadId: "t1" });
    expect(notifyArgs()?.turnId).toBeNull();
  });

  it("设置关闭时不发通知", () => {
    store.settings.error_notify_enabled = false;
    notifyCodexError({ message: "boom", threadId: "t1" });
    expect(notifyArgs()).toBeUndefined();
  });

  it("无 threadId 时不发通知（无法定位会话）", () => {
    notifyCodexError({ message: "boom", threadId: null });
    notifyCodexError({ message: "boom" });
    expect(notifyArgs()).toBeUndefined();
  });

  it("正文为空时不发通知", () => {
    notifyCodexError({ message: "   ", threadId: "t1" });
    expect(notifyArgs()).toBeUndefined();
  });

  it("后台临时线程（如标题总结 ephemeral 线程）不发通知", () => {
    backgroundThreadIds.add("helper-1");
    notifyCodexError({ message: "boom", threadId: "helper-1" });
    backgroundThreadIds.delete("helper-1");
    expect(notifyArgs()).toBeUndefined();
  });

  it("invoke 失败不影响调用方（静默）", async () => {
    mockedInvoke.mockImplementation(() => Promise.reject(new Error("no ipc")));
    expect(() => notifyCodexError({ message: "boom", threadId: "t1" })).not.toThrow();
    await Promise.resolve();
  });
});
