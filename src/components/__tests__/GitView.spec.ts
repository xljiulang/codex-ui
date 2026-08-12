import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod =
    await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    refreshThreads: vi.fn(),
  };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import GitView from "../GitView.vue";
import { store } from "../../composables/useCodex";
import { __resetGitChangesForTest } from "../../composables/useGitChanges";
import type { GitStatus } from "../../lib/gitChanges";

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);
const rootPath = "D:\\codex\\demo";

const okStatus: GitStatus = {
  repoRoot: rootPath,
  branch: "main",
  files: [
    { path: "a.txt", status: "modified" },
    { path: "b.txt", status: "untracked" },
  ],
};

function mockWatcherAndDefaults() {
  mockedInvoke.mockImplementation((cmd) => {
    if (
      cmd === "git_changes_watch_start" ||
      cmd === "git_changes_watch_stop"
    ) {
      return Promise.resolve(undefined);
    }
    return Promise.resolve(undefined);
  });
}

beforeEach(() => {
  store.threads = [];
  store.server.workspace = rootPath;
  store.currentThreadCwd = null;
  store.newChatCwd = null;
  mockedInvoke.mockClear();
  __resetGitChangesForTest();
});

describe("GitView 空状态与初始化", () => {
  it("非仓库：显示“添加到 Git”，确认后初始化并刷新", async () => {
    let inited = false;
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return inited
          ? Promise.resolve(okStatus)
          : Promise.reject({ code: "not_a_repo", message: "不在仓库内" });
      }
      if (cmd === "git_changes_init") {
        inited = true;
        return Promise.resolve(okStatus);
      }
      return Promise.resolve(undefined);
    });

    const wrapper = mount(GitView, { props: { active: true } });
    await flushPromises();

    expect(wrapper.find(".git-view").text()).toContain("不是 Git 仓库");
    expect(wrapper.find(".git-init-btn").text()).toContain("添加到 Git");

    await wrapper.find(".git-init-btn").trigger("click");
    expect(wrapper.find(".modal").exists()).toBe(true);
    await wrapper.find(".git-init-ok").trigger("click");
    await flushPromises();

    expect(
      mockedInvoke.mock.calls.some(([cmd]) => cmd === "git_changes_init"),
    ).toBe(true);
    // 初始化成功后应重新同步监听（挂载时 1 次 + init 后 1 次）
    expect(
      mockedInvoke.mock.calls.filter(
        ([cmd]) => cmd === "git_changes_watch_start",
      ).length,
    ).toBeGreaterThanOrEqual(2);
    expect(wrapper.find(".git-view").text()).toContain("main");
    expect(wrapper.find(".git-view").text()).toContain("a.txt");
  });

  it("未安装 Git：显示提示且无初始化按钮", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.reject({ code: "git_not_found", message: "未检测到 Git" });
      }
      return Promise.resolve(undefined);
    });

    const wrapper = mount(GitView, { props: { active: true } });
    await flushPromises();

    expect(wrapper.find(".git-view").text()).toContain("未检测到 Git");
    expect(wrapper.find(".git-init-btn").exists()).toBe(false);
  });

  it("其它错误：显示错误信息与重试按钮", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.reject({ code: "repo_error", message: "boom" });
      }
      return Promise.resolve(undefined);
    });

    const wrapper = mount(GitView, { props: { active: true } });
    await flushPromises();

    expect(wrapper.find(".git-view").text()).toContain("boom");
    expect(wrapper.find(".git-view").text()).toContain("重试");
  });
});

describe("GitView 文件列表与 diff", () => {
  it("展示变更文件，点击行调用 open_diff_window", async () => {
    mockWatcherAndDefaults();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_diff") {
        return Promise.resolve(
          "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-hello\n+hello2\n",
        );
      }
      if (cmd === "open_diff_window") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });

    const wrapper = mount(GitView, { props: { active: true } });
    await flushPromises();

    expect(wrapper.findAll(".git-file")).toHaveLength(2);
    expect(wrapper.find(".git-view").text()).toContain("修改");
    expect(wrapper.find(".git-view").text()).toContain("未跟踪");

    await wrapper.findAll(".git-file")[0].trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "open_diff_window",
    );
    expect(call).toBeTruthy();
    const params = (
      call?.[1] as {
        params: {
          path: string;
          kind: string;
          diff: string;
          workspace_root: string;
        };
      }
    ).params;
    expect(params.path).toBe("a.txt");
    expect(params.kind).toBe("modify");
    expect(params.workspace_root).toBe(rootPath);
    expect(params.diff).toContain("@@");
  });

  it("事件冷却：1s 内重复事件只触发一次刷新", async () => {
    mockWatcherAndDefaults();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      return Promise.resolve(undefined);
    });

    const wrapper = mount(GitView, { props: { active: true } });
    await flushPromises();

    const statusCalls = () =>
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "git_changes_status")
        .length;
    expect(statusCalls()).toBe(1); // 激活时首次刷新

    const changedCb = mockedListen.mock.calls.find(
      ([ev]) => ev === "git-changes/changed",
    )?.[1] as () => void;
    expect(changedCb).toBeTruthy();

    // 等待超过冷却窗口后触发事件 → 应刷新一次
    await new Promise((r) => setTimeout(r, 1200));
    changedCb();
    await flushPromises();
    expect(statusCalls()).toBe(2);

    // 冷却窗口内的重复事件 → 忽略，不刷新
    changedCb();
    await flushPromises();
    expect(statusCalls()).toBe(2);

    wrapper.unmount();
  });
});
