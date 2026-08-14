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
import { tooltipDirective } from "../../directives/tooltip";
import { settleConfirm, store } from "../../composables/useCodex";
import { __resetGitChangesForTest } from "../../composables/useGitChanges";
import {
  __resetEditorTabsForTest,
  tabs,
} from "../../composables/useEditorTabs";
import type { GitCommitEntry, GitStatus } from "../../lib/gitChanges";

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);
const rootPath = "D:\\codex\\demo";

const okStatus: GitStatus = {
  repoRoot: rootPath,
  branch: "main",
  files: [
    { path: "a.txt", status: "modified", staged: false, worktree: true },
    { path: "b.txt", status: "untracked", staged: false, worktree: true },
  ],
};

/** 挂载 GitView 并注入全局 tooltip 指令（v-tooltip） */
function mountGitView(options: Parameters<typeof mount>[1] = {}) {
  const merged = {
    ...options,
    global: {
      ...(options.global ?? {}),
      directives: {
        ...(options.global?.directives ?? {}),
        tooltip: tooltipDirective,
      },
    },
  } as Parameters<typeof mount>[1];
  return mount(GitView, merged);
}

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
  __resetEditorTabsForTest();
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

    const wrapper = mountGitView({ props: { active: true } });
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

  it("其它错误：显示错误信息与重试按钮", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.reject({ code: "repo_error", message: "boom" });
      }
      return Promise.resolve(undefined);
    });

    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    expect(wrapper.find(".git-view").text()).toContain("boom");
    expect(wrapper.find(".git-view").text()).toContain("重试");
  });
});

describe("GitView 文件列表与 diff", () => {
  it("展示变更文件与状态图标", async () => {
    mockWatcherAndDefaults();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_diff") {
        return Promise.resolve(
          "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-hello\n+hello2\n",
        );
      }
      if (cmd === "build_diff_preview") {
        return Promise.resolve([{ kind: "ctx", oldNo: 1, newNo: 1, text: "a" }]);
      }
      return Promise.resolve(undefined);
    });

    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    expect(wrapper.findAll(".git-file")).toHaveLength(2);
    for (const row of wrapper.findAll(".git-file")) {
      expect(row.attributes("data-tip")).toBeUndefined();
    }
    const modifiedIcon = wrapper.find(".git-status-icon.git-status-modified");
    const untrackedIcon = wrapper.find(".git-status-icon.git-status-untracked");
    expect(modifiedIcon.exists()).toBe(true);
    // 文件状态图标不再带 tooltip
    expect(modifiedIcon.attributes("data-tip")).toBeUndefined();
    expect(untrackedIcon.exists()).toBe(true);
    expect(untrackedIcon.attributes("data-tip")).toBeUndefined();
    expect(untrackedIcon.text().trim()).toBe("U");
  });

  it("单击文件行打开 diff 窗口", async () => {
    mockWatcherAndDefaults();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_diff") {
        return Promise.resolve(
          "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-hello\n+hello2\n",
        );
      }
      if (cmd === "build_diff_preview") {
        return Promise.resolve([{ kind: "ctx", oldNo: 1, newNo: 1, text: "a" }]);
      }
      return Promise.resolve(undefined);
    });

    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.findAll(".git-file")[0].trigger("click");
    await flushPromises();

    const diffCall = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_diff",
    );
    expect(diffCall).toBeTruthy();
    expect(diffCall?.[1]).toEqual({
      root: rootPath,
      path: "a.txt",
      kind: "modified",
    });

    const openCall = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "build_diff_preview",
    );
    expect(openCall).toBeTruthy();
    const params = (
      openCall?.[1] as {
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
    expect(
      tabs.some((t) => t.kind === "diff" && t.path === "a.txt"),
    ).toBe(true);
    wrapper.unmount();
  });

  it("事件冷却：1s 内重复事件只触发一次刷新", async () => {
    mockWatcherAndDefaults();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      return Promise.resolve(undefined);
    });

    const wrapper = mountGitView({ props: { active: true } });
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

