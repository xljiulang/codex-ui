// 会话级事件 → Windows 通知：前端触发层的门槛（设置/会话归属）与文案拼装
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  errorNoticeBody,
  flattenNoticeText,
  interactionKindForMethod,
  noticeTitle,
  notifySessionError,
  notifySessionInteraction,
  sessionTitleForThread,
} from "../useCodex/sessionNotify";
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

/** 取出本次 notify_session_event 调用参数（未调用返回 undefined） */
function notifyArgs() {
  const call = mockedInvoke.mock.calls.find(([cmd]) => cmd === "notify_session_event");
  return call?.[1] as
    | {
        title: string;
        body: string;
        threadId: string;
        turnId?: string | null;
        source: string;
      }
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

describe("errorNoticeBody", () => {
  it("走友好中文映射（结构化 codexErrorInfo 优先）", () => {
    expect(errorNoticeBody("raw message", "contextWindowExceeded")).toBe(
      "上下文已超出模型窗口",
    );
    expect(
      errorNoticeBody(
        "cannot resume running thread thr_9 with history while it is already running",
      ),
    ).toBe("该会话正被占用（已有回合在运行或其它进程持有），请稍后再试");
  });

  it("未命中映射时保留原文", () => {
    expect(errorNoticeBody("some future error")).toBe("some future error");
  });
});

describe("noticeTitle / sessionTitleForThread", () => {
  it("打开标签有名称时带会话名", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "修复登录" }));
    expect(sessionTitleForThread("t1")).toBe("修复登录");
    expect(noticeTitle("会话错误", "t1")).toBe("会话错误 · 修复登录");
  });

  it("标签无名称时回退历史摘要（name → preview）", () => {
    tabs.push(makeSessionTab("s1", "t1"));
    store.threads = [
      { id: "t1", name: null, preview: "摘要标题", createdAt: 0 },
    ];
    expect(noticeTitle("需要审批", "t1")).toBe("需要审批 · 摘要标题");
  });

  it("取不到会话名时只用固定前缀（不显示「新建会话」占位）", () => {
    tabs.push(makeSessionTab("s1", "t1"));
    expect(sessionTitleForThread("t1")).toBe("");
    expect(noticeTitle("会话错误", "t1")).toBe("会话错误");
  });

  it("会话名超长时截断到 40 字符", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "标".repeat(60) }));
    expect(noticeTitle("会话错误", "t1")).toBe(`会话错误 · ${"标".repeat(40)}…`);
  });
});

describe("interactionKindForMethod", () => {
  it("审批类 method（v2 与 legacy）归入 approval", () => {
    for (const m of [
      "item/commandExecution/requestApproval",
      "execCommandApproval",
      "item/fileChange/requestApproval",
      "applyPatchApproval",
      "item/permissions/requestApproval",
    ]) {
      expect(interactionKindForMethod(m)).toBe("approval");
    }
  });

  it("提问与 MCP 表单各自归类，未知 method 归入 other", () => {
    expect(interactionKindForMethod("item/tool/requestUserInput")).toBe("question");
    expect(interactionKindForMethod("mcpServer/elicitation/request")).toBe(
      "elicitation",
    );
    expect(interactionKindForMethod("some/future/request")).toBe("other");
  });
});

describe("notifySessionError", () => {
  it("带会话名与回合 id 调后端命令（source=error）", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "修复登录" }));
    notifySessionError({
      message:
        "cannot resume running thread thr_9 with history while it is already running",
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(notifyArgs()).toEqual({
      title: "会话错误 · 修复登录",
      body: "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试",
      threadId: "t1",
      turnId: "turn-1",
      source: "error",
    });
  });

  it("无回合 id 时传 null（后端按正文去重）", () => {
    notifySessionError({ message: "boom", threadId: "t1" });
    expect(notifyArgs()?.turnId).toBeNull();
  });

  it("设置关闭时不发通知", () => {
    store.settings.error_notify_enabled = false;
    notifySessionError({ message: "boom", threadId: "t1" });
    expect(notifyArgs()).toBeUndefined();
  });

  it("无 threadId 时不发通知（无法定位会话）", () => {
    notifySessionError({ message: "boom", threadId: null });
    notifySessionError({ message: "boom" });
    expect(notifyArgs()).toBeUndefined();
  });

  it("正文为空时不发通知", () => {
    notifySessionError({ message: "   ", threadId: "t1" });
    expect(notifyArgs()).toBeUndefined();
  });

  it("后台临时线程（如标题总结 ephemeral 线程）不发通知", () => {
    backgroundThreadIds.add("helper-1");
    notifySessionError({ message: "boom", threadId: "helper-1" });
    backgroundThreadIds.delete("helper-1");
    expect(notifyArgs()).toBeUndefined();
  });

  it("invoke 失败不影响调用方（静默）", async () => {
    mockedInvoke.mockImplementation(() => Promise.reject(new Error("no ipc")));
    expect(() => notifySessionError({ message: "boom", threadId: "t1" })).not.toThrow();
    await Promise.resolve();
  });
});

describe("notifySessionInteraction", () => {
  it("审批/提问/MCP 表单：固定标题+通用正文（source=interaction）", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "修复登录" }));
    const cases: [Parameters<typeof notifySessionInteraction>[0]["kind"], string, string][] = [
      ["approval", "需要审批 · 修复登录", "会话正在等待你批准操作"],
      ["question", "需要输入 · 修复登录", "会话正在等待你回答问题"],
      ["elicitation", "MCP 表单 · 修复登录", "MCP 工具正在等待你填写表单"],
      ["other", "需要确认 · 修复登录", "会话正在等待你的确认"],
    ];
    for (const [kind, title, body] of cases) {
      mockedInvoke.mockClear();
      notifySessionInteraction({ kind, threadId: "t1" });
      expect(notifyArgs()).toEqual({
        title,
        body,
        threadId: "t1",
        turnId: null,
        source: "interaction",
      });
    }
  });

  it("计划就绪：source 记为 plan", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "修复登录" }));
    notifySessionInteraction({ kind: "plan", threadId: "t1" });
    expect(notifyArgs()).toEqual({
      title: "计划已就绪 · 修复登录",
      body: "会话已产出计划，等待你确认是否执行",
      threadId: "t1",
      turnId: null,
      source: "plan",
    });
  });

  it("无会话名时标题只用固定前缀", () => {
    notifySessionInteraction({ kind: "approval", threadId: "t1" });
    expect(notifyArgs()?.title).toBe("需要审批");
  });

  it("设置关闭时不发通知", () => {
    store.settings.interaction_notify_enabled = false;
    notifySessionInteraction({ kind: "approval", threadId: "t1" });
    expect(notifyArgs()).toBeUndefined();
  });

  it("无 threadId 时不发通知", () => {
    notifySessionInteraction({ kind: "approval", threadId: null });
    notifySessionInteraction({ kind: "approval" });
    expect(notifyArgs()).toBeUndefined();
  });

  it("后台临时线程不发通知", () => {
    backgroundThreadIds.add("helper-1");
    notifySessionInteraction({ kind: "approval", threadId: "helper-1" });
    backgroundThreadIds.delete("helper-1");
    expect(notifyArgs()).toBeUndefined();
  });
});
