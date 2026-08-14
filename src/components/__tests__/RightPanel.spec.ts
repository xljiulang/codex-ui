import { describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    deleteThread: vi.fn(),
    togglePin: vi.fn(),
    refreshThreads: vi.fn(),
    searchThreads: vi.fn(),
  };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import RightPanel from "../RightPanel.vue";
import ResourceView from "../ResourceView.vue";
import GitView from "../GitView.vue";
import { store } from "../../composables/useCodex";
import { __resetSessionFsForTest } from "../../composables/useSessionFs";
import { __resetGitChangesForTest } from "../../composables/useGitChanges";

const mockedInvoke = vi.mocked(invoke);
const rootPath = "D:\\codex\\codex-ui";

const rootEntry = {
  name: "codex-ui",
  path: rootPath,
  relPath: ".",
  isDir: true,
  size: null,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: 2,
};
const srcDir = {
  name: "src",
  path: rootPath + "\\src",
  relPath: "src",
  isDir: true,
  size: null,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: 1,
};
const aTxt = {
  name: "a.txt",
  path: rootPath + "\\a.txt",
  relPath: "a.txt",
  isDir: false,
  size: 1024,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: null,
};
const mainTs = {
  name: "main.ts",
  path: rootPath + "\\src\\main.ts",
  relPath: "src/main.ts",
  isDir: false,
  size: 2048,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: null,
};

function mockFs() {
  mockedInvoke.mockImplementation((cmd, args) => {
    if (cmd === "session_fs_metadata") return Promise.resolve(rootEntry);
    if (cmd === "session_fs_list") {
      const dir = (args as { dir?: string }).dir;
      if (dir === rootPath) return Promise.resolve([srcDir, aTxt]);
      if (dir === srcDir.path) return Promise.resolve([mainTs]);
      return Promise.resolve([]);
    }
    if (cmd === "session_fs_search") return Promise.resolve([mainTs]);
    if (
      cmd === "session_fs_watch_start" ||
      cmd === "session_fs_watch_stop" ||
      cmd === "session_fs_rename" ||
      cmd === "session_fs_delete" ||
      cmd === "session_fs_paste"
    ) {
      return Promise.resolve(undefined);
    }
    if (cmd === "git_changes_status") {
      return Promise.resolve({
        repoRoot: rootPath,
        branch: "main",
        files: [
          { path: "src/a.ts", status: "modified", staged: false, worktree: true },
        ],
      });
    }
    if (
      cmd === "git_changes_watch_start" ||
      cmd === "git_changes_watch_stop" ||
      cmd === "git_changes_init" ||
      cmd === "git_changes_diff"
    ) {
      return Promise.resolve(cmd === "git_changes_diff" ? "diff" : undefined);
    }
    return Promise.resolve(undefined);
  });
}