describe("GitView 分支管理", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  function mockBranchRepo(current = "main") {
    const branches = ["dev", "feature", "main"];
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.resolve({ ...okStatus, branch: current });
      }
      if (cmd === "git_changes_branches") {
        return Promise.resolve({ current, branches });
      }
      if (cmd === "git_changes_branch_switch") {
        return Promise.resolve({ ...okStatus, branch: "dev" });
      }
      if (cmd === "git_changes_branch_create" || cmd === "git_changes_branch_delete") {
        return Promise.resolve({ ...okStatus, branch: current });
      }
      if (cmd === "git_changes_branch_merge") {
        return Promise.resolve({
          status: { ...okStatus, branch: current },
          kind: "merged",
          message: "已将分支 feature 合并到 main",
        });
      }
      if (cmd === "git_changes_log") {
        return Promise.resolve([]);
      }
      if (
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
  }

  it("点击分支按钮打开弹层，当前分支高亮且无删除按钮", async () => {
    mockBranchRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    // 分支按钮带 git 分支图标
    expect(wrapper.find(".git-branch-btn .git-branch-icon").exists()).toBe(true);
    await wrapper.find(".git-branch-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".git-branch-menu").exists()).toBe(true);
    // 弹层应为 .git-head 子节点（absolute 定位相对头部，修复被面板裁掉的问题）
    expect(wrapper.find(".git-head .git-branch-menu").exists()).toBe(true);
    const items = wrapper.findAll(".git-branch-menu-item");
    expect(items).toHaveLength(3);
    expect(items.map((i) => i.find(".git-branch-name").text())).toEqual([
      "dev",
      "feature",
      "main",
    ]);
    const current = wrapper.find(".git-branch-menu-item.current");
    expect(current.find(".git-branch-name").text()).toBe("main");
    expect(current.find(".git-branch-check").text()).toContain("✓");
    expect(current.find(".git-branch-delete").exists()).toBe(false);
    expect(current.find(".git-branch-merge").exists()).toBe(false);
    expect(
      items.filter((i) => i.find(".git-branch-delete").exists()),
    ).toHaveLength(2);
    expect(
      items.filter((i) => i.find(".git-branch-merge").exists()),
    ).toHaveLength(2);
    wrapper.unmount();
  });

  it("点击分支按钮上的箭头 SVG 也能打开并保持菜单", async () => {
    mockBranchRepo();
    const wrapper = mountGitView({
      props: { active: true },
      attachTo: document.body,
    });
    await flushPromises();
    // SVG 目标是 Element 而非 HTMLElement，不应被当作“外部点击”关闭
    await wrapper.find(".git-branch-btn svg").trigger("click");
    await flushPromises();
    expect(wrapper.find(".git-branch-menu").exists()).toBe(true);
    wrapper.unmount();
  });

  it("点击分支行调用切换并更新状态、关闭弹层", async () => {
    mockBranchRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-branch-btn").trigger("click");
    await flushPromises();

    const dev = wrapper
      .findAll(".git-branch-menu-item")
      .find((i) => i.find(".git-branch-name").text() === "dev")!;
    await dev.trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_branch_switch",
    );
    expect(call).toBeTruthy();
    expect((call?.[1] as { path: string; name: string }).name).toBe("dev");
    expect(wrapper.find(".git-branch-menu").exists()).toBe(false);
    expect(wrapper.find(".git-branch-btn").text()).toContain("dev");
    wrapper.unmount();
  });

  it("新建分支调用创建并刷新分支列表", async () => {
    mockBranchRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-branch-btn").trigger("click");
    await flushPromises();

    await wrapper.find(".git-branch-input").setValue("hotfix");
    await wrapper.find(".git-branch-create-btn").trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_branch_create",
    );
    expect(call).toBeTruthy();
    expect((call?.[1] as { path: string; name: string }).name).toBe("hotfix");
    expect(
      (wrapper.find(".git-branch-input").element as HTMLInputElement).value,
    ).toBe("");
    // 创建后重新拉取列表（打开弹层 1 次 + 创建后 1 次）
    expect(
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "git_changes_branches")
        .length,
    ).toBeGreaterThanOrEqual(2);
    wrapper.unmount();
  });

  it("删除分支调用删除并更新列表", async () => {
    mockBranchRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-branch-btn").trigger("click");
    await flushPromises();

    const feature = wrapper
      .findAll(".git-branch-menu-item")
      .find((i) => i.find(".git-branch-name").text() === "feature")!;
    await feature.find(".git-branch-delete").trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_branch_delete",
    );
    expect(call).toBeTruthy();
    expect((call?.[1] as { path: string; name: string }).name).toBe("feature");
    const names = wrapper.findAll(".git-branch-name").map((n) => n.text());
    expect(names).not.toContain("feature");
    wrapper.unmount();
  });

  it("合并分支调用合并接口、提示结果并关闭弹层", async () => {
    mockBranchRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-branch-btn").trigger("click");
    await flushPromises();

    const feature = wrapper
      .findAll(".git-branch-menu-item")
      .find((i) => i.find(".git-branch-name").text() === "feature")!;
    // 合并按钮位于删除按钮之前
    const mergeBtn = feature.find(".git-branch-merge");
    expect(mergeBtn.exists()).toBe(true);
    const deleteBtn = feature.find(".git-branch-delete");
    expect(deleteBtn.exists()).toBe(true);
    expect(
      mergeBtn.element.compareDocumentPosition(deleteBtn.element) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    await mergeBtn.trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_branch_merge",
    );
    expect(call).toBeTruthy();
    expect((call?.[1] as { path: string; name: string }).name).toBe("feature");
    expect(store.toast).toContain("已将分支 feature 合并到 main");
    expect(wrapper.find(".git-branch-menu").exists()).toBe(false);
    wrapper.unmount();
  });

  it("外部点击与 Escape 关闭弹层", async () => {
    mockBranchRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-branch-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".git-branch-menu").exists()).toBe(true);

    window.dispatchEvent(new MouseEvent("click"));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".git-branch-menu").exists()).toBe(false);

    await wrapper.find(".git-branch-btn").trigger("click");
    await flushPromises();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".git-branch-menu").exists()).toBe(false);
    wrapper.unmount();
  });

  it("外部滚动不关闭弹层，面板内滚动关闭弹层", async () => {
    mockBranchRepo();
    const wrapper = mountGitView({
      props: { active: true },
      attachTo: document.body,
    });
    await flushPromises();
    await wrapper.find(".git-branch-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".git-branch-menu").exists()).toBe(true);

    const chatEl = document.createElement("div");
    chatEl.className = "chat-scroll";
    document.body.appendChild(chatEl);
    chatEl.dispatchEvent(new Event("scroll"));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".git-branch-menu").exists()).toBe(true);
    chatEl.remove();

    wrapper.find(".git-file-list").element.dispatchEvent(new Event("scroll"));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".git-branch-menu").exists()).toBe(false);
    wrapper.unmount();
  });
});

