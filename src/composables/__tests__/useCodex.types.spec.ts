import { toastError } from "../useCodex/toast";
import { isGoalStatus } from "../useCodex/types";
import { resetUseCodexState } from "./useCodexTestHarness";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

const { mockWin } = vi.hoisted(() => ({
  mockWin: {
    label: "main",
    isMinimized: vi.fn().mockResolvedValue(false),
    unminimize: vi.fn(),
    setFocus: vi.fn(),
    setTitle: vi.fn(),
    setProgressBar: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => mockWin),
  ProgressBarStatus: { Indeterminate: "Indeterminate", None: "None" },
}));

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);

beforeEach(() => {
  resetUseCodexState(mockedInvoke, mockedListen);
});

describe("toastError 错误提示提取", () => {
  it("优先提取 error.error.message / error.message，回退 String(e)", () => {
    expect(toastError({ error: { message: "服务端错误" } })).toBe("服务端错误");
    expect(toastError({ message: "普通错误" })).toBe("普通错误");
    expect(toastError("raw string")).toBe("raw string");
    expect(toastError(42)).toBe("42");
    expect(toastError(null)).toBe("null");
  });

  it("服务端已知消息映射为友好中文", () => {
    expect(
      toastError({
        error: {
          message:
            "Context window exceeded while compacting; removing oldest history item.",
        },
      }),
    ).toBe("上下文超出窗口，已自动压缩并移除最早的历史内容");
    expect(toastError("Server overloaded; retry later.")).toBe(
      "服务过载，请稍后重试",
    );
    expect(
      toastError(
        "cannot resume running thread thr_1 with history while it is already running",
      ),
    ).toBe("该会话正被占用（已有回合在运行或其它进程持有），请稍后再试");
  });
});
describe("GoalStatus 枚举对齐协议 ThreadGoalStatus", () => {
  it("接受协议全部取值（active/paused/blocked/usageLimited/budgetLimited/complete）", () => {
    expect(isGoalStatus("active")).toBe(true);
    expect(isGoalStatus("paused")).toBe(true);
    expect(isGoalStatus("blocked")).toBe(true);
    expect(isGoalStatus("usageLimited")).toBe(true);
    expect(isGoalStatus("budgetLimited")).toBe(true);
    expect(isGoalStatus("complete")).toBe(true);
  });

  it("拒绝旧版/非协议取值（completed/budget_limited/cleared 等）", () => {
    expect(isGoalStatus("completed")).toBe(false);
    expect(isGoalStatus("budget_limited")).toBe(false);
    expect(isGoalStatus("cleared")).toBe(false);
    expect(isGoalStatus("")).toBe(false);
    expect(isGoalStatus(null)).toBe(false);
    expect(isGoalStatus(undefined)).toBe(false);
  });
});
