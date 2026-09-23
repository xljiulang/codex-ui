import { describe, expect, it } from "vitest";
import {
  buildTurns,
  createTurnsBuilder,
  findCurrentTurnIndex,
  markdownToPlainText,
  turnPreviewText,
} from "../turns";
import type { ThreadItem } from "../types";

function msg(id: string, type: string, ts?: number): ThreadItem {
  return {
    id,
    type,
    ...(ts === undefined ? {} : { startedAtMs: ts }),
  } as ThreadItem;
}

function formatDay(ts: number): string {
  const d = new Date(ts);
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}

describe("buildTurns 回合分组", () => {
  it("userMessage 起始新回合，后续条目归入当前回合", () => {
    const turns = buildTurns(
      [
        msg("u1", "userMessage"),
        msg("r1", "reasoning"),
        msg("t1", "commandExecution"),
        msg("a1", "agentMessage"),
        msg("u2", "userMessage"),
        msg("a2", "agentMessage"),
      ],
      formatDay,
    );
    expect(turns.map((t) => t.key)).toEqual(["turn-u1", "turn-u2"]);
    expect(turns[0].rows.map((r) => r.key)).toEqual(["u1", "r1", "t1", "a1"]);
    expect(turns[1].rows.map((r) => r.key)).toEqual(["u2", "a2"]);
  });

  it("首个用户消息之前的条目归入单一伪回合", () => {
    const turns = buildTurns(
      [
        msg("a0", "agentMessage"),
        msg("t0", "fileChange"),
        msg("u1", "userMessage"),
        msg("a1", "agentMessage"),
      ],
      formatDay,
    );
    expect(turns.map((t) => t.key)).toEqual(["pre-a0", "turn-u1"]);
    expect(turns[0].rows.map((r) => r.key)).toEqual(["a0", "t0"]);
    expect(turns[1].rows.map((r) => r.key)).toEqual(["u1", "a1"]);
  });

  it("连续 userMessage 各自成回合", () => {
    const turns = buildTurns(
      [
        msg("u1", "userMessage"),
        msg("u2", "userMessage"),
        msg("a1", "agentMessage"),
      ],
      formatDay,
    );
    expect(turns.map((t) => t.key)).toEqual(["turn-u1", "turn-u2"]);
    expect(turns[1].rows.map((r) => r.key)).toEqual(["u2", "a1"]);
  });

  it("空列表返回空回合", () => {
    expect(buildTurns([], formatDay)).toEqual([]);
  });

  it("跨天分隔线插入所属回合：首个用户回合与 pre 回合各带一条", () => {
    const day1 = new Date(2026, 7, 9, 10, 0).getTime();
    const day2 = new Date(2026, 7, 10, 9, 0).getTime();
    const turns = buildTurns(
      [
        msg("a0", "agentMessage", day1),
        msg("u1", "userMessage", day2),
        msg("a1", "agentMessage", day2),
      ],
      formatDay,
    );
    expect(turns[0].rows.map((r) => r.kind)).toEqual(["sep", "msg"]);
    expect(turns[0].rows[0]).toMatchObject({ kind: "sep", date: "2026-8-9" });
    expect(turns[1].rows.map((r) => r.kind)).toEqual(["sep", "msg", "msg"]);
    expect(turns[1].rows[0]).toMatchObject({ kind: "sep", date: "2026-8-10" });
  });

  it("同一天不重复插入分隔线，缺少时间戳时不插入", () => {
    const day1 = new Date(2026, 7, 9, 10, 0).getTime();
    const turns = buildTurns(
      [
        msg("u1", "userMessage", day1),
        msg("a1", "agentMessage", day1),
        msg("a2", "agentMessage"),
      ],
      formatDay,
    );
    expect(
      turns.flatMap((t) => t.rows).filter((r) => r.kind === "sep"),
    ).toHaveLength(1);
  });

  it("builder 行对象按 key 复用，未变化条目引用不变", () => {
    const builder = createTurnsBuilder(formatDay);
    const items = [msg("u1", "userMessage"), msg("a1", "agentMessage")];
    const first = builder.build(items);
    const second = builder.build(items);
    expect(first[0].rows[1]).toBe(second[0].rows[1]);
    // 切换会话后清缓存
    builder.clear();
    const third = builder.build(items);
    expect(third[0].rows[1]).not.toBe(first[0].rows[1]);
  });

  it("跨次 build 日期分隔线 key 稳定复用，缓存不随构建次数增长", () => {
    const builder = createTurnsBuilder(formatDay);
    const day = new Date(2026, 7, 9, 10, 0).getTime();
    const items = [
      msg("u1", "userMessage", day),
      msg("a1", "agentMessage", day),
    ];

    const seps = (b: ReturnType<typeof createTurnsBuilder>) =>
      b
        .build(items)
        .flatMap((t) => t.rows)
        .filter((r) => r.kind === "sep");

    const firstSep = seps(builder)[0];
    const secondSep = seps(builder)[0];
    // 同一位置的分隔线复用同一行对象/key（旧实现每次 build 生成新 key 并缓存）
    expect(secondSep).toBe(firstSep);
    expect(secondSep?.key).toBe(firstSep?.key);
  });
});