describe("GitView 提交历史", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  function mockRepoWithLog(entries: GitCommitEntry[]) {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_log") return Promise.resolve(entries);
      if (
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
  }

  /** 分页 mock：按 before 游标返回每批 50 条，记录每次调用的游标参数 */
  function mockPagedLog(total: number) {
    const all: GitCommitEntry[] = Array.from({ length: total }, (_, i) => ({
      hash: String(i).padStart(40, "0"),
      shortHash: String(i).padStart(7, "0"),
      subject: `commit ${total - 1 - i}`,
      author: "tester",
      timeSecs: 1700000000 + i,
    }));
    const logCalls: Array<{ before: string | null }> = [];
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_log") {
        const before =
          (args as unknown as { before?: string | null } | undefined)
            ?.before ?? null;
        logCalls.push({ before });
        const start =
          before == null ? 0 : all.findIndex((e) => e.hash === before) + 1;
        return Promise.resolve(all.slice(start, start + 50));
      }
      if (
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    return { all, logCalls };
  }

  it("展示提交历史（主题/作者/时间/短哈希），位于暂存区之后", async () => {
    mockRepoWithLog([
      {
        hash: "a".repeat(40),
        shortHash: "aaaaaaa",
        subject: "feat: 初始化",
        author: "tester",
        timeSecs: 1700000000,
      },
      {
        hash: "b".repeat(40),
        shortHash: "bbbbbbb",
        subject: "fix: bug",
        author: "tester",
        timeSecs: 1700003600,
      },
    ]);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    // 提交历史区位于暂存区之后
    const heads = wrapper.findAll(".git-section-head");
    expect(heads.map((h) => h.find("span").text())).toEqual([
      "更改",
      "暂存更改",
      "提交历史",
    ]);
    // 历史区默认折叠，先展开再断言内容
    expect(wrapper.findAll(".git-section")[2].classes()).toContain("collapsed");
    await wrapper
      .findAll(".git-section")[2]
      .find(".git-section-head")
      .trigger("click");
    await wrapper.vm.$nextTick();
    // 冗余的「刷新提交历史」按钮已移除
    expect(wrapper.find(".git-section-log").exists()).toBe(false);
    const items = wrapper.findAll(".git-log-item");
    expect(items).toHaveLength(2);
    expect(items[0].find(".git-log-subject").text()).toBe("feat: 初始化");
    expect(items[0].find(".git-log-meta").text()).toContain("tester");
    expect(items[0].find(".git-log-meta").text()).toContain("2023-");
    expect(items[0].find(".git-log-hash").text()).toBe("aaaaaaa");
    expect(items[0].find(".git-log-hash").attributes("title")).toBe(
      "a".repeat(40),
    );
    expect(items[1].find(".git-log-subject").text()).toBe("fix: bug");
    wrapper.unmount();
  });

  it("无提交时显示空状态", async () => {
    mockRepoWithLog([]);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    // 历史区默认折叠，先展开
    await wrapper
      .findAll(".git-section")[2]
      .find(".git-section-head")
      .trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".git-view").text()).toContain("暂无提交记录");
    wrapper.unmount();
  });

  it("首批 50 条显示加载更多，点击追加第二批并携带 before 游标", async () => {
    const { all, logCalls } = mockPagedLog(120);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper
      .findAll(".git-section")[2]
      .find(".git-section-head")
      .trigger("click");
    await wrapper.vm.$nextTick();

    expect(wrapper.findAll(".git-log-item")).toHaveLength(50);
    expect(wrapper.find(".git-log-more").exists()).toBe(true);
    expect(wrapper.find(".git-log-more").attributes("aria-label")).toBe("加载更多");
    expect(wrapper.find(".git-log-more").attributes("data-tip")).toBe("加载更多");
    expect(logCalls.some((c) => c.before === null)).toBe(true);

    await wrapper.find(".git-log-more").trigger("click");
    await flushPromises();

    expect(wrapper.findAll(".git-log-item")).toHaveLength(100);
    expect(logCalls.some((c) => c.before === all[49].hash)).toBe(true);
    expect(wrapper.find(".git-log-more").attributes("aria-label")).toBe("加载更多");
    wrapper.unmount();
  });

  it("返回不足 50 条时加载更多按钮消失", async () => {
    const { all, logCalls } = mockPagedLog(60);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper
      .findAll(".git-section")[2]
      .find(".git-section-head")
      .trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".git-log-more").exists()).toBe(true);
    expect(logCalls.some((c) => c.before === null)).toBe(true);

    await wrapper.find(".git-log-more").trigger("click");
    await flushPromises();

    expect(wrapper.findAll(".git-log-item")).toHaveLength(60);
    expect(logCalls.some((c) => c.before === all[49].hash)).toBe(true);
    expect(wrapper.find(".git-log-more").exists()).toBe(false);
    wrapper.unmount();
  });

  it("空批时按钮消失且列表保持不变", async () => {
    const { all, logCalls } = mockPagedLog(100);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper
      .findAll(".git-section")[2]
      .find(".git-section-head")
      .trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".git-log-item")).toHaveLength(50);

    await wrapper.find(".git-log-more").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".git-log-item")).toHaveLength(100);
    expect(wrapper.find(".git-log-more").exists()).toBe(true);

    await wrapper.find(".git-log-more").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".git-log-item")).toHaveLength(100);
    expect(wrapper.find(".git-log-more").exists()).toBe(false);
    expect(logCalls.some((c) => c.before === all[99].hash)).toBe(true);
    wrapper.unmount();
  });
});

