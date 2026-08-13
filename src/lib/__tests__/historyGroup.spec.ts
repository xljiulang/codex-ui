import { describe, expect, it } from "vitest";
import {
  dirLabel,
  groupThreads,
  normalizeDirKey,
} from "../historyGroup";
import type { ThreadSummary } from "../types";

function t(
  id: string,
  over: Partial<ThreadSummary> = {},
): ThreadSummary {
  return {
    id,
    createdAt: 0,
    recencyAt: 0,
    ...over,
  } as ThreadSummary;
}

describe("historyGroup 目录分组", () => {
  it("相同 cwd 的会话归入同一目录，仅 1 条也建目录", () => {
    const rows = groupThreads([
      t("a", { cwd: "D:\\codex\\codex-ui", recencyAt: 2 }),
      t("b", { cwd: "D:\\codex\\codex-ui", recencyAt: 1 }),
      t("c", { cwd: "D:\\codex\\codex-proxy", recencyAt: 3 }),
    ]);

    expect(rows).toHaveLength(2);
    expect(rows[0].kind).toBe("group");
    expect(rows[1].kind).toBe("group");
    if (rows[0].kind === "group" && rows[1].kind === "group") {
      // 目录按组内第一会话时间倒序：codex-proxy（c=3）在 codex-ui（a=2）之前
      expect(rows[0].group.threads.map((x) => x.id)).toEqual(["c"]);
      expect(rows[1].group.threads.map((x) => x.id)).toEqual(["a", "b"]);
    }
  });

  it("cwd 缺失或空串的会话保持平铺，不建目录", () => {
    const rows = groupThreads([
      t("a", { cwd: "" }),
      t("b", { cwd: "D:\\repo" }),
      t("c", { cwd: undefined }),
    ]);

    const groups = rows.filter((r) => r.kind === "group");
    const items = rows.filter((r) => r.kind === "item");
    expect(groups).toHaveLength(1);
    expect(items.map((r) => (r.kind === "item" ? r.thread.id : ""))).toEqual(["a", "c"]);
  });

  it("label 取路径最后一段，兼容反斜杠/正斜杠/根路径", () => {
    expect(dirLabel("D:\\codex\\codex-ui")).toBe("codex-ui");
    expect(dirLabel("D:/codex/codex-ui/")).toBe("codex-ui");
    expect(dirLabel("C:\\")).toBe("C:\\");
  });

  it("Windows 下大小写不同的同路径合并为同一目录（保留原路径展示）", () => {
    const rows = groupThreads([
      t("a", { cwd: "D:\\Codex\\Codex-UI" }),
      t("b", { cwd: "d:\\codex\\codex-ui" }),
    ]);

    expect(rows).toHaveLength(1);
    if (rows[0].kind === "group") {
      expect(rows[0].group.key).toBe(normalizeDirKey("D:\\Codex\\Codex-UI"));
      expect(rows[0].group.threads).toHaveLength(2);
      expect(rows[0].group.path).toBe("D:\\Codex\\Codex-UI");
    }
  });

  it("顶层排序：目录按组内第一会话的置顶优先 + 时间倒序", () => {
    const rows = groupThreads([
      t("old", { cwd: "D:\\a", recencyAt: 100 }),
      t("new", { cwd: "D:\\b", recencyAt: 200 }),
      t("pin", { cwd: "D:\\pin-dir", recencyAt: 50, isPinned: true }),
    ]);

    const labels = rows.map((r) =>
      r.kind === "group" ? r.group.label : r.thread.id,
    );
    expect(labels).toEqual(["pin-dir", "b", "a"]);
  });

  it("组内对话按置顶优先 + 最近时间倒序（与入参顺序无关）", () => {
    const rows = groupThreads([
      t("n2", { cwd: "D:\\x", recencyAt: 3 }),
      t("p1", { cwd: "D:\\x", recencyAt: 1, isPinned: true }),
      t("n1", { cwd: "D:\\x", recencyAt: 5 }),
    ]);

    expect(rows).toHaveLength(1);
    if (rows[0].kind === "group") {
      expect(rows[0].group.threads.map((x) => x.id)).toEqual(["p1", "n1", "n2"]);
    }
  });

  it("目录按组内第一会话排序（并列时 A-Z 兜底），平铺会话在后", () => {
    const rows = groupThreads([
      t("s1", { recencyAt: 100 }),
      t("a1", { cwd: "D:\\zeta", recencyAt: 5 }),
      t("b1", { cwd: "D:\\alpha", recencyAt: 5 }),
      t("sp", { recencyAt: 50, isPinned: true }),
    ]);

    const labels = rows.map((r) =>
      r.kind === "group" ? r.group.label : r.thread.id,
    );
    expect(labels).toEqual(["alpha", "zeta", "sp", "s1"]);
  });
});
