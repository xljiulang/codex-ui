import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
/** 捕获事件处理器，便于测试里手动派发进度/完成事件 */
const listeners = new Map<string, (e: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  deleteKnowledge,
  knowledgeKbs,
  knowledgeProgress,
  refreshKnowledgeList,
  startKnowledgeIndex,
  subscribeKnowledgeEvents,
} from "../useKnowledge";
import { store } from "../useCodex";

const mockedInvoke = vi.mocked(invoke);

const row = {
  file: "shouhou-1a2b3c4d.sqlite",
  cwd: "D:\\work\\售后",
  docs: 2,
  chunks: 7,
  updated_at: 1700000000,
  available: true,
  error: null,
};

beforeEach(() => {
  listeners.clear();
  knowledgeKbs.value = [];
  knowledgeProgress.value = {};
  store.toast = "";
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation(async (cmd) => {
    if (cmd === "knowledge_list") return [{ ...row }];
    if (cmd === "knowledge_index_start") return { text: "已开始建库" };
    return undefined;
  });
});

describe("useKnowledge（列表 / 删除 / 建库事件）", () => {
  it("刷新列表：读取 knowledge_list；异常响应回落到空列表", async () => {
    await refreshKnowledgeList();
    expect(knowledgeKbs.value).toHaveLength(1);
    expect(knowledgeKbs.value[0].cwd).toBe("D:\\work\\售后");

    mockedInvoke.mockImplementation(async () => undefined);
    await refreshKnowledgeList();
    expect(knowledgeKbs.value).toEqual([]);
  });

  it("删除后自动刷新列表", async () => {
    await deleteKnowledge(row.file);
    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_delete", {
      file: row.file,
    });
    const listCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "knowledge_list",
    );
    expect(listCalls).toHaveLength(1);
  });

  it("建库入口透传 cwd/paths/full 并回传摘要文本", async () => {
    const text = await startKnowledgeIndex("D:\\work\\售后", ["D:\\work\\售后\\手册"], true);
    expect(text).toBe("已开始建库");
    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_index_start", {
      cwd: "D:\\work\\售后",
      paths: ["D:\\work\\售后\\手册"],
      full: true,
    });
  });

  it("进度事件写入（键按规范化工作目录）并在完成事件时清理 + 刷新 + toast", async () => {
    const unlisten = await subscribeKnowledgeEvents();

    listeners.get("knowledge/index-progress")!({
      payload: {
        cwd: "D:/work/售后/",
        phase: "index",
        processed: 3,
        total: 10,
        current: "D:\\work\\售后\\手册.pdf",
      },
    });
    // 大小写/斜杠/尾分隔符差异归一后写同一个键
    expect(knowledgeProgress.value["d:\\work\\售后"]).toContain("3/10");
    expect(knowledgeProgress.value["d:\\work\\售后"]).toContain("手册.pdf");

    listeners.get("knowledge/index-done")!({
      payload: {
        cwd: "D:\\work\\售后",
        summary: { text: "知识库更新：新增 1 篇文档" },
      },
    });
    await flushPromises();
    expect(knowledgeProgress.value["d:\\work\\售后"]).toBeUndefined();
    expect(store.toast).toContain("新增 1 篇文档");
    expect(
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "knowledge_list"),
    ).toHaveLength(1);

    unlisten();
    expect(listeners.has("knowledge/index-progress")).toBe(false);
  });

  it("完成事件带 error 时 toast 失败原因并清进度", async () => {
    await subscribeKnowledgeEvents();
    knowledgeProgress.value = { "d:\\kb": "建库中 1/2" };
    listeners.get("knowledge/index-done")!({
      payload: { cwd: "D:\\kb", error: "向量模型未就绪" },
    });
    await flushPromises();
    expect(store.toast).toContain("向量模型未就绪");
    expect(knowledgeProgress.value["d:\\kb"]).toBeUndefined();
  });

  it("done 阶段进度事件只清进度不写入文案", async () => {
    await subscribeKnowledgeEvents();
    knowledgeProgress.value = { "d:\\kb": "建库中 2/2" };
    listeners.get("knowledge/index-progress")!({
      payload: {
        cwd: "D:\\kb",
        phase: "done",
        processed: 2,
        total: 2,
        current: "",
      },
    });
    expect(knowledgeProgress.value["d:\\kb"]).toBeUndefined();
  });
});
