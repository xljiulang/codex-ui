import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { handleDynamicToolCall } from "../useCodex/dynamicToolCall";
import { store } from "../useCodex";
import type { ScheduledTask } from "../../lib/types";

const mockedInvoke = vi.mocked(invoke);

/** 组装 item/tool/call 载荷（协议 DynamicToolCallParams：threadId/turnId/callId/namespace/tool/arguments） */
function payload(arguments_: Record<string, unknown>) {
  return {
    requestId: 1,
    method: "item/tool/call",
    params: {
      threadId: "t1",
      turnId: "turn-1",
      callId: "call-1",
      namespace: "codexui",
      tool: "add_scheduled_task",
      arguments: arguments_,
    },
  };
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
  store.settings.dynamic_tools_disabled = [];
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

  it("busyPolicy 透传给创建命令，缺省 defer", async () => {
    await handleDynamicToolCall(
      payload({ name: "n", prompt: "p", cron: "0 0 9 * * *", busyPolicy: "skip" }),
    );
    await flushPromises();
    const [, args] = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "scheduled_task_add",
    )!;
    expect((args as { busyPolicy: string }).busyPolicy).toBe("skip");
  });

  it("工具被禁用时应答 failure（dynamic_tools_disabled 过滤）", async () => {
    store.settings.dynamic_tools_disabled = ["codexui.add_scheduled_task"];
    await handleDynamicToolCall(
      payload({ name: "n", prompt: "p", cron: "0 0 9 * * *" }),
    );
    const texts = await respondTexts();
    expect(texts).toHaveLength(1);
    expect(texts[0]).toContain("已被禁用");
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
