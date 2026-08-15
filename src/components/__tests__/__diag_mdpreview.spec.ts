import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";

vi.mock("../ChatView.vue", () => ({
  default: {
    name: "ChatViewStub",
    template: "<div class='chat-stub' />",
  },
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    loadAddon() {}
    open() {}
    onData() {}
    write() {}
    focus() {}
    dispose() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
    proposeDimensions() {
      return { cols: 100, rows: 30 };
    }
  },
}));
vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {},
  getDocument: vi.fn(() => ({
    promise: Promise.reject(new Error("mocked pdf")),
    destroy: vi.fn(() => Promise.resolve()),
  })),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import EditorPane from "../EditorPane.vue";
import {
  __resetEditorTabsForTest,
  activateTab,
  openFileTab,
  tabs,
} from "../../composables/useEditorTabs";
import { tooltipDirective } from "../../directives/tooltip";
import { __resetSessionFsForTest } from "../../composables/useSessionFs";
import { store } from "../../composables/useCodex";

const mockedInvoke = vi.mocked(invoke);
const root = "D:\\repo";
const bTxt = root + "\\b.txt";
const bMd = root + "\\b.md";

function fileContent(content: string) {
  return { content, validUtf8: true, byteSize: content.length };
}

async function settle() {
  await flushPromises();
  await nextTick();
}

async function waitForEl(wrapper: ReturnType<typeof mount>, selector: string) {
  await vi.waitFor(
    () => {
      expect(wrapper.find(selector).exists()).toBe(true);
    },
    { timeout: 5000, interval: 20 },
  );
}

describe("DIAG md preview after tab switch", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetEditorTabsForTest();
    __resetSessionFsForTest();
    store.confirm = null;
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("file-to-file and chat switch keep rendered content", async () => {
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_read") {
        const path = (args as { path?: string }).path;
        return Promise.resolve(
          path === bTxt
            ? fileContent("plain text b")
            : fileContent("# 标题\n\n**加粗** 正文"),
        );
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mount(EditorPane, {
      global: { directives: { tooltip: tooltipDirective } },
    });
    await openFileTab(root, "a.md");
    await settle();
    await waitForEl(wrapper, ".text-editor-actions button[aria-label='预览']");
    await wrapper
      .find(".text-editor-actions button[aria-label='预览']")
      .trigger("click");
    await settle();

    await vi.waitFor(
      () => {
        expect(wrapper.find(".text-editor-preview .md").text()).toContain("标题");
      },
      { timeout: 5000, interval: 20 },
    );
    const aTab = tabs.find((t) => t.title === "a.md")!;

    // 文件间切换后回来
    await openFileTab(root, bTxt);
    await settle();
    activateTab(aTab.id);
    await settle();
    await vi.waitFor(
      () => {
        const txt = wrapper.find(".text-editor-preview .md").text();
        expect(txt).toContain("标题");
        expect(txt).toContain("正文");
        expect(txt).not.toContain("plain text b");
      },
      { timeout: 5000, interval: 20 },
    );

    // 切到会话再回来
    activateTab("chat");
    await settle();
    activateTab(aTab.id);
    await settle();
    await vi.waitFor(
      () => {
        const txt = wrapper.find(".text-editor-preview .md").text();
        expect(txt).toContain("标题");
        expect(txt).toContain("正文");
      },
      { timeout: 5000, interval: 20 },
    );
    wrapper.unmount();
  });

  it("a.md 预览后打开 b.md 并预览：显示 b 内容而非 a 内容", async () => {
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_read") {
        const path = (args as { path?: string }).path;
        return Promise.resolve(
          path === bMd
            ? fileContent("# B标题\n\nB 正文")
            : fileContent("# A标题\n\nA 正文"),
        );
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mount(EditorPane, {
      global: { directives: { tooltip: tooltipDirective } },
    });

    // 打开 a.md 并进入预览，确认渲染的是 A 内容
    await openFileTab(root, "a.md");
    await settle();
    await waitForEl(wrapper, ".text-editor-actions button[aria-label='预览']");
    await wrapper
      .find(".text-editor-actions button[aria-label='预览']")
      .trigger("click");
    await settle();
    await vi.waitFor(
      () => {
        const txt = wrapper.find(".text-editor-preview .md").text();
        expect(txt).toContain("A标题");
      },
      { timeout: 5000, interval: 20 },
    );

    // 打开 b.md 并进入预览：必须显示 B 内容，不得残留 A 内容
    await openFileTab(root, bMd);
    await settle();
    await waitForEl(wrapper, ".text-editor-actions button[aria-label='预览']");
    await wrapper
      .find(".text-editor-actions button[aria-label='预览']")
      .trigger("click");
    await settle();
    await vi.waitFor(
      () => {
        const txt = wrapper.find(".text-editor-preview .md").text();
        expect(txt).toContain("B标题");
        expect(txt).not.toContain("A标题");
      },
      { timeout: 5000, interval: 20 },
    );
    wrapper.unmount();
  });
});
