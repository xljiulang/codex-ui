import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

const { mockListen } = vi.hoisted(() => ({ mockListen: vi.fn() }));

vi.mock("@tauri-apps/api/event", () => ({ listen: mockListen }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main", setTitle: vi.fn() }),
}));

import { invoke } from "@tauri-apps/api/core";
import { store } from "../useCodex";
import {
  __resetGitChangesForTest,
  gitErrorMsg,
  gitState,
  gitStatus,
  initGitRepo,
  refreshGitChanges,
  setGitChangesActive,
} from "../useGitChanges";

const mockedInvoke = vi.mocked(invoke);
const STATUS = {
  repoRoot: "D:/repo",
  branch: "main",
  hasRemote: true,
  files: [],
};

describe("useGitChanges 状态机与监听", () => {
  beforeEach(() => {
    __resetGitChangesForTest();
    store.currentThreadId = "t1";
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.server.workspace = "";
    mockedInvoke.mockReset();
    mockListen.mockReset();
    mockListen.mockResolvedValue(vi.fn());
  });

  afterEach(() => {
    __resetGitChangesForTest();
  });

  it("刷新成功进入 ok 并保存状态", async () => {
    store.currentThreadCwd = "D:/repo";
    mockedInvoke.mockResolvedValue(STATUS);
    await refreshGitChanges();
    expect(gitState.value).toBe("ok");
    expect(gitStatus.value).toEqual(STATUS);
  });

  it("not_a_repo 错误进入 not_repo", async () => {
    store.currentThreadCwd = "D:/repo";
    mockedInvoke.mockRejectedValue({ code: "not_a_repo", message: "not a git repo" });
    await refreshGitChanges();
    expect(gitState.value).toBe("not_repo");
    expect(gitStatus.value).toBeNull();
  });

  it("其他错误进入 error 并提取中文提示", async () => {
    store.currentThreadCwd = "D:/repo";
    mockedInvoke.mockRejectedValue({ code: "repo_error", message: "boom" });
    await refreshGitChanges();
    expect(gitState.value).toBe("error");
    expect(gitErrorMsg.value).toBe("boom");
  });

  it("无工作目录时进入 error 并提示", async () => {
    await refreshGitChanges();
    expect(gitState.value).toBe("error");
    expect(gitErrorMsg.value).toBe("暂无工作目录");
  });

  it("initGitRepo 成功返回 true 并刷新为 ok", async () => {
    store.currentThreadCwd = "D:/repo";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "git_changes_status") return Promise.resolve(STATUS);
      return Promise.resolve(undefined);
    });
    const ok = await initGitRepo();
    expect(ok).toBe(true);
    expect(gitState.value).toBe("ok");
  });

  it("激活时启动监听并刷新，停用时停止监听", async () => {
    store.currentThreadCwd = "D:/repo";
    mockedInvoke.mockResolvedValue(STATUS);
    setGitChangesActive(true);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_watch_start", {
      root: "D:/repo",
    });
    setGitChangesActive(false);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_watch_stop");
  });

  it("1 秒冷却期内监听事件不触发重复刷新", async () => {
    store.currentThreadCwd = "D:/repo";
    mockedInvoke.mockResolvedValue(STATUS);
    setGitChangesActive(true);
    await flushPromises();
    const handler = mockListen.mock.calls[0]?.[1];
    expect(handler).toBeTypeOf("function");
    await refreshGitChanges();
    await flushPromises();
    handler();
    await flushPromises();
    const statusCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "git_changes_status",
    ).length;
    expect(statusCalls).toBe(2);
  });
});
