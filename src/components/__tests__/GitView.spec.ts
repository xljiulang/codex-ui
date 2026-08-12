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

    const wrapper = mountGitView({ props: { active: true } });
    await flushPromises();

    expect(wrapper.findAll(".git-file")).toHaveLength(2);
    for (const row of wrapper.findAll(".git-file")) {
      expect(row.attributes("data-tip")).toBeUndefined();
    }
    const modifiedIcon = wrapper.find(".git-status-icon.git-status-modified");
    const untrackedIcon = wrapper.find(".git-status-icon.git-status-untracked");
    expect(modifiedIcon.exists()).toBe(true);
    expect(modifiedIcon.attributes("data-tip")).toBe("修改");
    expect(untrackedIcon.exists()).toBe(true);
    expect(untrackedIcon.attributes("data-tip")).toBe("未跟踪");
    expect(untrackedIcon.find("svg path").attributes("d")).toBeTruthy();

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
    expect(
      items.filter((i) => i.find(".git-branch-delete").exists()),
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