describe("GitView 分区折叠", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.toast = "";
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  function mockRepo(status: GitStatus, commits: GitCommitEntry[] = []) {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(status);
      if (cmd === "git_changes_log") return Promise.resolve(commits);
      if (
        cmd === "git_changes_stage_all" ||
        cmd === "git_changes_unstage_all" ||
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(status);
      }
      return Promise.resolve(undefined);
    });
  }

  it("更改/暂存默认展开，提交历史默认折叠，点击标题可切换", async () => {
    mockRepo(okStatus, [
      {
        hash: "a".repeat(40),
        shortHash: "aaaaaaa",
        subject: "feat: init",
        author: "t",
        timeSecs: 1700000000,
      },
    ]);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const sections = wrapper.findAll(".git-section");
    expect(sections).toHaveLength(3);
    // 更改/暂存默认展开
    expect(sections[0].find(".git-file-list").exists()).toBe(true);
    expect(sections[1].find(".git-commit-bar").exists()).toBe(true);
    expect(
      sections[0].find(".git-section-head").attributes("aria-expanded"),
    ).toBe("true");
    // 提交历史默认折叠
    expect(sections[2].find(".git-log-list").exists()).toBe(false);
    expect(sections[2].classes()).toContain("collapsed");
    expect(
      sections[2].find(".git-section-head").attributes("aria-expanded"),
    ).toBe("false");

    // 展开「提交历史」区
    await wrapper
      .findAll(".git-section")[2]
      .find(".git-section-head")
      .trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".git-section")[2].find(".git-log-list").exists()).toBe(
      true,
    );
    expect(wrapper.findAll(".git-section")[2].classes()).not.toContain(
      "collapsed",
    );

    // 折叠「更改」区
    await wrapper
      .findAll(".git-section")[0]
      .find(".git-section-head")
      .trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".git-section")[0].find(".git-file-list").exists()).toBe(
      false,
    );
    expect(
      wrapper
        .findAll(".git-section")[0]
        .find(".git-section-head")
        .attributes("aria-expanded"),
    ).toBe("false");
    expect(wrapper.findAll(".git-section")[0].classes()).toContain("collapsed");

    // 再次点击展开「更改」区
    await wrapper
      .findAll(".git-section")[0]
      .find(".git-section-head")
      .trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".git-section")[0].find(".git-file-list").exists()).toBe(
      true,
    );
    expect(wrapper.findAll(".git-section")[0].classes()).not.toContain("collapsed");
    wrapper.unmount();
  });

  it("折叠状态跨刷新保留", async () => {
    mockRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-section-head").trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".git-section")[0].find(".git-file-list").exists()).toBe(
      false,
    );

    // 点刷新按钮触发状态刷新，折叠状态应保留
    await wrapper.find(".git-refresh").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".git-section")[0].find(".git-file-list").exists()).toBe(
      false,
    );
    expect(wrapper.findAll(".git-section")[0].classes()).toContain("collapsed");
    wrapper.unmount();
  });

  it("点击分区内动作按钮不触发折叠", async () => {
    mockRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper
      .findAll(".git-section")[0]
      .find(".git-section-action")
      .trigger("click");
    await flushPromises();
    // 仍为展开状态，且操作正常执行
    expect(wrapper.findAll(".git-section")[0].find(".git-file-list").exists()).toBe(
      true,
    );
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_stage_all",
    );
    expect(call).toBeTruthy();
    wrapper.unmount();
  });
});

