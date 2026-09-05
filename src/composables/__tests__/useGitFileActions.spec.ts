import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { ActionMenuItem } from "../useActionMenu";
import type { GitDirNode, GitFileNode } from "../../lib/gitTree";
import type { GitStatus } from "../../lib/gitChanges";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useCodex")>();
  return { ...mod, askConfirm: vi.fn(), setToast: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import { askConfirm } from "../useCodex";
import { useGitFileActions } from "../useGitFileActions";

const mockedInvoke = vi.mocked(invoke);
const mockedAskConfirm = vi.mocked(askConfirm);

const okStatus: GitStatus = {
  repoWorkspace: "D:\\repo",
  branch: "main",
  hasRemote: true,
  files: [],
};

function fileNode(
  path: string,
  status: GitFileNode["file"]["status"],
  staged = false,
): GitFileNode {
  return {
    kind: "file",
    name: path.split("/").pop() ?? path,
    relPath: path,
    depth: 0,
    file: { path, status, staged, worktree: !staged },
  };
}

function dirNode(overrides: Partial<GitDirNode> = {}): GitDirNode {
  return {
    kind: "dir",
    name: "src",
    relPath: "src",
    depth: 0,
    collapsed: false,
    childCount: 1,
    hasStaged: false,
    hasUnstaged: true,
    hasUntracked: false,
    children: [],
    ...overrides,
  };
}

function setup() {
  const gitStatus = ref<GitStatus | null>(okStatus);
  const captured: ActionMenuItem[][] = [];
  const openCtx = vi.fn((_e: MouseEvent, items: ActionMenuItem[]) => {
    captured.push(items);
  });
  const openDiff = vi.fn();
  const actions = useGitFileActions({ gitStatus, openCtx, openDiff });
  return { gitStatus, captured, openCtx, openDiff, actions };
}

function mockEvent() {
  return {
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  } as unknown as MouseEvent;
}

describe("useGitFileActions 右键菜单", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedAskConfirm.mockReset();
  });

  it("modified 文件：更改区含暂存，暂存区含取消暂存，均含撤消", () => {
    const { captured, actions } = setup();
    actions.openFileCtx("changes", fileNode("a.txt", "modified").file, mockEvent());
    expect(captured[0].map((i) => i.label)).toEqual([
      "打开",
      "暂存",
      "撤消更改",
    ]);

    actions.openFileCtx("staged", fileNode("a.txt", "modified", true).file, mockEvent());
    expect(captured[1].map((i) => i.label)).toEqual([
      "打开",
      "取消暂存",
      "撤消更改",
    ]);
  });

  it("untracked 含暂存/忽略/删除，added 含取消暂存/删除", () => {
    const { captured, actions } = setup();
    actions.openFileCtx("changes", fileNode("b.txt", "untracked").file, mockEvent());
    expect(captured[0].map((i) => i.label)).toEqual([
      "打开",
      "暂存",
      "忽略此本地项",
      "删除文件",
    ]);

    actions.openFileCtx("staged", fileNode("c.txt", "added", true).file, mockEvent());
    expect(captured[1].map((i) => i.label)).toEqual([
      "打开",
      "取消暂存",
      "删除文件",
    ]);
  });

  it("conflicted 仅保留打开；目录菜单按分区聚合", () => {
    const { captured, actions } = setup();
    actions.openFileCtx("changes", fileNode("x.txt", "conflicted").file, mockEvent());
    expect(captured[0].map((i) => i.label)).toEqual(["打开"]);

    actions.openDirCtx("changes", dirNode({ hasUntracked: true }), mockEvent());
    expect(captured[1].map((i) => i.label)).toEqual([
      "暂存",
      "忽略此本地项",
      "撤消更改",
    ]);

    actions.openDirCtx("staged", dirNode(), mockEvent());
    expect(captured[2].map((i) => i.label)).toEqual(["取消暂存", "撤消更改"]);
  });
});

