import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { sessionLog } from "../sessionLog";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("sessionLog", () => {
  it("透传 level/threadId/event/detail 参数", async () => {
    await sessionLog("info", "t-1", "user-send", "chars=3");
    expect(invoke).toHaveBeenCalledWith("session_log", {
      level: "info",
      threadId: "t-1",
      event: "user-send",
      detail: "chars=3",
    });
  });

  it("threadId/detail 缺省时传 null", async () => {
    await sessionLog("warn", undefined, "thread-delete");
    expect(invoke).toHaveBeenCalledWith("session_log", {
      level: "warn",
      threadId: null,
      event: "thread-delete",
      detail: null,
    });
  });

  it("invoke 失败时静默不抛错", async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error("boom"));
    await expect(
      sessionLog("error", "t-1", "user-stop"),
    ).resolves.toBeUndefined();
  });
});
