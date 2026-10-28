import { describe, expect, it, vi } from "vitest";
import { finishStartup } from "../bootRecovery";

describe("启动恢复决策", () => {
  it("renderer 恢复后打开设置且不恢复上次重型会话", async () => {
    const restoreLastSession = vi.fn(async () => true);
    const openSettingsTab = vi.fn();
    const notifyRecovery = vi.fn();

    await finishStartup({
      recovery: {
        recovered: true,
        failureKind: "renderProcessExited",
        exitCode: 3221226504,
        happenedAtMs: 1,
      },
      restoreLastSession,
      openSettingsTab,
      notifyRecovery,
    });

    expect(restoreLastSession).not.toHaveBeenCalled();
    expect(openSettingsTab).toHaveBeenCalledTimes(1);
    expect(notifyRecovery).toHaveBeenCalledWith(
      "界面渲染进程异常，已使用安全模式恢复。请从右侧历史重新打开会话。",
    );
  });

  it("正常启动仍优先恢复上次会话", async () => {
    const restoreLastSession = vi.fn(async () => true);
    const openSettingsTab = vi.fn();

    await finishStartup({
      recovery: null,
      restoreLastSession,
      openSettingsTab,
      notifyRecovery: vi.fn(),
    });

    expect(restoreLastSession).toHaveBeenCalledTimes(1);
    expect(openSettingsTab).not.toHaveBeenCalled();
  });

  it("正常启动且没有可恢复会话时打开设置", async () => {
    const openSettingsTab = vi.fn();

    await finishStartup({
      recovery: null,
      restoreLastSession: async () => false,
      openSettingsTab,
      notifyRecovery: vi.fn(),
    });

    expect(openSettingsTab).toHaveBeenCalledTimes(1);
  });
});
