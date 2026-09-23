import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { GitStatus } from "../../lib/gitChanges";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useCodex")>();
  return { ...mod, setToast: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import { useGitRemoteOps } from "../useGitRemoteOps";

const mockedInvoke = vi.mocked(invoke);

function status(branch = "main"): GitStatus {
  return { repoWorkspace: "D:\\repo", branch, hasRemote: true, files: [] };
}

describe("useGitRemoteOps", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("doPull 成功后回写 gitStatus", async () => {
    const gitStatus = ref<GitStatus | null>(status());
    mockedInvoke.mockResolvedValueOnce({
      status: status("main"),
      message: "已是最新",
    });
    const ops = useGitRemoteOps({ gitStatus });
    await ops.doPull();
    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_pull", {
      workspace: "D:\\repo",
    });
    expect(ops.pullBusy.value).toBe(false);
  });

  it("游离 HEAD 不发起拉取/推送", async () => {
    const gitStatus = ref<GitStatus | null>(status("HEAD"));
    const ops = useGitRemoteOps({ gitStatus });
    await ops.doPull();
    await ops.doPush();
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("忙碌互斥：push 忙时 pull 被跳过", async () => {
    const ops = useGitRemoteOps({ gitStatus: ref<GitStatus | null>(status()) });
    ops.pushBusy.value = true;
    await ops.doPull();
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("checkGitAvailable 探测失败视为不可用", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("no git"));
    const ops = useGitRemoteOps({ gitStatus: ref<GitStatus | null>(status()) });
    await ops.checkGitAvailable();
    expect(ops.gitAvailable.value).toBe(false);
  });

  it("checkGitAvailable 传入工作区并写回可用状态", async () => {
    mockedInvoke.mockResolvedValueOnce(true);
    const ops = useGitRemoteOps({ gitStatus: ref<GitStatus | null>(status()) });
    await ops.checkGitAvailable();
    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_git_available", {
      workspace: "D:\\repo",
    });
    expect(ops.gitAvailable.value).toBe(true);
  });

  it("无仓库根目录时静默返回", async () => {
    const ops = useGitRemoteOps({ gitStatus: ref<GitStatus | null>(null) });
    await ops.doPull();
    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});
