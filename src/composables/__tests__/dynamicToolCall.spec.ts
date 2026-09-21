import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { handleDynamicToolCall } from "../useCodex/dynamicToolCall";
import { store } from "../useCodex";
import { freshSessionTab } from "../useCodex/sessionState";
import { tabs } from "../useTabs";
import type { ScheduledTask } from "../../lib/types";

const mockedInvoke = vi.mocked(invoke);

/** 组装 item/tool/call 载荷（协议 DynamicToolCallParams：threadId/turnId/callId/namespace/tool/arguments） */
function payload(arguments_: Record<string, unknown>) {
  return callPayload("add_scheduled_task", arguments_);
}

/** 指定工具名的载荷（知识库工具用） */
function callPayload(tool: string, arguments_: Record<string, unknown>) {
  return {
    requestId: 1,
    method: "item/tool/call",
    params: {
      threadId: "t1",
      turnId: "turn-1",
      callId: "call-1",
      namespace: "codexui",
      tool,
      arguments: arguments_,
    },
  };
}

/** 准备一个绑定 t1 的会话标签（知识库按工作目录归属） */
function useSessionTab(workspace: string | null = "D:\\kb") {
  tabs.splice(0);
  const tab = freshSessionTab();
  tab.threadId = "t1";
  tab.workspace = workspace;
  tabs.push(tab);
  return tab;
}

const task: ScheduledTask = {
  id: "task-1",
  name: "每日总结",
  prompt: "总结提交",
  cron: "0 0 9 * * *",
  threadId: "t1",
  busyPolicy: "defer",
  enabled: true,
  done: false,
  createdAt: 100,
  nextRun: 200,
};

/** 提取 interaction_respond 的应答文本 */
async function respondTexts(): Promise<string[]> {
  await flushPromises();
  return mockedInvoke.mock.calls
    .filter(([cmd]) => cmd === "interaction_respond")
    .map(([, args]) => JSON.stringify(args));
}

function addCalls(): number {
  return mockedInvoke.mock.calls.filter(([cmd]) => cmd === "scheduled_task_add")
    .length;
}

beforeEach(() => {
  store.confirm = null;
  store.interactions = [];
  store.settings.dynamic_tools_state = {};
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation(async (cmd) => {
    if (cmd === "scheduled_task_add") return { ...task };
    return undefined;
  });
});

describe("动态工具 add_scheduled_task（直接创建，无前端确认框）", () => {
  it("调用后直接创建任务并应答成功（仅应答一次）", async () => {
    await handleDynamicToolCall(
      payload({ name: "每日总结", prompt: "总结提交", cron: "0 0 9 * * *" }),
    );
    expect(addCalls()).toBe(1);
    const texts = await respondTexts();
    expect(texts).toHaveLength(1);
    expect(texts[0]).toContain("已创建");
    expect(store.confirm).toBeNull();
  });

  it("busyPolicy 透传给创建命令：显式 defer 则 defer，缺省/未知则 skip", async () => {
    await handleDynamicToolCall(
      payload({ name: "n", prompt: "p", cron: "0 0 9 * * *", busyPolicy: "defer" }),
    );
    await flushPromises();
    const [, args] = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "scheduled_task_add",
    )!;
    expect((args as { busyPolicy: string }).busyPolicy).toBe("defer");

    mockedInvoke.mockClear();
    await handleDynamicToolCall(
      payload({ name: "n", prompt: "p", cron: "0 0 9 * * *" }),
    );
    await flushPromises();
    const [, defaultArgs] = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "scheduled_task_add",
    )!;
    expect((defaultArgs as { busyPolicy: string }).busyPolicy).toBe("skip");
  });

  it("工具被显式关闭时应答 failure（dynamic_tools_state=false）", async () => {
    store.settings.dynamic_tools_state = { "codexui.add_scheduled_task": false };
    await handleDynamicToolCall(
      payload({ name: "n", prompt: "p", cron: "0 0 9 * * *" }),
    );
    const texts = await respondTexts();
    expect(texts).toHaveLength(1);
    expect(texts[0]).toContain("未开启");
    expect(addCalls()).toBe(0);
  });

  it("缺少必填参数时应答 failure", async () => {
    await handleDynamicToolCall(
      payload({ name: "n", prompt: "", cron: "0 0 9 * * *" }),
    );
    const texts = await respondTexts();
    expect(texts).toHaveLength(1);
    expect(texts[0]).toContain("均为必填");
    expect(addCalls()).toBe(0);
  });

  it("后端校验失败（如 cron 颗粒度过小）应答 failure 并带回错误文案", async () => {
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === "scheduled_task_add")
        throw new Error("定时颗粒度过小，最小间隔为 1 分钟（建议不低于 5 分钟）");
      return undefined;
    });
    await handleDynamicToolCall(
      payload({ name: "n", prompt: "p", cron: "* * * * * *" }),
    );
    const texts = await respondTexts();
    expect(texts[0]).toContain("最小间隔为 1 分钟");
  });
});

describe("知识库动态工具已移除（改由 skill 调 CLI）", () => {
  it("search_docs 不再被处理：响应未知工具且不触发检索", async () => {
    useSessionTab("D:\\work\\售后");
    store.settings.dynamic_tools_state = { "codexui.search_docs": true };
    await handleDynamicToolCall(callPayload("search_docs", { query: "无法开机" }));
    const texts = await respondTexts();
    expect(texts[0]).toContain("未知的 codexui 工具");
    expect(
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "knowledge_search"),
    ).toHaveLength(0);
  });

  it("index_docs 不再被处理：响应未知工具且不调建库命令", async () => {
    useSessionTab();
    store.settings.dynamic_tools_state = { "codexui.index_docs": true };
    await handleDynamicToolCall(callPayload("index_docs", {}));
    const texts = await respondTexts();
    expect(texts[0]).toContain("未知的 codexui 工具");
    expect(
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "knowledge_index_start"),
    ).toHaveLength(0);
  });
});