describe("GitView 变更文件右键菜单", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.confirm = null;
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  function mockFileRepo(status: GitStatus) {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(status);
      if (
        cmd === "git_changes_stage" ||
        cmd === "git_changes_unstage" ||
        cmd === "git_changes_restore" ||
        cmd === "git_changes_delete" ||
        cmd === "git_changes_ignore" ||
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(status);
      }
      return Promise.resolve(undefined);
    });
  }

  function menuLabels(wrapper: ReturnType<typeof mountGitView>) {
    return wrapper.findAll(".ctx-menu-item").map((i) => i.text());
  }

  it("更改区 modified：显示打开/暂存/撤消更改，无取消暂存", async () => {
    mockFileRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual(["打开", "暂存", "撤消更改"]);
    expect(wrapper.find(".ctx-menu-item.danger").text()).toBe("撤消更改");
    wrapper.unmount();
  });

  it("暂存更改区 modified：显示取消暂存，无暂存", async () => {
    mockFileRepo({
      ...okStatus,
      files: [
        { path: "a.txt", status: "modified", staged: true, worktree: false },
      ],
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual(["打开", "取消暂存", "撤消更改"]);
    wrapper.unmount();
  });

  it("untracked：显示暂存/忽略此本地项/删除文件（danger）", async () => {
    mockFileRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.findAll(".git-file")[1].trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual([
      "打开",
      "暂存",
      "忽略此本地项",
      "删除文件",
    ]);
    expect(wrapper.find(".ctx-menu-item.danger").text()).toBe("删除文件");
    wrapper.unmount();
  });

  it("added：显示取消暂存与删除文件", async () => {
    mockFileRepo({
      ...okStatus,
      files: [
        { path: "new.txt", status: "added", staged: true, worktree: false },
      ],
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual(["打开", "取消暂存", "删除文件"]);
    wrapper.unmount();
  });

  it("点击打开调用 build_diff_preview 并关闭菜单", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_diff") {
        return Promise.resolve(
          "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-hello\n+hello2\n",
        );
      }
      if (cmd === "build_diff_preview") {
        return Promise.resolve([{ kind: "ctx", oldNo: 1, newNo: 1, text: "a" }]);
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "打开")!
      .trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "build_diff_preview",
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
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    wrapper.unmount();
  });

  it("点击暂存调用 git_changes_stage 并关闭菜单", async () => {
    mockFileRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "暂存")!
      .trigger("click");
    await flushPromises();
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_stage",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, path: "a.txt" });
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    wrapper.unmount();
  });

  it("点击取消暂存调用 git_changes_unstage", async () => {
    mockFileRepo({
      ...okStatus,
      files: [
        { path: "a.txt", status: "modified", staged: true, worktree: false },
      ],
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "取消暂存")!
      .trigger("click");
    await flushPromises();
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_unstage",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, path: "a.txt" });
    wrapper.unmount();
  });

  it("点击忽略此本地项调用 git_changes_ignore", async () => {
    mockFileRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.findAll(".git-file")[1].trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "忽略此本地项")!
      .trigger("click");
    await flushPromises();
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_ignore",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, path: "b.txt" });
    wrapper.unmount();
  });

  it("撤消更改需要确认，取消不调用命令", async () => {
    mockFileRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "撤消更改")!
      .trigger("click");
    await flushPromises();
    expect(store.confirm?.title).toBe("撤消更改");
    settleConfirm(false);
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.some(([cmd]) => cmd === "git_changes_restore"),
    ).toBe(false);
    wrapper.unmount();
  });

  it("撤消更改确认后调用 git_changes_restore", async () => {
    mockFileRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "撤消更改")!
      .trigger("click");
    await flushPromises();
    settleConfirm(true);
    await flushPromises();
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_restore",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, path: "a.txt" });
    wrapper.unmount();
  });

  it("删除文件确认后调用 git_changes_delete", async () => {
    mockFileRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.findAll(".git-file")[1].trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "删除文件")!
      .trigger("click");
    await flushPromises();
    expect(store.confirm?.title).toBe("删除文件");
    settleConfirm(true);
    await flushPromises();
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_delete",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, path: "b.txt" });
    wrapper.unmount();
  });

  it("外部点击与 Escape 关闭右键菜单", async () => {
    mockFileRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await wrapper.find(".git-file").trigger("contextmenu");
    expect(wrapper.find(".ctx-menu").exists()).toBe(true);

    window.dispatchEvent(new MouseEvent("click"));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);

    await wrapper.find(".git-file").trigger("contextmenu");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    wrapper.unmount();
  });
});

