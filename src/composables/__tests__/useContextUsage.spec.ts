import { beforeEach, describe, expect, it, vi } from "vitest";
import { reactive } from "vue";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useCodex")>();
  return { ...mod, setToast: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import { setToast, type SessionTab } from "../useCodex";
import { useContextUsage } from "../useContextUsage";
import { activeTabId, tabs } from "../useEditorTabs";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { makeSessionTab } from "./useCodexTestHarness";

const mockedInvoke = vi.mocked(invoke);
const mockedToast = vi.mocked(setToast);

describe("useContextUsage", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedToast.mockClear();
    __resetSessionTabsForTest();
    tabs.push(reactive(makeSessionTab("s1", "t1")));
    activeTabId.value = "s1";
  });

  it("window 已知时计算百分比并截断到 100", () => {
    (tabs[0] as SessionTab).threadTokenUsage = {
      contextUsed: 5000,
      window: 10000,
    };
    const u = useContextUsage();
    expect(u.ctxUsage.value).toEqual({ pct: 50, used: 5000, window: 10000 });

    (tabs[0] as SessionTab).threadTokenUsage = {
      contextUsed: 12000,
      window: 10000,
    };
    expect(u.ctxUsage.value?.pct).toBe(100);
  });

  it("window 缺失/非正数时返回 null", () => {
    const u = useContextUsage();
    expect(u.ctxUsage.value).toBeNull();
    (tabs[0] as SessionTab).threadTokenUsage = { contextUsed: 100, window: 0 };
    expect(u.ctxUsage.value).toBeNull();
    expect(u.ctxTooltip.value).toBe("");
  });

  it("tooltip 按万/亿格式化", () => {
    (tabs[0] as SessionTab).threadTokenUsage = {
      contextUsed: 1_500_000,
      window: 2_000_000,
    };
    const u = useContextUsage();
    expect(u.ctxTooltip.value).toBe("上下文已用 150万，共 200万，双击进行压缩");
  });

  it("compactNow 发起 thread/compact/start 并提示；无会话不发起", async () => {
    const u = useContextUsage();
    mockedInvoke.mockResolvedValueOnce(undefined);
    await u.compactNow();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/compact/start",
      params: { threadId: "t1" },
    });
    expect(mockedToast).toHaveBeenCalledWith("已开始压缩上下文");

    // 无活动会话（新建对话编辑态）不发起压缩
    __resetSessionTabsForTest();
    await u.compactNow();
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
  });

  it("压缩进行中防重入，失败时 toast 错误", async () => {
    const u = useContextUsage();
    mockedInvoke.mockRejectedValueOnce(new Error("boom"));
    await u.compactNow();
    expect(mockedToast).toHaveBeenCalled();
    expect(u.compacting.value).toBe(false);
  });

  it("传入 tab 时以该 tab 为准，而非活动会话", () => {
    const other = reactive(makeSessionTab("s2", "t2"));
    tabs.push(other);
    (other as SessionTab).threadTokenUsage = {
      contextUsed: 2000,
      window: 4000,
    };
    // 活动会话为 s1（无 window），传入 s2 时仍应基于 s2 计算
    activeTabId.value = "s1";
    const u = useContextUsage(other);
    expect(u.ctxUsage.value).toEqual({ pct: 50, used: 2000, window: 4000 });
  });

  it("传入 tab 时压缩以该 tab 的 threadId 发起", async () => {
    const other = reactive(makeSessionTab("s2", "t2"));
    tabs.push(other);
    mockedInvoke.mockResolvedValueOnce(undefined);
    const u = useContextUsage(other);
    await u.compactNow();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/compact/start",
      params: { threadId: "t2" },
    });
  });
});
