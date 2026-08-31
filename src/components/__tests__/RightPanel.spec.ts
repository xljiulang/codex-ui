import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
        repoWorkspace: rootPath,
        branch: "main",
        hasRemote: true,
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
    store.loadingSessions = false;
    store.workspace = rootPath;
    store.panelTab = "session";
    mockedInvoke.mockClear();
    mockFs();
    __resetSessionFsForTest();
    __resetGitChangesForTest();
  });

  it("默认显示会话 Tab，资源面板隐藏", async () => {
    const wrapper = mount(RightPanel);
    await flushPromises();

    const tabs = wrapper.findAll(".panel-tab");
    expect(tabs).toHaveLength(3);
    expect(tabs.map((t) => t.text().trim())).toEqual([
      "会话",
      "资源",
      "Git",
    ]);
    // git 状态未加载时角标不显示
    expect(tabs[2].find(".tab-badge").exists()).toBe(false);
    expect(tabs[0].classes()).toContain("active");
    // v-show 单根化后互斥生效（happy-dom 的 isVisible 不可靠，直接断言 inline style）
    expect(
      (wrapper.find(".session-view").element as HTMLElement).style.display,
    ).toBe("");
    expect(
      (wrapper.find(".resource-view").element as HTMLElement).style.display,
    ).toBe("none");
    expect(
      (wrapper.find(".git-view").element as HTMLElement).style.display,
    ).toBe("none");
    // 默认会话 Tab：不加载资源树
    expect(wrapper.find(".resource-root").exists()).toBe(false);
  });

  it("切换到资源/会话 Tab：底部高亮切换、资源树加载", async () => {
    const wrapper = mount(RightPanel);
    await flushPromises();

    // 默认会话 → 切到资源
    await wrapper.findAll(".panel-tab")[1].trigger("click");
    await flushPromises();

    expect(wrapper.findAll(".panel-tab")[1].classes()).toContain("active");
    expect(wrapper.findAll(".panel-tab")[0].classes()).not.toContain("active");
    expect(
      (wrapper.find(".resource-view").element as HTMLElement).style.display,
    ).toBe("");
    expect(
      (wrapper.find(".session-view").element as HTMLElement).style.display,
    ).toBe("none");
    expect(wrapper.find(".resource-root").exists()).toBe(true);
    expect(
      mockedInvoke.mock.calls.some(([cmd]) => cmd === "session_fs_list"),
    ).toBe(true);

    // 切回会话
    await wrapper.findAll(".panel-tab")[0].trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".panel-tab")[0].classes()).toContain("active");
    expect(
      (wrapper.find(".session-view").element as HTMLElement).style.display,
    ).toBe("");
    expect(
      (wrapper.find(".resource-view").element as HTMLElement).style.display,
    ).toBe("none");
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
      (wrapper.find(".session-view").element as HTMLElement).style.display,
    ).toBe("none");
    expect(
      mockedInvoke.mock.calls.some(([cmd]) => cmd === "git_changes_status"),
    ).toBe(true);
    expect(wrapper.findComponent(GitView).exists()).toBe(true);
    expect(wrapper.find(".git-view").text()).toContain("a.ts");
  });

  it("Tab 切换保留资源树展开状态（v-show）", async () => {
    const wrapper = mount(RightPanel);
    await flushPromises();

    // 切到资源 Tab：展开 src，main.ts 出现
    await wrapper.findAll(".panel-tab")[1].trigger("click");
    await flushPromises();
    await wrapper
      .findAll(".resource-row.resource-dir")[0]
      .trigger("click");
    await flushPromises();
    expect(wrapper.find(".resource-view").text()).toContain("main.ts");

    // 切回会话再切回资源：组件实例与展开状态保留
    await wrapper.findAll(".panel-tab")[0].trigger("click");
    await wrapper.findAll(".panel-tab")[1].trigger("click");
    await flushPromises();

    expect(wrapper.findComponent(ResourceView).exists()).toBe(true);
    expect(wrapper.find(".resource-view").text()).toContain("main.ts");
  });
});

describe("RightPanel 宽度调节", () => {
  const realInnerWidth = window.innerWidth;

  beforeEach(() => {
    Object.defineProperty(window, "innerWidth", {
      value: 1024,
      configurable: true,
    });
    store.threads = [];
    store.loadingSessions = false;
    store.workspace = rootPath;
    store.panelTab = "session";
    mockedInvoke.mockClear();
    mockFs();
    __resetSessionFsForTest();
    __resetGitChangesForTest();
  });

  afterEach(() => {
    Object.defineProperty(window, "innerWidth", {
      value: realInnerWidth,
      configurable: true,
    });
  });

  it("初始宽度按窗口比例 24%（1024 → 246px）；向左拖拽加宽并钳制在半个窗口宽度内", async () => {
    const wrapper = mount(RightPanel);
    expect(wrapper.find(".right-panel").attributes("style")).toContain(
      "width: 246px",
    );
    const handle = wrapper.find(".right-panel-resize-handle");
    await handle.trigger("pointerdown", { clientX: 500 });
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 300 }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".right-panel").attributes("style")).toContain(
      "width: 446px",
    );

    window.dispatchEvent(new PointerEvent("pointermove", { clientX: -1000 }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".right-panel").attributes("style")).toContain(
      "width: 512px",
    );

    window.dispatchEvent(new PointerEvent("pointerup"));
    wrapper.unmount();
  });

  it("向右拖拽不窄于最小宽度（16% 与 200px 兜底取较大者）", async () => {
    const wrapper = mount(RightPanel);
    const handle = wrapper.find(".right-panel-resize-handle");
    await handle.trigger("pointerdown", { clientX: 100 });
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 5000 }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".right-panel").attributes("style")).toContain(
      "width: 200px",
    );

    window.dispatchEvent(new PointerEvent("pointerup"));
    wrapper.unmount();
  });
});