describe("GitView 变更文件树形目录", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.confirm = null;
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  const treeStatus: GitStatus = {
    repoRoot: rootPath,
    branch: "main",
    files: [
      { path: "a.txt", status: "modified", staged: false, worktree: true },
      { path: "src/b.txt", status: "untracked", staged: false, worktree: true },
      {
        path: "src/deep/c.txt",
        status: "modified",
        staged: true,
        worktree: false,
      },
      { path: "src2/d.txt", status: "untracked", staged: false, worktree: true },
    ],
  };

  function mockTreeRepo() {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(treeStatus);
      if (
        cmd === "git_changes_stage" ||
        cmd === "git_changes_unstage" ||
        cmd === "git_changes_restore" ||
        cmd === "git_changes_ignore" ||
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(treeStatus);
      }
      return Promise.resolve(undefined);
    });
  }

  function menuLabels(wrapper: ReturnType<typeof mountGitView>) {
    return wrapper.findAll(".ctx-menu-item").map((i) => i.text());
  }

  function dirRow(wrapper: ReturnType<typeof mountGitView>, name: string) {
    return wrapper
      .findAll(".git-dir")
      .find((r) => r.find(".git-dir-name").text() === name)!;
  }

  function dirRows(wrapper: ReturnType<typeof mountGitView>, name: string) {
    return wrapper
      .findAll(".git-dir")
      .filter((r) => r.find(".git-dir-name").text() === name);
  }

  it("渲染更改/暂存更改两个分区并分别构建树", async () => {
    mockTreeRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const heads = wrapper.findAll(".git-section-head");
    expect(heads).toHaveLength(3);
    expect(heads[0].find("span").text()).toBe("更改");
    expect(heads[1].find("span").text()).toBe("暂存更改");
    expect(heads[2].find("span").text()).toBe("提交历史");
    // 分区徽章：更改总数 4（每个文件计一次），暂存数 1
    expect(heads[0].find(".git-section-count").text()).toBe("4");
    expect(heads[1].find(".git-section-count").text()).toBe("1");
    // 徽章位于 .git-section-actions 内、排在操作按钮之前
    const actions0 = heads[0].find(".git-section-actions");
    expect(actions0.exists()).toBe(true);
    expect(
      actions0.element.children[0].classList.contains("git-section-count"),
    ).toBe(true);
    expect(
      actions0.element.children[1].classList.contains("git-section-stage"),
    ).toBe(true);
    const actions1 = heads[1].find(".git-section-actions");
    expect(actions1.exists()).toBe(true);
    expect(
      actions1.element.children[0].classList.contains("git-section-count"),
    ).toBe(true);
    expect(
      actions1.element.children[1].classList.contains("git-section-unstage"),
    ).toBe(true);

    // 更改区：src(1) → b.txt → src2(1) → d.txt → 根文件 a.txt
    const sections = wrapper.findAll(".git-section");
    const changesRows = sections[0].findAll(".git-tree-row");
    expect(changesRows).toHaveLength(5);
    expect(changesRows[0].find(".git-dir-name").text()).toBe("src");
    expect(changesRows[1].text()).toContain("src/b.txt");
    expect(changesRows[2].find(".git-dir-name").text()).toBe("src2");
    expect(changesRows[4].text()).toContain("a.txt");

    // 暂存更改区：src → deep → c.txt
    const stagedRows = sections[1].findAll(".git-tree-row");
    expect(stagedRows).toHaveLength(3);
    expect(stagedRows[0].find(".git-dir-name").text()).toBe("src");
    expect(stagedRows[1].find(".git-dir-name").text()).toBe("deep");
    expect(stagedRows[2].text()).toContain("src/deep/c.txt");
    wrapper.unmount();
  });

  it("0 个更改时分区徽章不渲染", async () => {
    mockTreeRepo();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.resolve({ ...treeStatus, files: [] });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    expect(wrapper.find(".git-section-count").exists()).toBe(false);
    // 操作按钮仍在 actions 容器内，保持右对齐结构
    const actions = wrapper.find(".git-section-actions");
    expect(actions.exists()).toBe(true);
    expect(actions.find(".git-section-stage").exists()).toBe(true);
    wrapper.unmount();
  });

  it("空分区显示占位文案", async () => {
    mockTreeRepo();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.resolve({ ...treeStatus, files: [] });
      }
      if (
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    const text = wrapper.find(".git-view").text();
    expect(text).toContain("无更改");
    expect(text).toContain("无暂存更改");
    wrapper.unmount();
  });

  it("同时有暂存+工作区更改的文件出现在两个分区，菜单按分区区分", async () => {
    mockTreeRepo();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.resolve({
          ...treeStatus,
          files: [
            {
              path: "src/deep/c.txt",
              status: "modified",
              staged: true,
              worktree: true,
            },
          ],
        });
      }
      if (
        cmd === "git_changes_stage" ||
        cmd === "git_changes_unstage" ||
        cmd === "git_changes_restore" ||
        cmd === "git_changes_ignore" ||
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(treeStatus);
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const sections = wrapper.findAll(".git-section");
    expect(sections[0].text()).toContain("src/deep/c.txt");
    expect(sections[1].text()).toContain("src/deep/c.txt");

    await sections[0].find(".git-file").trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual(["打开", "暂存", "撤消更改"]);

    window.dispatchEvent(new MouseEvent("click"));
    await wrapper.vm.$nextTick();
    await sections[1].find(".git-file").trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual(["打开", "取消暂存", "撤消更改"]);
    wrapper.unmount();
  });

  it("点击目录行折叠/展开子项，折叠状态跨分区共享", async () => {
    mockTreeRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    expect(wrapper.findAll(".git-tree-row")).toHaveLength(8);

    await dirRow(wrapper, "src").trigger("click");
    await wrapper.vm.$nextTick();
    // 更改区 src 收起（隐藏 b.txt），暂存区 src 同步收起（隐藏 deep/c.txt）
    expect(wrapper.findAll(".git-tree-row")).toHaveLength(5);

    await dirRow(wrapper, "src").trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".git-tree-row")).toHaveLength(8);
    wrapper.unmount();
  });

  it("目录菜单按分区显隐：更改区=暂存/忽略此本地项/撤消更改", async () => {
    mockTreeRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await dirRow(wrapper, "src").trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual([
      "暂存",
      "忽略此本地项",
      "撤消更改",
    ]);
    expect(wrapper.find(".ctx-menu-item.danger").text()).toBe("撤消更改");

    window.dispatchEvent(new MouseEvent("click"));
    await wrapper.vm.$nextTick();
    await dirRow(wrapper, "src2").trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual(["暂存", "忽略此本地项", "撤消更改"]);
    wrapper.unmount();
  });

  it("暂存更改区目录菜单：取消暂存/撤消更改", async () => {
    mockTreeRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await dirRows(wrapper, "src")[1].trigger("contextmenu");
    expect(menuLabels(wrapper)).toEqual(["取消暂存", "撤消更改"]);
    wrapper.unmount();
  });

  it("更改区目录“暂存”调用 git_changes_stage（路径为目录）", async () => {
    mockTreeRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await dirRow(wrapper, "src2").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "暂存")!
      .trigger("click");
    await flushPromises();
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_stage",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, path: "src2" });
    wrapper.unmount();
  });

  it("目录撤消更改需确认：取消不调用，确认后调用 git_changes_restore", async () => {
    mockTreeRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await dirRow(wrapper, "src").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "撤消更改")!
      .trigger("click");
    await flushPromises();
    expect(store.confirm?.title).toBe("撤消更改");
    settleConfirm(false);
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.some(([cmd]) => cmd === "git_changes_restore"),
    ).toBe(false);

    await dirRow(wrapper, "src").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "撤消更改")!
      .trigger("click");
    await flushPromises();
    settleConfirm(true);
    await flushPromises();
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_restore",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, path: "src" });
    wrapper.unmount();
  });

  it("目录“忽略此本地项”调用 git_changes_ignore", async () => {
    mockTreeRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await dirRow(wrapper, "src").trigger("contextmenu");
    await wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text() === "忽略此本地项")!
      .trigger("click");
    await flushPromises();
    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_ignore",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, path: "src" });
    wrapper.unmount();
  });

  it("刷新后保留手动折叠状态", async () => {
    mockTreeRepo();
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    await dirRow(wrapper, "src").trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".git-tree-row")).toHaveLength(5);

    await wrapper.find(".git-refresh").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".git-tree-row")).toHaveLength(5);
    expect(wrapper.find(".git-view").text()).not.toContain("src/deep/c.txt");
    wrapper.unmount();
  });
});

