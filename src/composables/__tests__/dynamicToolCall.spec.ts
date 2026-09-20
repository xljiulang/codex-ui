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

describe("动态工具 search_docs / index_docs（知识库，默认关闭）", () => {
  it("默认关闭：search_docs 未被显式开启时应答 failure 且不触发检索", async () => {
    useSessionTab();
    await handleDynamicToolCall(callPayload("search_docs", { query: "无法开机" }));
    const texts = await respondTexts();
    expect(texts[0]).toContain("未开启");
    expect(
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "knowledge_search"),
    ).toHaveLength(0);
  });

  it("开启后按会话工作目录检索，应答带出处与匹配分", async () => {
    useSessionTab("D:\\work\\售后");
    store.settings.dynamic_tools_state = { "codexui.search_docs": true };
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === "knowledge_search") {
        return [
          {
            doc_path: "D:\\work\\售后\\手册.pdf",
            title_path: "售后手册 › 第3章 故障 › 3.2 无法开机",
            text: "请检查电源适配器是否插紧。",
            score: 0.82,
          },
        ];
      }
      return undefined;
    });

    await handleDynamicToolCall(
      callPayload("search_docs", { query: "设备无法开机", topK: 3 }),
    );
    const [, args] = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "knowledge_search",
    )!;
    expect(args).toEqual({
      cwd: "D:\\work\\售后",
      query: "设备无法开机",
      topK: 3,
    });

    const texts = await respondTexts();
    expect(texts[0]).toContain("手册.pdf");
    expect(texts[0]).toContain("第3章 故障");
    expect(texts[0]).toContain("匹配 0.82");
    expect(texts[0]).toContain("请检查电源适配器");
  });

  it("空结果为成功应答并提示可先建库（不报错）", async () => {
    useSessionTab();
    store.settings.dynamic_tools_state = { "codexui.search_docs": true };
    await handleDynamicToolCall(callPayload("search_docs", { query: "没有的词" }));
    const texts = await respondTexts();
    expect(texts[0]).toContain("没有匹配");
    expect(texts[0]).toContain("index_docs");
  });

  it("缺少工作目录时明确报错（不落到活动标签兜底）", async () => {
    useSessionTab(null);
    store.settings.dynamic_tools_state = { "codexui.search_docs": true };
    await handleDynamicToolCall(callPayload("search_docs", { query: "问题" }));
    const texts = await respondTexts();
    expect(texts[0]).toContain("没有工作目录");
    expect(
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "knowledge_search"),
    ).toHaveLength(0);
  });

  it("缺少 query 参数时报错", async () => {
    useSessionTab();
    store.settings.dynamic_tools_state = { "codexui.search_docs": true };
    await handleDynamicToolCall(callPayload("search_docs", {}));
    const texts = await respondTexts();
    expect(texts[0]).toContain("缺少 query");
  });

  it("检索命令报错时把原因回传给 agent", async () => {
    useSessionTab();
    store.settings.dynamic_tools_state = { "codexui.search_docs": true };
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === "knowledge_search") throw new Error("向量模型未就绪");
      return undefined;
    });
    await handleDynamicToolCall(callPayload("search_docs", { query: "问题" }));
    const texts = await respondTexts();
    expect(texts[0]).toContain("向量模型未就绪");
  });

  it("index_docs：开启后按工作目录建库并透传 Rust 摘要文本", async () => {
    useSessionTab("D:\\work\\售后");
    store.settings.dynamic_tools_state = { "codexui.index_docs": true };
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === "knowledge_index_start") {
        return { text: "知识库无变化（内容与上次索引一致）：当前共 3 篇文档 / 12 个切块。" };
      }
      return undefined;
    });

    await handleDynamicToolCall(
      callPayload("index_docs", { paths: ["D:\\work\\售后\\手册"], full: true }),
    );
    const [, args] = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "knowledge_index_start",
    )!;
    expect(args).toEqual({
      cwd: "D:\\work\\售后",
      paths: ["D:\\work\\售后\\手册"],
      full: true,
    });
    const texts = await respondTexts();
    expect(texts[0]).toContain("无变化");
    expect(texts[0]).toContain("3 篇");
  });

  it("index_docs：缺省不传 paths/full（由 Rust 用工作目录增量）", async () => {
    useSessionTab();
    store.settings.dynamic_tools_state = { "codexui.index_docs": true };
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === "knowledge_index_start") return { text: "已开始" };
      return undefined;
    });
    await handleDynamicToolCall(callPayload("index_docs", {}));
    const [, args] = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "knowledge_index_start",
    )!;
    expect(args).toEqual({ cwd: "D:\\kb", paths: undefined, full: undefined });
  });

  it("index_docs：缺少工作目录时报错且不调用命令", async () => {
    useSessionTab(null);
    store.settings.dynamic_tools_state = { "codexui.index_docs": true };
    await handleDynamicToolCall(callPayload("index_docs", {}));
    const texts = await respondTexts();
    expect(texts[0]).toContain("没有工作目录");
    expect(
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "knowledge_index_start"),
    ).toHaveLength(0);
  });
});