describe("useGitFileActions 文件操作", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedAskConfirm.mockReset();
  });

  it("stageFile 调 git_changes_stage 并回写 gitStatus", async () => {
    const { gitStatus, actions } = setup();
    mockedInvoke.mockResolvedValueOnce(okStatus);
    actions.stageFile(fileNode("a.txt", "modified").file);
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("git_changes_stage", {
        workspace: "D:\\repo",
        path: "a.txt",
      });
    });
    expect(gitStatus.value).toStrictEqual(okStatus);
  });

  it("restoreFile 未确认不调用，确认后调 git_changes_restore", async () => {
    const { actions } = setup();
    mockedAskConfirm.mockResolvedValueOnce(false);
    await actions.restoreFile(fileNode("a.txt", "modified").file);
    expect(mockedInvoke).not.toHaveBeenCalled();

    mockedAskConfirm.mockResolvedValueOnce(true);
    mockedInvoke.mockResolvedValueOnce(okStatus);
    await actions.restoreFile(fileNode("a.txt", "modified").file);
    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_restore", {
      workspace: "D:\\repo",
      path: "a.txt",
    });
  });

  it("stageAll 调 git_changes_stage_all", async () => {
    const { actions } = setup();
    mockedInvoke.mockResolvedValueOnce(okStatus);
    actions.stageAll();
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("git_changes_stage_all", {
        workspace: "D:\\repo",
      });
    });
  });

  it("无仓库根目录时静默返回", async () => {
    const gitStatus = ref<GitStatus | null>(null);
    const actions = useGitFileActions({
      gitStatus,
      openCtx: vi.fn(),
      openDiff: vi.fn(),
    });
    actions.stageFile(fileNode("a.txt", "modified").file);
    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});

describe("useGitFileActions 分区菜单与分区撤销", () => {
  const changesStatus: GitStatus = {
    ...okStatus,
    files: [
      { path: "a.txt", status: "modified", staged: false, worktree: true },
      { path: "b.txt", status: "untracked", staged: false, worktree: true },
    ],
  };
  const stagedStatus: GitStatus = {
    ...okStatus,
    files: [
      { path: "a.txt", status: "modified", staged: true, worktree: false },
    ],
  };

  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedAskConfirm.mockReset();
  });

  it("openSectionCtx：更改区为暂存/撤消更改，暂存区为取消暂存/撤消更改，空分区不弹", () => {
    const { gitStatus, captured, actions } = setup();
    actions.openSectionCtx("changes", mockEvent());
    expect(captured.length).toBe(0);

    gitStatus.value = changesStatus;
    actions.openSectionCtx("changes", mockEvent());
    expect(captured[0].map((i) => i.label)).toEqual(["暂存", "撤消更改"]);
    expect(captured[0][1].danger).toBe(true);

    gitStatus.value = stagedStatus;
    actions.openSectionCtx("staged", mockEvent());
    expect(captured[1].map((i) => i.label)).toEqual(["取消暂存", "撤消更改"]);
  });

  it("restoreSection：未确认不执行，确认后逐文件调用 restore", async () => {
    const { gitStatus, actions } = setup();
    gitStatus.value = changesStatus;

    mockedAskConfirm.mockResolvedValueOnce(false);
    await actions.restoreSection("changes");
    expect(mockedInvoke).not.toHaveBeenCalled();

    mockedAskConfirm.mockResolvedValueOnce(true);
    mockedInvoke.mockResolvedValue(changesStatus);
    await actions.restoreSection("changes");
    expect(mockedAskConfirm).toHaveBeenCalledWith(
      expect.objectContaining({ title: "撤消更改", confirmLabel: "撤消更改" }),
    );
    const paths = mockedInvoke.mock.calls
      .filter(([cmd]) => cmd === "git_changes_restore")
      .map(([, args]) => (args as { path: string }).path);
    expect(paths).toEqual(["a.txt", "b.txt"]);
  });
});