describe("GitView 提交", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.confirm = null;
    store.toast = "";
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  const stagedStatus: GitStatus = {
    repoRoot: rootPath,
    branch: "main",
    files: [
      { path: "a.txt", status: "modified", staged: true, worktree: false },
      { path: "b.txt", status: "added", staged: true, worktree: false },
    ],
  };

  it("有暂存时提示文件数；消息为空时提交按钮禁用", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(stagedStatus);
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    expect(wrapper.find(".git-commit-hint").text()).toBe("将提交 2 个文件");
    expect(
      (wrapper.find(".git-commit-btn").element as HTMLButtonElement).disabled,
    ).toBe(true);
    wrapper.unmount();
  });

  it("填写消息后提交：调用 git_changes_commit、清空消息并 toast", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(stagedStatus);
      if (cmd === "git_changes_commit") {
        return Promise.resolve({ ...stagedStatus, files: [] });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-commit-input").setValue("feat: 提交 a");
    await wrapper.find(".git-commit-btn").trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_commit",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath, message: "feat: 提交 a" });
    expect(
      (wrapper.find(".git-commit-input").element as HTMLTextAreaElement).value,
    ).toBe("");
    expect(store.toast).toBe("提交成功");
    wrapper.unmount();
  });

  it("Ctrl+Enter 提交", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(stagedStatus);
      if (cmd === "git_changes_commit") {
        return Promise.resolve({ ...stagedStatus, files: [] });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-commit-input").setValue("feat: x");
    await wrapper.find(".git-commit-input").trigger("keydown.ctrl.enter");
    await flushPromises();

    expect(
      mockedInvoke.mock.calls.some(([cmd]) => cmd === "git_changes_commit"),
    ).toBe(true);
    wrapper.unmount();
  });

  it("无暂存更改时提示先暂存，提交按钮禁用", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    expect(wrapper.find(".git-commit-hint").text()).toBe("先在上方暂存更改");
    await wrapper.find(".git-commit-input").setValue("msg");
    expect(
      (wrapper.find(".git-commit-btn").element as HTMLButtonElement).disabled,
    ).toBe(true);
    wrapper.unmount();
  });

  it("提交失败时 toast 展示后端错误", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(stagedStatus);
      if (cmd === "git_changes_commit") {
        return Promise.reject({ message: "没有已暂存的更改" });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-commit-input").setValue("msg");
    await wrapper.find(".git-commit-btn").trigger("click");
    await flushPromises();
    expect(store.toast).toBe("没有已暂存的更改");
    wrapper.unmount();
  });
});

describe("GitView 拉取", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.confirm = null;
    store.toast = "";
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  it("拉取/推送合并为胶囊：容器包含两按钮与分隔线", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_git_available") return Promise.resolve(true);
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();
    const capsule = wrapper.find(".git-pull-push");
    expect(capsule.exists()).toBe(true);
    expect(capsule.find(".git-pull").exists()).toBe(true);
    expect(capsule.find(".git-push").exists()).toBe(true);
    expect(capsule.find(".git-pull-push-divider").exists()).toBe(true);
    wrapper.unmount();
  });

  it("点击拉取调用 git_changes_pull 并更新状态与 toast", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_git_available") return Promise.resolve(true);
      if (cmd === "git_changes_pull") {
        return Promise.resolve({
          status: okStatus,
          kind: "fast_forward",
          message: "已快进更新到远端 origin/main",
        });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-pull").trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_pull",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath });
    expect(store.toast).toBe("已快进更新到远端 origin/main");
    wrapper.unmount();
  });

  it("拉取失败时 toast 展示后端错误", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_git_available") return Promise.resolve(true);
      if (cmd === "git_changes_pull") {
        return Promise.reject({ message: "连接远端失败" });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-pull").trigger("click");
    await flushPromises();
    expect(store.toast).toBe("连接远端失败");
    wrapper.unmount();
  });

  it("游离 HEAD 时拉取按钮禁用", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.resolve({ ...okStatus, branch: "HEAD" });
      }
      if (cmd === "git_changes_git_available") return Promise.resolve(true);
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    expect(
      (wrapper.find(".git-pull").element as HTMLButtonElement).disabled,
    ).toBe(true);
    wrapper.unmount();
  });

  it("本机未安装 git 时拉取按钮禁用并提示", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_git_available") return Promise.resolve(false);
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const pullBtn = wrapper.find(".git-pull");
    expect((pullBtn.element as HTMLButtonElement).disabled).toBe(true);
    expect(pullBtn.attributes("data-tip")).toBe("未检测到 git，无法拉取");
    wrapper.unmount();
  });
});

describe("GitView 推送", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.confirm = null;
    store.toast = "";
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  it("点击推送调用 git_changes_push 并更新状态与 toast", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_git_available") return Promise.resolve(true);
      if (cmd === "git_changes_push") {
        return Promise.resolve({
          status: okStatus,
          kind: "pushed",
          message: "已推送到 origin/main",
        });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-push").trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_push",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath });
    expect(store.toast).toBe("已推送到 origin/main");
    wrapper.unmount();
  });

  it("推送失败时 toast 展示后端错误", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_git_available") return Promise.resolve(true);
      if (cmd === "git_changes_push") {
        return Promise.reject({ message: "推送被拒绝：远端领先本地" });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-push").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("推送被拒绝");
    wrapper.unmount();
  });

  it("游离 HEAD 时推送按钮禁用", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.resolve({ ...okStatus, branch: "HEAD" });
      }
      if (cmd === "git_changes_git_available") return Promise.resolve(true);
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    expect(
      (wrapper.find(".git-push").element as HTMLButtonElement).disabled,
    ).toBe(true);
    wrapper.unmount();
  });

  it("本机未安装 git 时推送按钮禁用并提示", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_git_available") return Promise.resolve(false);
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const pushBtn = wrapper.find(".git-push");
    expect((pushBtn.element as HTMLButtonElement).disabled).toBe(true);
    expect(pushBtn.attributes("data-tip")).toBe("未检测到 git，无法推送");
    wrapper.unmount();
  });

  it("拉取忙碌时推送按钮禁用（互斥）", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(okStatus);
      if (cmd === "git_changes_git_available") return Promise.resolve(true);
      if (cmd === "git_changes_pull") return new Promise(() => {});
      return Promise.resolve(undefined);
    });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    await wrapper.find(".git-pull").trigger("click");
    expect(
      (wrapper.find(".git-push").element as HTMLButtonElement).disabled,
    ).toBe(true);
    wrapper.unmount();
  });
});