describe("findCurrentTurnIndex 当前回合判定", () => {
  const tops = [100, 300, 500];

  it("空锚点列表返回 -1", () => {
    expect(findCurrentTurnIndex([], 100)).toBe(-1);
  });

  it("视口仍在首个锚点上方时返回 -1", () => {
    expect(findCurrentTurnIndex(tops, 90)).toBe(-1);
  });

  it("恰好等于锚点顶时返回该下标", () => {
    expect(findCurrentTurnIndex(tops, 100)).toBe(0);
    expect(findCurrentTurnIndex(tops, 300)).toBe(1);
    expect(findCurrentTurnIndex(tops, 500)).toBe(2);
  });

  it("位于两个锚点之间时返回上一个", () => {
    expect(findCurrentTurnIndex(tops, 250)).toBe(0);
    expect(findCurrentTurnIndex(tops, 450)).toBe(1);
  });

  it("容差内越过锚点顶部仍算当前回合", () => {
    expect(findCurrentTurnIndex(tops, 101)).toBe(0);
    expect(findCurrentTurnIndex(tops, 102)).toBe(0);
  });

  it("超过最后一个锚点后返回末尾下标", () => {
    expect(findCurrentTurnIndex(tops, 600)).toBe(2);
  });
});

describe("回合导航预览文本", () => {
  it("markdown 标题/列表/代码块/链接转为纯文本", () => {
    expect(
      markdownToPlainText(
        [
          "# 标题",
          "- 列表项 **加粗**",
          "`inline` [链接](https://x)  ![图](a.png)",
          "```ts\nconst x = 1;\n```",
          "结尾",
        ].join("\n"),
      ),
    ).toBe("标题 列表项 加粗 inline 链接 图 结尾");
  });

  it("用户消息正文剥离文件引用段", () => {
    const item = {
      id: "u1",
      type: "userMessage",
      content: [
        {
          type: "text",
          text: [
            "# Files mentioned by the user:",
            "## a.cs: D:/a.cs",
            "",
            "## My request:",
            "请看看 **这个** 文件",
          ].join("\n"),
          text_elements: [],
        },
      ],
    } as const;
    expect(
      turnPreviewText(item as unknown as import("../types").ThreadItem),
    ).toBe("请看看 这个 文件");
  });

  it("纯图片/引用消息回退占位文本", () => {
    const item = {
      id: "u2",
      type: "userMessage",
      content: [
        { type: "localImage", path: "D:/a.png" },
        { type: "mention", name: "app.ts", path: "D:/app.ts" },
        { type: "skill", name: "skill-a", path: "D:/SKILL.md" },
      ],
    } as const;
    expect(
      turnPreviewText(item as unknown as import("../types").ThreadItem),
    ).toBe("[图片] @app.ts $skill-a");
  });
});