describe("RightPanel Tab 栏", () => {
  beforeEach(() => {
    store.threads = [];
    store.loadingHistory = false;
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.panelTab = "resources";
    mockedInvoke.mockClear();
    mockFs();
    __resetSessionFsForTest();
    __resetGitChangesForTest();
  });

  it("默认显示资源 Tab，历史会话面板隐藏", async () => {
    const wrapper = mount(RightPanel);
    await flushPromises();

    const tabs = wrapper.findAll(".panel-tab");
    expect(tabs).toHaveLength(3);
    expect(tabs.map((t) => t.text().trim())).toEqual([
      "资源",
      "会话",
      "Git",
    ]);
    // git 状态未加载时角标不显示
    expect(tabs[2].find(".tab-badge").exists()).toBe(false);
    expect(tabs[0].classes()).toContain("active");
    // v-show 单根化后互斥生效（happy-dom 的 isVisible 不可靠，直接断言 inline style）
    expect(
      (wrapper.find(".resource-view").element as HTMLElement).style.display,
    ).toBe("");
    expect(
      (wrapper.find(".history-view").element as HTMLElement).style.display,
    ).toBe("none");
    expect(
      (wrapper.find(".git-view").element as HTMLElement).style.display,
    ).toBe("none");
    // 默认资源 Tab 激活时加载文件树
    expect(wrapper.find(".resource-root").exists()).toBe(true);
  });

  it("切换到会话/资源 Tab：底部高亮切换、资源树加载", async () => {
    const wrapper = mount(RightPanel);
    await flushPromises();

    // 默认资源 → 切到会话
    await wrapper.findAll(".panel-tab")[1].trigger("click");
    await flushPromises();

    expect(wrapper.findAll(".panel-tab")[1].classes()).toContain("active");
    expect(wrapper.findAll(".panel-tab")[0].classes()).not.toContain("active");
    expect(
      (wrapper.find(".history-view").element as HTMLElement).style.display,
    ).toBe("");
    expect(
      (wrapper.find(".resource-view").element as HTMLElement).style.display,
    ).toBe("none");

    // 切回资源：资源树加载
    await wrapper.findAll(".panel-tab")[0].trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".panel-tab")[0].classes()).toContain("active");
    expect(
      (wrapper.find(".resource-view").element as HTMLElement).style.display,
    ).toBe("");
    expect(wrapper.find(".resource-root").exists()).toBe(true);
    expect(
      mockedInvoke.mock.calls.some(([cmd]) => cmd === "session_fs_list"),
    ).toBe(true);
  });

  it("切换到 Git 更改 Tab：底部高亮切换、加载 git 状态", async () => {
    const wrapper = mount(RightPanel);
    await flushPromises();

    await wrapper.findAll(".panel-tab")[2].trigger("click");
    await flushPromises();

    expect(wrapper.findAll(".panel-tab")[2].classes()).toContain("active");
    expect(
      (wrapper.find(".git-view").element as HTMLElement).style.display,
    ).toBe("");
    expect(
      (wrapper.find(".history-view").element as HTMLElement).style.display,
    ).toBe("none");
    expect(
      mockedInvoke.mock.calls.some(([cmd]) => cmd === "git_changes_status"),
    ).toBe(true);
    expect(wrapper.findComponent(GitView).exists()).toBe(true);
    expect(wrapper.find(".git-view").text()).toContain("src/a.ts");
    // Git Tab 角标显示更改文件数
    const gitTab = wrapper.findAll(".panel-tab")[2];
    expect(gitTab.find(".tab-badge").exists()).toBe(true);
    expect(gitTab.find(".tab-badge").text()).toBe("1");
  });

  it("Git 更改数角标：0 个文件时不渲染", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_status") {
        return Promise.resolve({
          repoRoot: rootPath,
          branch: "main",
          files: [],
        });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mount(RightPanel);
    await flushPromises();

    await wrapper.findAll(".panel-tab")[2].trigger("click");
    await flushPromises();

    expect(
      wrapper.findAll(".panel-tab")[2].find(".tab-badge").exists(),
    ).toBe(false);
    wrapper.unmount();
  });

  it("Tab 切换保留资源树展开状态（v-show）", async () => {
    const wrapper = mount(RightPanel);
    await flushPromises();

    // 默认资源 Tab：展开 src，main.ts 出现
    await wrapper
      .findAll(".resource-row.resource-dir")[0]
      .trigger("click");
    await flushPromises();
    expect(wrapper.find(".resource-view").text()).toContain("main.ts");

    // 切回历史再切回资源：组件实例与展开状态保留
    await wrapper.findAll(".panel-tab")[1].trigger("click");
    await wrapper.findAll(".panel-tab")[0].trigger("click");
    await flushPromises();

    expect(wrapper.findComponent(ResourceView).exists()).toBe(true);
    expect(wrapper.find(".resource-view").text()).toContain("main.ts");
  });

  it("新建会话后激活资源 Tab：从会话 Tab 切回资源并显示资源树", async () => {
    const wrapper = mount(RightPanel);
    await flushPromises();

    // 先切到会话 Tab，模拟新建会话前停留的位置
    await wrapper.findAll(".panel-tab")[1].trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".panel-tab")[1].classes()).toContain("active");
    expect(
      (wrapper.find(".resource-view").element as HTMLElement).style.display,
    ).toBe("none");

    // 模拟新建会话：全局 store 将 tab 置回资源
    store.panelTab = "resources";
    await flushPromises();

    expect(wrapper.findAll(".panel-tab")[0].classes()).toContain("active");
    expect(wrapper.findAll(".panel-tab")[1].classes()).not.toContain("active");
    expect(
      (wrapper.find(".resource-view").element as HTMLElement).style.display,
    ).toBe("");
    expect(
      (wrapper.find(".history-view").element as HTMLElement).style.display,
    ).toBe("none");
    expect(wrapper.find(".resource-root").exists()).toBe(true);
    wrapper.unmount();
  });
});

describe("RightPanel 宽度调节", () => {
  beforeEach(() => {
    store.threads = [];
    store.loadingHistory = false;
    store.server.workspace = rootPath;
    store.panelTab = "resources";
    mockedInvoke.mockClear();
    mockFs();
    __resetSessionFsForTest();
    __resetGitChangesForTest();
  });

  it("向左拖拽加宽面板，并钳制在半个窗口宽度内", async () => {
    const wrapper = mount(RightPanel);
    const handle = wrapper.find(".history-resize-handle");
    await handle.trigger("pointerdown", { clientX: 500 });
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 300 }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".history-panel").attributes("style")).toContain(
      "width: 464px",
    );

    window.dispatchEvent(new PointerEvent("pointermove", { clientX: -1000 }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".history-panel").attributes("style")).toContain(
      "width: 512px",
    );

    window.dispatchEvent(new PointerEvent("pointerup"));
    wrapper.unmount();
  });

  it("向右拖拽不窄于默认宽度 264px", async () => {
    const wrapper = mount(RightPanel);
    const handle = wrapper.find(".history-resize-handle");
    await handle.trigger("pointerdown", { clientX: 100 });
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 5000 }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".history-panel").attributes("style")).toContain(
      "width: 264px",
    );

    window.dispatchEvent(new PointerEvent("pointerup"));
    wrapper.unmount();
  });
});