describe("GitView 状态字母与全部暂存/取消暂存", () => {
  beforeEach(() => {
    store.threads = [];
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.confirm = null;
    store.toast = "";
    mockedInvoke.mockClear();
    __resetGitChangesForTest();
  });

  const letterStatus: GitStatus = {
    repoRoot: rootPath,
    branch: "main",
    files: [
      { path: "added.txt", status: "added", staged: true, worktree: false },
      { path: "mod.txt", status: "modified", staged: true, worktree: false },
      { path: "del.txt", status: "deleted", staged: true, worktree: false },
      { path: "ren.txt", status: "renamed", staged: true, worktree: false },
      { path: "unt.txt", status: "untracked", staged: false, worktree: true },
      { path: "conf.txt", status: "conflicted", staged: true, worktree: false },
    ],
  };

  const stagedStatus: GitStatus = {
    repoRoot: rootPath,
    branch: "main",
    files: [
      { path: "a.txt", status: "modified", staged: true, worktree: false },
      { path: "b.txt", status: "added", staged: true, worktree: false },
    ],
  };

  function mockRepo(status: GitStatus) {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") return Promise.resolve(status);
      if (
        cmd === "git_changes_stage_all" ||
        cmd === "git_changes_unstage_all" ||
        cmd === "git_changes_watch_start" ||
        cmd === "git_changes_watch_stop"
      ) {
        return Promise.resolve(status);
      }
      return Promise.resolve(undefined);
    });
  }

  it("各状态渲染对应字母徽标，删除行路径加删除线", async () => {
    mockRepo(letterStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const letterByPath: Record<string, string> = {};
    for (const row of wrapper.findAll(".git-file")) {
      const path = row.find(".git-path").text();
      letterByPath[path] = row.find(".git-status-icon").text().trim();
    }
    expect(letterByPath["added.txt"]).toBe("A");
    expect(letterByPath["mod.txt"]).toBe("M");
    expect(letterByPath["del.txt"]).toBe("D");
    expect(letterByPath["ren.txt"]).toBe("R");
    expect(letterByPath["unt.txt"]).toBe("U");
    expect(letterByPath["conf.txt"]).toBe("C");

    const deletedRow = wrapper
      .findAll(".git-file")
      .find((r) => r.find(".git-path").text() === "del.txt")!;
    expect(deletedRow.find(".git-path").classes()).toContain(
      "git-path-strike",
    );
    const modifiedRow = wrapper
      .findAll(".git-file")
      .find((r) => r.find(".git-path").text() === "mod.txt")!;
    expect(modifiedRow.find(".git-path").classes()).not.toContain(
      "git-path-strike",
    );
    wrapper.unmount();
  });

  it("更改区标题向下按钮点击调用 git_changes_stage_all", async () => {
    mockRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const sections = wrapper.findAll(".git-section");
    const stageBtn = sections[0].find(".git-section-action");
    expect(stageBtn.exists()).toBe(true);
    expect(stageBtn.attributes("aria-label")).toBe("全部暂存");
    await stageBtn.trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_stage_all",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath });
    wrapper.unmount();
  });

  it("暂存更改区标题向上按钮点击调用 git_changes_unstage_all", async () => {
    mockRepo(stagedStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const sections = wrapper.findAll(".git-section");
    const unstageBtn = sections[1].find(".git-section-action");
    expect(unstageBtn.exists()).toBe(true);
    expect(unstageBtn.attributes("aria-label")).toBe("全部取消暂存");
    await unstageBtn.trigger("click");
    await flushPromises();

    const call = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_unstage_all",
    );
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ root: rootPath });
    wrapper.unmount();
  });

  it("对应分区无文件时按钮禁用", async () => {
    mockRepo({ ...okStatus, files: [] });
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const sections = wrapper.findAll(".git-section");
    for (const s of sections.slice(0, 2)) {
      const btn = s.find(".git-section-action");
      expect(btn.exists()).toBe(true);
      expect((btn.element as HTMLButtonElement).disabled).toBe(true);
    }
    wrapper.unmount();
  });

  it("暂存区为空时向上按钮禁用，更改区有文件时向下按钮可用", async () => {
    mockRepo(okStatus);
    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    const sections = wrapper.findAll(".git-section");
    const stageBtn = sections[0].find(".git-section-action");
    const unstageBtn = sections[1].find(".git-section-action");
    expect(stageBtn.exists()).toBe(true);
    expect(unstageBtn.exists()).toBe(true);
    expect((stageBtn.element as HTMLButtonElement).disabled).toBe(false);
    expect((unstageBtn.element as HTMLButtonElement).disabled).toBe(true);
    wrapper.unmount();
  });
});
