import { describe, expect, it } from "vitest";
import type { CommandExecutionItem, ThreadItem } from "../types";
import {
  applyHistoryDetailPage,
  capHistoryCommandOutput,
  composeHistoryItems,
  createHistoryTurnStates,
  findNextOlderDetailTurnId,
} from "../historyWindow";

function user(id: string): ThreadItem {
  return {
    id,
    type: "userMessage",
    content: [{ type: "text", text: id, text_elements: [] }],
  };
}

function agent(id: string): ThreadItem {
  return { id, type: "agentMessage", text: id };
}

describe("历史窗口合成", () => {
  it("摘要用户消息置首、最终答复补尾，明细按原顺序填充且按 id 去重", () => {
    const turns = createHistoryTurnStates([
      {
        id: "turn-1",
        status: "completed",
        items: [user("u1"), agent("a1")],
      },
    ]);
    const loaded = applyHistoryDetailPage(
      turns[0],
      [
        agent("a1"),
        { id: "r1", type: "reasoning", content: ["思考"] },
        {
          id: "c1",
          type: "commandExecution",
          command: "x",
          status: "completed",
        },
        user("u1"),
      ],
      null,
    );

    expect(composeHistoryItems([loaded]).map((item) => item.id)).toEqual([
      "u1",
      "c1",
      "r1",
      "a1",
    ]);
  });

  it("较旧的倒序分页插到当前明细之前，跨页重复项只保留一次", () => {
    const turns = createHistoryTurnStates([
      {
        id: "turn-1",
        status: "completed",
        items: [user("u1"), agent("a1")],
      },
    ]);
    const newest = applyHistoryDetailPage(
      turns[0],
      [
        agent("a1"),
        {
          id: "c2",
          type: "commandExecution",
          command: "2",
          status: "completed",
        },
        {
          id: "c1",
          type: "commandExecution",
          command: "1",
          status: "completed",
        },
      ],
      "older-1",
    );
    const older = applyHistoryDetailPage(
      newest,
      [
        {
          id: "c1",
          type: "commandExecution",
          command: "1",
          status: "completed",
        },
        user("u1"),
      ],
      null,
    );

    expect(older.detailItems.map((item) => item.id)).toEqual([
      "u1",
      "c1",
      "c2",
      "a1",
    ]);
    expect(older.detailHasMore).toBe(false);
  });

  it("自动向前加载优先补完连续明细窗口中的当前旧回合，再进入上一回合", () => {
    const turns = createHistoryTurnStates([
      { id: "turn-3", status: "completed", items: [user("u3")] },
      { id: "turn-2", status: "completed", items: [user("u2")] },
      { id: "turn-1", status: "completed", items: [user("u1")] },
    ]);
    const withLatest = turns.map((turn, index) =>
      index >= 1
        ? {
            ...turn,
            detailLoaded: true,
            detailHasMore: index === 1,
          }
        : turn,
    );

    expect(findNextOlderDetailTurnId(withLatest)).toBe("turn-2");
    withLatest[1] = { ...withLatest[1], detailHasMore: false };
    expect(findNextOlderDetailTurnId(withLatest)).toBe("turn-1");
  });

  it("最新回合本身仍有更早明细页时先补完该回合", () => {
    const turns = createHistoryTurnStates([
      { id: "turn-3", status: "completed", items: [user("u3")] },
      { id: "turn-2", status: "completed", items: [user("u2")] },
    ]).map((turn) => ({
      ...turn,
      detailLoaded: true,
      detailHasMore: turn.id === "turn-3",
    }));

    expect(findNextOlderDetailTurnId(turns)).toBe("turn-3");
  });
});

describe("历史命令输出截断", () => {
  it("只保留末尾 5000 行并标记截断，不修改协议原对象", () => {
    const source = Array.from({ length: 6000 }, (_, i) => `L${i}`).join("\n");
    const item: CommandExecutionItem = {
      id: "c1",
      type: "commandExecution",
      command: "run",
      status: "completed",
      aggregatedOutput: source,
    };

    const capped = capHistoryCommandOutput(item);

    expect(item.aggregatedOutput).toBe(source);
    const lines = String(capped.aggregatedOutput).split("\n");
    expect(lines).toHaveLength(5000);
    expect(lines[0]).toBe("L1000");
    expect(lines[lines.length - 1]).toBe("L5999");
    expect(capped._historyOutputTruncated).toBe(true);
    expect(capped._historyOutputTruncation).toBe("lines");
  });

  it("超长单行按 512 KiB UTF-8 字节截断并保留合法字符尾部", () => {
    const source = `prefix-${"中".repeat(200_000)}-suffix`;
    const item: CommandExecutionItem = {
      id: "c1",
      type: "commandExecution",
      command: "run",
      status: "completed",
      aggregatedOutput: source,
    };

    const capped = capHistoryCommandOutput(item);
    const output = String(capped.aggregatedOutput);

    expect(new TextEncoder().encode(output).length).toBeLessThanOrEqual(
      512 * 1024,
    );
    expect(output.endsWith("-suffix")).toBe(true);
    expect(output.includes("\uFFFD")).toBe(false);
    expect(capped._historyOutputTruncated).toBe(true);
    expect(capped._historyOutputTruncation).toBe("bytes");
  });
});
