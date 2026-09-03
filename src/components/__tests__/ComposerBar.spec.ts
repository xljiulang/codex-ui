import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { reactive } from "vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

const mockDragDropHandlers: Array<
  (event: { payload: { type: string; paths?: string[]; position?: unknown } }) => void
> = [];

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn(
      async (handler: (event: { payload: { type: string; paths?: string[]; position?: unknown } }) => void) => {
        mockDragDropHandlers.push(handler);
        return () => {};
      },
    ),
  }),
}));

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, sendPrompt: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import ComposerBar from "../ComposerBar.vue";
import {
  addAttachmentToActiveSession,
  ensureThreadPlugins,
  sendPrompt,
  store,
  type SessionTab,
  __resetSessionTabsForTest,
  activeSessionTab,
} from "../../composables/useCodex";
import type { UserInput } from "../../lib/types";
import { activeTabId, tabs as _tabs } from "../../composables/useEditorTabs";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";
/** 本 spec 的会话标签 fixture 直接放入统一列表 */
const tabs = _tabs as unknown as SessionTab[];

/** 默认会话标签 fixture：多数渲染用例仅需满足 ComposerBar 的 tab prop */
function defaultTab(): SessionTab {
  __resetSessionTabsForTest();
  const t = reactive(makeSessionTab("s1", null));
  // 注册为活动标签：activeSessionTab()/附件/模型等按标签读写的路径可见
  tabs.push(t);
  activeTabId.value = "s1";
  return t;
}

const mockedInvoke = vi.mocked(invoke);
const mockedSendPrompt = vi.mocked(sendPrompt);

const SKILLS_RESPONSE = {
  data: [
    {
      skills: [
        {
          name: "csharp-code-rules",
          key: "csharp-code-rules",
          path: "C:/Users/x/.codex/skills/csharp-code-rules/SKILL.md",
          desc: "C# 代码规范",
        },
        {
          name: "ida-pro-mcp:idapython",
          key: "ida-pro-mcp:idapython",
          path: "C:/Users/x/.codex/plugins/cache/mrexodia/ida-pro-mcp/0.1.0/skills/idapython/SKILL.md",
          desc: "IDA Python",
        },
      ],
    },
  ],
};

const PLUGINS_RESPONSE = {
  marketplaces: [
    {
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          installed: true,
          enabled: true,
          source: {
            path: "C:/Users/x/.codex/plugins/cache/documents",
          },
          interface: {
            displayName: "Documents",
            shortDescription: "文档处理",
            composerIcon:
              "C:/Users/x/.codex/plugins/cache/documents/assets/icon.png",
            brandColor: "#2563EB",
          },
        },
        {
          id: "pdf@openai-primary-runtime",
          name: "pdf",
          installed: true,
          enabled: true,
          source: {
            path: "C:/Users/x/.codex/plugins/cache/pdf",
          },
          interface: {
            displayName: "PDF",
            shortDescription: "PDF 处理",
          },
        },
      ],
    },
  ],
};

const FILE_RESULT = {
  root: "D:/repo",
  path: "src/a.cs",
  file_name: "a.cs",
  match_type: "file",
  score: 1,
  indices: null,
};

function mockRpc(withFileResults: boolean) {
  mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
    if (cmd !== "codex_rpc") return {};
    if (args?.method === "plugin/list") {
      return PLUGINS_RESPONSE;
    }
    if (args?.method === "skills/list") {
      return SKILLS_RESPONSE;
    }
    if (args?.method === "fuzzyFileSearch") {
      return withFileResults ? { files: [FILE_RESULT] } : { files: [] };
    }
    return {};
  });
}

const waitSearch = () => new Promise((r) => setTimeout(r, 320));

function getEditor(): any {
  const ed = (window as unknown as Record<string, unknown>)
    .__CODEX_UI_EDITOR__;
  if (!ed) throw new Error("编辑器实例未暴露");
  return ed;
}

async function typeInEditor(text: string) {
  getEditor().commands.insertContent(text);
  await flushPromises();
}

describe("ComposerBar TipTap 富文本编辑器", () => {
  let wrapper: VueWrapper | null = null;

  beforeEach(() => {
    activeSessionTab()?.attachments.splice(0);
    store.workspace = "D:/repo";
    store.settings.enter_to_send = true;
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
    mockRpc(false);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  const menuLabels = () =>
    wrapper!
      .findAll(".mention-menu button.menu-item .menu-item-label")
      .map((b) => b.text().trim());

  async function clickMenuRow(text: string) {
    const row = wrapper!
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes(text));
    expect(row).toBeTruthy();
    await row!.trigger("click");
    await flushPromises();
  }

  it("挂载后渲染富文本编辑器（.ProseMirror）且 + 按钮不存在", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await flushPromises();
    expect(wrapper.find(".ProseMirror").exists()).toBe(true);
    expect(wrapper.find("textarea").exists()).toBe(false);
    expect(wrapper.find("button.plus-btn").exists()).toBe(false);
  });

  it("会话输入框右键事件被拦截（不弹右键菜单）", () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    const el = wrapper.find(".rich-editor").element;
    const ev = new MouseEvent("contextmenu", {
      cancelable: true,
      bubbles: true,
    });
    el.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
  });

  it("新会话态不再渲染输入框上方的项目目录行", () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    expect(wrapper.find(".newchat-cwd-row").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("项目目录");
  });

  it("@ 空 token：菜单固定行在前、插件列表在后", async () => {
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("@");
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
    const labels = menuLabels();
    expect(labels.slice(0, 2)).toEqual(["选择文件…", "选择文件夹…"]);
    expect(labels).toContain("Documents");
    expect(labels.indexOf("选择文件夹…")).toBeLessThan(
      labels.indexOf("Documents"),
    );
  });

  it("联合搜索：命中插件在命中文件前", async () => {
    mockRpc(true);
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("@doc");
    await waitSearch();
    const labels = menuLabels();
    expect(labels).toContain("a.cs");
    expect(labels).toContain("Documents");
    expect(labels.indexOf("Documents")).toBeLessThan(labels.indexOf("a.cs"));
  });

  it("@ 无插件时文件结果完整展示：组标题不吞首条结果，可点击选中", async () => {
    mockRpc(true);
    // 不预加载插件缓存，模拟当前会话无可用插件
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await typeInEditor("@a");
    await waitSearch();
    const labels = menuLabels();
    expect(labels).toContain("a.cs");
    const firstFile = wrapper!
      .findAll(".mention-menu button.menu-item .menu-item-label")
      .find((b) => b.text().trim() === "a.cs");
    expect(firstFile).toBeTruthy();
    await firstFile!.trigger("click");
    await flushPromises();
    const rowChip = wrapper!.find(".attachment-chip");
    expect(rowChip.exists()).toBe(true);
    expect(rowChip.text()).toContain("@a.cs");
    expect(activeSessionTab()?.attachments).toEqual([
      { type: "mention", name: "a.cs", path: "D:/repo/src/a.cs" },
    ]);
  });

  it("选中文件进附件区（不内联），触发词被移除且附件同步", async () => {
    mockRpc(true);
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("看 @a.cs");
    await waitSearch();
    await clickMenuRow("a.cs");
    const ed = getEditor();
    expect(ed.getText().replace(/\s+/g, " ").trim()).toBe("看");
    expect(wrapper.find(".ref-chip").exists()).toBe(false);
    const rowChip = wrapper.find(".attachment-chip");
    expect(rowChip.exists()).toBe(true);
    expect(rowChip.text()).toContain("@a.cs");
    expect(activeSessionTab()?.attachments).toEqual([
      { type: "mention", name: "a.cs", path: "D:/repo/src/a.cs" },
    ]);
  });

  it("选中插件生成 @ 前缀 chip 与 source=plugin 附件", async () => {
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("@doc");
    await waitSearch();
    await clickMenuRow("Documents");
    const chip = wrapper.find(".ref-chip");
    expect(chip.exists()).toBe(true);
    expect(chip.text()).toBe("@documents");
    expect(activeSessionTab()?.attachments).toEqual([
      {
        type: "skill",
        name: "documents",
        path: "C:/Users/x/.codex/plugins/cache/documents",
        source: "plugin",
        pluginId: "documents@openai-primary-runtime",
      },
    ]);
  });

  it("$ 技能选中生成 $ 前缀 chip 与 source=skill 附件", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await typeInEditor("$csharp-code-rules");
    await flushPromises();
    await clickMenuRow("csharp-code-rules");
    const chip = wrapper.find(".ref-chip");
    expect(chip.exists()).toBe(true);
    expect(chip.text()).toBe("$csharp-code-rules");
    expect(activeSessionTab()?.attachments).toEqual([
      {
        type: "skill",
        name: "csharp-code-rules",
        path: "C:/Users/x/.codex/skills/csharp-code-rules/SKILL.md",
        source: "skill",
      },
    ]);
  });

  it("混编：文件进附件区、技能内联，按顺序序列化", async () => {
    mockRpc(true);
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("先 ");
    await typeInEditor("@a.cs");
    await waitSearch();
    await clickMenuRow("a.cs");
    await typeInEditor("中间 ");
    await typeInEditor("$csharp-code-rules");
    await waitSearch(); // 等待技能列表异步加载
    await clickMenuRow("csharp-code-rules");
    await typeInEditor("结尾");
    const ed = getEditor();
    expect(ed.getText().replace(/\s+/g, " ").trim()).toBe("先 中间 结尾");
    // 编辑器只内联技能；文件在附件区
    expect(wrapper.findAll(".ref-chip").map((x) => x.text())).toEqual([
      "$csharp-code-rules",
    ]);
    const rowTexts = wrapper
      .findAll(".attachment-chip")
      .map((x) => x.text().replace("×", "").trim());
    expect(rowTexts).toEqual(["@a.cs"]);
    expect(activeSessionTab()?.attachments.map((a) => a.type)).toEqual(["skill", "mention"]);
  });

  it("发送时把内联引用写入 activeSessionTab()?.attachments 并调用 sendPrompt", async () => {
    mockRpc(true);
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("@a.cs");
    await waitSearch();
    await clickMenuRow("a.cs");
    await typeInEditor("检查一下");
    await wrapper.find("button.send-btn").trigger("click");
    expect(mockedSendPrompt).toHaveBeenCalledWith(
      expect.stringContaining("## a.cs: D:/repo/src/a.cs"),
      false,
    );
    expect(mockedSendPrompt.mock.calls[0][0]).toContain("检查一下");
    expect(activeSessionTab()?.attachments).toEqual([
      { type: "mention", name: "a.cs", path: "D:/repo/src/a.cs" },
    ]);
    expect(getEditor().getText()).toBe("");
  });

  it("图片文件经本地选择器仍进入下方附件区（不内联）", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "pick_files") return ["D:/repo/pic.png"];
      if (cmd === "codex_rpc") {
        if (args?.method === "plugin/list") return PLUGINS_RESPONSE;
        if (args?.method === "skills/list") return SKILLS_RESPONSE;
      }
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await typeInEditor("@");
    await flushPromises();
    await wrapper.find(".ProseMirror").trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("pick_files", {
      multiple: true,
      initialDir: "D:/repo",
    });
    expect(wrapper.findAll(".ref-chip").length).toBe(0);
    expect(wrapper.findAll(".attachment-chip").length).toBe(1);
    expect(activeSessionTab()?.attachments).toEqual([
      { type: "localImage", path: "D:/repo/pic.png" },
    ]);
  });

  it("@ 菜单打开时 Enter 选中高亮行而不是发送", async () => {
    mockRpc(true);
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("@a.cs");
    await waitSearch();
    await wrapper.find(".ProseMirror").trigger("keydown", { key: "ArrowDown" });
    await wrapper.find(".ProseMirror").trigger("keydown", { key: "ArrowDown" });
    await wrapper.find(".ProseMirror").trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(mockedSendPrompt).not.toHaveBeenCalled();
    expect(wrapper.find(".ref-chip").exists()).toBe(false);
    expect(wrapper.find(".attachment-chip").exists()).toBe(true);
  });

  it("词中 @ 不弹菜单，空格后 @ 弹菜单", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await typeInEditor("看下@file");
    expect(wrapper.find(".mention-menu").exists()).toBe(false);
    await typeInEditor(" 看下 @file.txt");
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
  });

  it("仅附件无文本时发送按钮可用", async () => {
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("@doc");
    await waitSearch();
    await clickMenuRow("Documents");
    const btn = wrapper.find("button.send-btn");
    expect(btn.attributes("disabled")).toBeUndefined();
  });

  it("Esc 关闭菜单", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await typeInEditor("@");
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(false);
  });

  it("快捷发送开启：Ctrl+Enter 插入硬换行且不发送", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await typeInEditor("abc");
    await wrapper
      .find(".ProseMirror")
      .trigger("keydown", { key: "Enter", ctrlKey: true });
    await flushPromises();
    expect(JSON.stringify(getEditor().getJSON())).toContain("hardBreak");
    expect(mockedSendPrompt).not.toHaveBeenCalled();
  });

  it("快捷发送开启：Shift+Enter 无操作，普通 Enter 发送", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await typeInEditor("abc");
    await wrapper
      .find(".ProseMirror")
      .trigger("keydown", { key: "Enter", shiftKey: true });
    await flushPromises();
    expect(JSON.stringify(getEditor().getJSON())).not.toContain("hardBreak");
    expect(getEditor().getText()).toBe("abc");
    expect(mockedSendPrompt).not.toHaveBeenCalled();
    await wrapper.find(".ProseMirror").trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(mockedSendPrompt).toHaveBeenCalledWith(
      expect.stringContaining("abc"),
      false,
    );
  });

  it("快捷发送关闭：Enter 换行不发送，Ctrl+Enter 发送并翻转", async () => {
    store.settings.enter_to_send = false;
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await typeInEditor("abc");
    await wrapper.find(".ProseMirror").trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(mockedSendPrompt).not.toHaveBeenCalled();
    const doc = getEditor().getJSON() as { content?: unknown[] };
    expect((doc.content ?? []).length).toBeGreaterThan(1);
    await wrapper
      .find(".ProseMirror")
      .trigger("keydown", { key: "Enter", ctrlKey: true });
    await flushPromises();
    expect(mockedSendPrompt).toHaveBeenCalledWith(
      expect.stringContaining("abc"),
      true,
    );
  });

  it("@ 菜单打开时 Ctrl+Enter 插入换行而非选中高亮项", async () => {
    mockRpc(true);
    const tab = defaultTab();
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("@doc");
    await waitSearch();
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
    await wrapper
      .find(".ProseMirror")
      .trigger("keydown", { key: "Enter", ctrlKey: true });
    await flushPromises();
    expect(JSON.stringify(getEditor().getJSON())).toContain("hardBreak");
    expect(mockedSendPrompt).not.toHaveBeenCalled();
    expect(wrapper.findAll(".attachment-chip").length).toBe(0);
    expect(wrapper.findAll(".ref-chip").length).toBe(0);
  });
});

describe("ComposerBar 粘贴图片/文件", () => {
  let wrapper: VueWrapper | null = null;

  beforeEach(() => {
    activeSessionTab()?.attachments.splice(0);
    store.toast = "";
    store.workspace = "D:/repo";
    store.settings.enter_to_send = true;
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  function makeFileItem(name: string, type: string, size = 8) {
    return {
      kind: "file",
      type,
      getAsFile: () =>
        ({
          name,
          type,
          size,
          arrayBuffer: async () => new Uint8Array(size).buffer,
        }) as unknown as File,
    };
  }

  function makeDataTransfer(items: unknown[]) {
    return {
      items,
      files: [],
      types: [],
      getData: () => "",
      setData: () => {},
    } as unknown as DataTransfer;
  }

  async function pasteItems(items: unknown[]) {
    await flushPromises();
    const dt = makeDataTransfer(items);
    const ev = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(ev, "clipboardData", { value: dt });
    wrapper!.find(".ProseMirror").element.dispatchEvent(ev);
    await flushPromises();
    // 排空宏任务：避免上一用例残留的异步 invoke 计入当前用例调用历史
    await new Promise((r) => setTimeout(r, 0));
    await flushPromises();
  }

  it("粘贴截图位图（无原始路径）：落盘并生成 localImage 附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return [];
      if (cmd === "save_pasted_image") return "C:/tmp/paste/pasted-1-1.png";
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await pasteItems([makeFileItem("clip.png", "image/png")]);

    expect(mockedInvoke).toHaveBeenCalledWith("save_pasted_image", {
      bytes: expect.any(Array),
      name: "pasted.png",
    });
    expect(activeSessionTab()?.attachments).toEqual([
      { type: "localImage", path: "C:/tmp/paste/pasted-1-1.png" },
    ]);
    expect(wrapper.find(".attachment-thumb").exists()).toBe(true);
    expect(getEditor().getText()).toBe("");
  });

  it("粘贴项取不到 File 时放行默认粘贴（不 preventDefault）", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await flushPromises(); // 等 TipTap 创建 .ProseMirror
    const dt = makeDataTransfer([
      { kind: "file", type: "image/png", getAsFile: () => null },
    ]);
    const ev = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(ev, "clipboardData", { value: dt });
    wrapper!.find(".ProseMirror").element.dispatchEvent(ev);
    await flushPromises();

    expect(ev.defaultPrevented).toBe(false);
    expect(activeSessionTab()?.attachments).toHaveLength(0);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "save_pasted_image",
      expect.anything(),
    );
  });

  it("粘贴图片文件（原始路径可解析）：直接用原路径，不落盘", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return ["D:/repo/shot.png"];
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await pasteItems([makeFileItem("shot.png", "image/png")]);

    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "save_pasted_image",
      expect.anything(),
    );
    expect(activeSessionTab()?.attachments).toEqual([
      { type: "localImage", path: "D:/repo/shot.png" },
    ]);
  });

  it("粘贴非图片文件（原始路径可解析）：生成 mention 附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return ["D:/repo/report.pdf"];
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await pasteItems([makeFileItem("report.pdf", "application/pdf")]);

    expect(activeSessionTab()?.attachments).toEqual([
      { type: "mention", name: "report.pdf", path: "D:/repo/report.pdf" },
    ]);
    expect(wrapper.find(".attachment-chip").text()).toContain("@report.pdf");
  });

  it("粘贴非图片文件（无原始路径）：提示暂不支持且不加附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return [];
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await pasteItems([makeFileItem("report.pdf", "application/pdf")]);

    expect(store.toast).toContain("暂不支持该粘贴");
    expect(activeSessionTab()?.attachments).toEqual([]);
  });

  it("粘贴超大图片（>20MB）：提示跳过且不加附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return [];
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await pasteItems([
      makeFileItem("huge.png", "image/png", 21 * 1024 * 1024),
    ]);

    expect(store.toast).toContain("图片过大");
    expect(activeSessionTab()?.attachments).toEqual([]);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "save_pasted_image",
      expect.anything(),
    );
  });

  it("纯文本粘贴：走默认行为，不触发附件逻辑", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await flushPromises();
    const dt = makeDataTransfer([
      { kind: "string", type: "text/plain" },
    ]);
    const ev = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(ev, "clipboardData", { value: dt });
    wrapper.find(".ProseMirror").element.dispatchEvent(ev);
    await flushPromises();

    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "clipboard_file_paths",
      expect.anything(),
    );
    expect(activeSessionTab()?.attachments).toEqual([]);
  });

  it("混合粘贴：截图位图落盘 + 文件原路径，一次全部处理", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return ["D:/repo/a.txt"];
      if (cmd === "save_pasted_image") return "C:/tmp/paste/pasted-2-2.png";
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await pasteItems([
      makeFileItem("clip.png", "image/png"),
      makeFileItem("a.txt", "text/plain"),
    ]);

    expect(activeSessionTab()?.attachments).toEqual([
      { type: "localImage", path: "C:/tmp/paste/pasted-2-2.png" },
      { type: "mention", name: "a.txt", path: "D:/repo/a.txt" },
    ]);
    expect(mockedInvoke).toHaveBeenCalledWith("clipboard_file_paths");
    expect(mockedInvoke).toHaveBeenCalledWith(
      "save_pasted_image",
      expect.objectContaining({ name: "pasted.png" }),
    );
    expect(
      mockedInvoke.mock.calls.filter(([cmd]) => cmd === "save_pasted_image"),
    ).toHaveLength(1);
  });
});

describe("ComposerBar 拖放图片/文件", () => {
  let wrapper: VueWrapper | null = null;

  beforeEach(() => {
    activeSessionTab()?.attachments.splice(0);
    store.toast = "";
    store.workspace = "D:/repo";
    store.settings.enter_to_send = true;
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
    mockRpc(false);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  function makeDropFile(name: string, type: string, size = 8, path?: string) {
    const f = {
      name,
      type,
      size,
      arrayBuffer: async () => new Uint8Array(size).buffer,
    } as unknown as File;
    if (path) Object.defineProperty(f, "path", { value: path });
    return f;
  }

  function dropFiles(files: File[]) {
    const dt = { files, items: [], types: [] } as unknown as DataTransfer;
    const ev = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(ev, "dataTransfer", { value: dt });
    wrapper!.find(".composer").element.dispatchEvent(ev);
  }

  async function dropAndFlush(files: File[]) {
    await flushPromises();
    dropFiles(files);
    await flushPromises();
  }

  it("拖入带路径的图片：直接用原路径生成 localImage 附件", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await dropAndFlush([makeDropFile("shot.png", "image/png", 8, "D:/repo/shot.png")]);

    expect(activeSessionTab()?.attachments).toEqual([
      { type: "localImage", path: "D:/repo/shot.png" },
    ]);
  });

  it("拖入无路径的图片：落盘生成 localImage 附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "save_pasted_image") return "C:/tmp/drop/pasted-1-1.png";
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await dropAndFlush([makeDropFile("clip.png", "image/png")]);

    expect(mockedInvoke).toHaveBeenCalledWith("save_pasted_image", {
      bytes: expect.any(Array),
      name: "pasted.png",
    });
    expect(activeSessionTab()?.attachments).toEqual([
      { type: "localImage", path: "C:/tmp/drop/pasted-1-1.png" },
    ]);
  });

  it("拖入带路径的非图片：生成 mention 附件", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await dropAndFlush([makeDropFile("a.txt", "text/plain", 8, "D:/repo/a.txt")]);

    expect(activeSessionTab()?.attachments).toEqual([
      { type: "mention", name: "a.txt", path: "D:/repo/a.txt" },
    ]);
  });

  it("拖入无路径的非图片：提示暂不支持拖放且不加附件", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await dropAndFlush([makeDropFile("a.txt", "text/plain")]);

    expect(store.toast).toContain("暂不支持该拖放");
    expect(activeSessionTab()?.attachments).toEqual([]);
  });

  it("纯文本拖放：不阻止默认行为，不生成附件", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await flushPromises();
    const ev = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(ev, "dataTransfer", {
      value: { files: [], items: [], types: [] },
    });
    wrapper!.find(".composer").element.dispatchEvent(ev);

    expect(ev.defaultPrevented).toBe(false);
    expect(activeSessionTab()?.attachments).toEqual([]);
  });

  it("Tauri 拖放事件：over 高亮、drop 路径生成附件（图片 localImage、文件 mention）", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await flushPromises();
    const handler = mockDragDropHandlers[mockDragDropHandlers.length - 1];

    handler({ payload: { type: "over", position: {} as never } });
    await flushPromises();
    expect(wrapper.find(".composer").classes()).toContain("dragover");

    handler({
      payload: {
        type: "drop",
        paths: ["D:/repo/shot.png", "D:/repo/a.txt"],
        position: {} as never,
      },
    });
    await flushPromises();

    expect(wrapper.find(".composer").classes()).not.toContain("dragover");
    expect(activeSessionTab()?.attachments).toEqual([
      { type: "localImage", path: "D:/repo/shot.png" },
      { type: "mention", name: "a.txt", path: "D:/repo/a.txt" },
    ]);
  });
});

describe("ComposerBar 输入框高度拖拽调节", () => {
  let wrapper: VueWrapper | null = null;
  const realInnerHeight = window.innerHeight;

  beforeEach(() => {
    Object.defineProperty(window, "innerHeight", {
      value: 600,
      configurable: true,
    });
    activeSessionTab()?.attachments.splice(0);
    store.workspace = "D:/repo";
    store.settings.enter_to_send = true;
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
    mockRpc(false);
  });

  afterEach(() => {
    Object.defineProperty(window, "innerHeight", {
      value: realInnerHeight,
      configurable: true,
    });
    wrapper?.unmount();
    wrapper = null;
  });

  function rowStyle(): string {
    return wrapper!.find(".composer-input-row").attributes("style") ?? "";
  }

  async function dragTo(clientY: number) {
    await flushPromises();
    const handle = wrapper!.find(".composer-resize-handle").element;
    handle.dispatchEvent(
      new PointerEvent("pointerdown", { clientY: 200, bubbles: true }),
    );
    window.dispatchEvent(new PointerEvent("pointermove", { clientY }));
    window.dispatchEvent(new PointerEvent("pointerup", {}));
    await flushPromises();
  }

  it("未拖拽时无内联高度；拖拽后设置高度，可继续以当前高度为基准增长", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await flushPromises();
    expect(rowStyle()).toBe("");

    await dragTo(100);
    const h1 = Number(
      /--composer-h:\s*(\d+)px/.exec(rowStyle())?.[1] ?? "0",
    );
    expect(h1).toBeGreaterThanOrEqual(100);

    await dragTo(50);
    const h2 = Number(
      /--composer-h:\s*(\d+)px/.exec(rowStyle())?.[1] ?? "0",
    );
    // 第二次拖拽以当前高度为基准继续增长
    expect(h2).toBeGreaterThan(h1);
  });

  it("向上拖超过窗口一半被夹紧到半屏（innerHeight/2 = 300）", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await dragTo(200 - 10000);
    expect(rowStyle()).toContain("--composer-h: 300px");
  });

  it("向下拖低于最低高度被夹紧到 78px（600 高下空态整卡保持 128px）", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await dragTo(200 + 10000);
    expect(rowStyle()).toContain("--composer-h: 78px");
  });
});

describe("ComposerBar 模型按钮与弹出层", () => {
  let wrapper: VueWrapper | null = null;
  let tab: SessionTab;

  const GPT5 = {
    id: "gpt-5",
    model: "gpt-5",
    displayName: "gpt-5",
    description: "最新模型",
    hidden: false,
    isDefault: true,
    supportedReasoningEfforts: [
      { reasoningEffort: "high", description: "高" },
    ],
    defaultReasoningEffort: "high",
  };

  const EXTRA = {
    id: "gpt-5-extra",
    model: "gpt-5-extra",
    displayName: "gpt-5-extra",
    description: "备选模型",
    hidden: false,
    isDefault: false,
    supportedReasoningEfforts: [
      { reasoningEffort: "low", description: "低" },
      { reasoningEffort: "medium", description: "中" },
    ],
    defaultReasoningEffort: "medium",
  };

  beforeEach(() => {
    __resetSessionTabsForTest();
    tab = defaultTab();
    tab.threadId = "t1";
    store.models = [];
    store.modelsLoaded = false;
    mockedInvoke.mockReset();
    mockRpc(false);
  });

  afterEach(() => {
    store.models = [];
    wrapper?.unmount();
    wrapper = null;
  });

  it("模型按钮显示“名称 (强度)”带空格；无强度时仅显示名称", async () => {
    store.models = [GPT5];
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    const chip = () => wrapper!.find(".model-chip").text();
    expect(chip()).toContain("gpt-5 (high)");

    // 无推理强度：仅显示模型名，不带括号
    store.models[0].defaultReasoningEffort = "";
    await flushPromises();
    expect(chip()).toBe("gpt-5");
  });

  it("弹出层外部 mousedown 自动关闭；内部与触发按钮不关闭", async () => {
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".model-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(true);

    // 弹出层内部 mousedown 不关闭
    await wrapper.find(".popup-menu .menu-group-title").trigger("mousedown");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(true);

    // 触发按钮 mousedown 不自动关闭，click 负责切换关闭
    await wrapper.find(".model-chip").trigger("mousedown");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(true);
    await wrapper.find(".model-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(false);

    // 权限/任务/模型三个菜单：外部 mousedown 均自动关闭
    await wrapper.find(".perm-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(true);
    window.dispatchEvent(new MouseEvent("mousedown"));
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(false);

    await wrapper.find(".collab-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(true);
    window.dispatchEvent(new MouseEvent("mousedown"));
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(false);

    await wrapper.find(".model-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(true);
    window.dispatchEvent(new MouseEvent("mousedown"));
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(false);
  });

  it("打开一个菜单时自动关闭另外两个，再点当前按钮关闭", async () => {
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".perm-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").text()).toContain("应如何批准 Codex 操作？");

    await wrapper.find(".collab-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").text()).toContain("协作模式");
    expect(wrapper.find(".popup-menu").text()).not.toContain("应如何批准 Codex 操作？");

    await wrapper.find(".model-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").text()).toContain("模型");
    expect(wrapper.find(".popup-menu").text()).not.toContain("协作模式");

    await wrapper.find(".model-chip").trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(false);
  });

  it("应用模型/强度后立即调用 thread/settings/update 同步当前会话", async () => {
    store.toast = "";
    store.modelsLoaded = true;
    store.models = [GPT5, EXTRA];
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".model-chip").trigger("click");
    await flushPromises();
    const optionBtns = () => wrapper!.findAll(".popup-menu .option-btn");
    // 选择非默认模型
    await optionBtns()[1].trigger("click");
    await flushPromises();
    // 选择非默认推理强度 low
    const low = optionBtns().find((b) => b.text().trim() === "low")!;
    await low.trigger("click");
    await flushPromises();
    await wrapper.find(".popup-menu .btn.primary").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/settings/update",
      params: { threadId: "t1", model: "gpt-5-extra", effort: "low" },
    });
    expect(activeSessionTab()?.model).toBe("gpt-5-extra");
    expect(activeSessionTab()?.effort).toBe("low");
    expect(wrapper.find(".popup-menu").exists()).toBe(false);
  });

  it("无当前会话时应用模型不调用 thread/settings/update", async () => {
    tab.threadId = null;
    store.toast = "";
    store.modelsLoaded = true;
    store.models = [GPT5, EXTRA];
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".model-chip").trigger("click");
    await flushPromises();
    await wrapper.findAll(".popup-menu .option-btn")[1].trigger("click");
    await flushPromises();
    await wrapper.find(".popup-menu .btn.primary").trigger("click");
    await flushPromises();
    expect(activeSessionTab()?.model).toBe("gpt-5-extra");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "codex_rpc",
      expect.objectContaining({ method: "thread/settings/update" }),
    );
  });

  it("thread/settings/update 失败时 toast 提示并关闭菜单", async () => {
    store.toast = "";
    store.modelsLoaded = true;
    store.models = [GPT5];
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "thread/settings/update") {
        throw new Error("同步失败");
      }
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".model-chip").trigger("click");
    await flushPromises();
    await wrapper.find(".popup-menu .btn.primary").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("同步失败");
    expect(activeSessionTab()?.model).toBeNull();
    expect(wrapper.find(".popup-menu").exists()).toBe(false);
  });
});

describe("ComposerBar 手动压缩上下文", () => {
  let wrapper: VueWrapper | null = null;
  let tab: SessionTab;

  beforeEach(() => {
    __resetSessionTabsForTest();
    tab = defaultTab();
    tab.threadId = "t1";
    tab.threadTokenUsage = { used: 5000, window: 10000 };
    store.toast = "";
    store.confirm = null;
    mockedInvoke.mockReset();
    mockRpc(false);
  });

  afterEach(() => {
    vi.useRealTimers();
    wrapper?.unmount();
    wrapper = null;
  });

  it("有用量时渲染百分比与 aria-label，window 未知时不渲染", async () => {
    wrapper = mount(ComposerBar, { props: { tab } });
    const btn = wrapper.find(".ctx-window");
    expect(btn.exists()).toBe(true);
    expect(btn.text()).toBe("50%");
    expect(btn.attributes("aria-label")).toBe("压缩上下文");

    tab.threadTokenUsage = null;
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".ctx-window").exists()).toBe(false);
  });

  it("单击不触发压缩", async () => {
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".ctx-window").trigger("click");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "codex_rpc",
      expect.objectContaining({ method: "thread/compact/start" }),
    );
  });

  it("双击发起压缩并提示", async () => {
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".ctx-window").trigger("dblclick");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/compact/start",
      params: { threadId: "t1" },
    });
    expect(store.toast).toContain("已开始压缩上下文");
  });

  it("无当前会话时按钮禁用，回合进行中不禁用且可压缩", async () => {
    tab.threadId = null;
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.vm.$nextTick();
    expect(
      (wrapper.find(".ctx-window").element as HTMLButtonElement).disabled,
    ).toBe(true);

    tab.threadId = "t1";
    await wrapper.vm.$nextTick();
    expect(
      (wrapper.find(".ctx-window").element as HTMLButtonElement).disabled,
    ).toBe(false);

    await wrapper.vm.$nextTick();
    expect(
      (wrapper.find(".ctx-window").element as HTMLButtonElement).disabled,
    ).toBe(false);

    // 忙时双击同样触发压缩
    await wrapper.find(".ctx-window").trigger("dblclick");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/compact/start",
      params: { threadId: "t1" },
    });
  });

  it("压缩请求进行中按钮禁用，完成后恢复", async () => {
    let resolveInvoke!: (v: unknown) => void;
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "thread/compact/start") {
        return new Promise((r) => {
          resolveInvoke = r;
        });
      }
      return Promise.resolve({});
    });
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".ctx-window").trigger("dblclick");
    await flushPromises();
    expect(
      (wrapper.find(".ctx-window").element as HTMLButtonElement).disabled,
    ).toBe(true);
    resolveInvoke({});
    await flushPromises();
    expect(
      (wrapper.find(".ctx-window").element as HTMLButtonElement).disabled,
    ).toBe(false);
    expect(store.toast).toContain("已开始压缩上下文");
  });

  it("压缩失败时 toast 错误", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "thread/compact/start") {
        throw new Error("压缩失败");
      }
      return {};
    });
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.find(".ctx-window").trigger("dblclick");
    await flushPromises();
    expect(store.toast).toContain("压缩失败");
  });
});

describe("ComposerBar 会话 token 用量芯片", () => {
  let wrapper: VueWrapper | null = null;
  let tab: SessionTab;

  beforeEach(() => {
    __resetSessionTabsForTest();
    tab = defaultTab();
    tab.threadId = "t1";
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  it("输入/输出都有时在上下文窗口左侧渲染，缺失或缺一则不渲染", async () => {
    tab.threadTokenUsage = {
      used: 5000,
      window: 10000,
      input: 12000,
      output: 34000,
    };
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.vm.$nextTick();

    const token = wrapper.find(".token-usage-chip");
    expect(token.exists()).toBe(true);
    expect(token.findAll(".token-usage-part svg")).toHaveLength(2);
    expect(token.findAll(".token-usage-part").map((x) => x.text())).toEqual([
      "12K",
      "34K",
    ]);
    expect(token.text()).toContain("12K");
    expect(token.text()).toContain("34K");

    // 位于上下文窗口按钮左侧
    const right = wrapper.find(".composer-right").element;
    const tokenEl = right.querySelector(".token-usage-chip");
    const ctxEl = right.querySelector(".ctx-window");
    expect(tokenEl).toBeTruthy();
    expect(ctxEl).toBeTruthy();
    expect([...right.children].indexOf(tokenEl!)).toBeLessThan(
      [...right.children].indexOf(ctxEl!),
    );

    // 缺 input/output 任一即隐藏
    tab.threadTokenUsage = { used: 5000, window: 10000 };
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".token-usage-chip").exists()).toBe(false);
  });

  it("纯信息展示：非 button，双击不触发压缩", async () => {
    tab.threadTokenUsage = {
      used: 5000,
      window: 10000,
      input: 12000,
      output: 34000,
    };
    wrapper = mount(ComposerBar, { props: { tab } });
    await wrapper.vm.$nextTick();

    const token = wrapper.find(".token-usage-chip");
    expect(token.element.tagName).toBe("SPAN");
    expect(token.attributes("aria-label")).toContain("输入");
    expect(token.attributes("aria-label")).toContain("输出");

    await token.trigger("dblclick");
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "codex_rpc",
      expect.objectContaining({ method: "thread/compact/start" }),
    );
    expect(store.toast).not.toContain("已开始压缩上下文");
  });
});

describe("ComposerBar 任务目标芯片", () => {
  let wrapper: VueWrapper | null = null;
  let tab: SessionTab;

  beforeEach(() => {
    __resetSessionTabsForTest();
    tab = defaultTab();
    activeSessionTab()?.attachments.splice(0);
    store.workspace = "D:/repo";
    store.settings.enter_to_send = true;
    store.confirm = null;
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
    mockRpc(false);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  it("无目标时只有旗子（无角标），点击勾选目标 flag", async () => {
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    expect(wrapper.find(".goal-chip").exists()).toBe(true);
    expect(wrapper.find(".goal-label").exists()).toBe(false);
    expect(wrapper.find(".goal-chip.has-goal").exists()).toBe(false);
    expect(wrapper.find(".goal-check").exists()).toBe(false);
    const btn = wrapper.find(".goal-icon-btn");
    expect(btn.attributes("disabled")).toBeUndefined();
    expect(
      (btn.element as HTMLButtonElement).getAttribute("aria-pressed"),
    ).toBe("false");
    await btn.trigger("click");
    await flushPromises();
    expect(activeSessionTab()?.goalArmed).toBe(true);
    expect(wrapper.find(".goal-icon-btn.has-goal").exists()).toBe(true);
    expect(wrapper.find(".goal-chip.has-goal").exists()).toBe(true);
    const armedBadge = wrapper.find(".goal-check");
    expect(armedBadge.exists()).toBe(true);
    expect(armedBadge.text()).toBe("");
    expect(wrapper.find(".goal-icon-btn.status-active").exists()).toBe(false);
    expect(
      (wrapper.find(".goal-icon-btn").element as HTMLButtonElement).getAttribute(
        "aria-pressed",
      ),
    ).toBe("true");
    expect(wrapper.find(".goal-menu").exists()).toBe(false);
  });

  it("已勾选未发送时再次点击：取消勾选（无目标值可清）", async () => {
    tab.goalArmed = true;
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    await wrapper.vm.$nextTick();
    await wrapper.find(".goal-icon-btn").trigger("click");
    await flushPromises();
    expect(activeSessionTab()?.goalArmed).toBe(false);
    expect(activeSessionTab()?.goalText).toBeNull();
  });

  it("回合运行中且无目标：目标按钮隐藏（不可勾选）", async () => {
    tab.turnActive = true;
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    expect(wrapper.find(".goal-chip").exists()).toBe(false);
    expect(wrapper.find(".goal-icon-btn").exists()).toBe(false);
    expect(activeSessionTab()?.goalArmed).toBe(false);
  });

  it("目标进行中：出现角标且挂 status-active（呼吸动画）", async () => {
    tab.goalText = "目标";
    tab.goalStatus = "active";
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    expect(wrapper.find(".goal-icon-btn.status-active").exists()).toBe(true);
    const badge = wrapper.find(".goal-check");
    expect(badge.exists()).toBe(true);
    expect(badge.text()).toBe("");
  });

  it("有目标时点击旗子直接取消目标（不弹确认、不开弹层）", async () => {
    tab.threadId = "t1";
    tab.goalText = "目标";
    tab.goalStatus = "active";
    mockedInvoke.mockResolvedValue(undefined);
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    await wrapper.find(".goal-icon-btn").trigger("click");
    await flushPromises();
    expect(store.confirm).toBeNull();
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", { threadId: "t1" });
    expect(activeSessionTab()?.goalText).toBeNull();
    expect(activeSessionTab()?.goalStatus).toBeNull();
    expect(activeSessionTab()?.goalArmed).toBe(false);
  });

  it("勾选后发送首条消息：纯文本成为目标并复位勾选态", async () => {
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    await wrapper.find(".goal-icon-btn").trigger("click");
    await typeInEditor("修复登录流程");
    await wrapper.find("button.send-btn").trigger("click");
    await flushPromises();
    expect(activeSessionTab()?.goalArmed).toBe(false);
    expect(activeSessionTab()?.goalText).toBe("修复登录流程");
    expect(activeSessionTab()?.goalStatus).toBeNull();
    expect(mockedSendPrompt).toHaveBeenCalled();
    expect(mockedSendPrompt.mock.calls[0][0]).toContain("修复登录流程");
  });

  it("计划模式下发送消息：不消费勾选，目标保持待首条执行消息", async () => {
    tab.collaborationMode = "plan";
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    await wrapper.find(".goal-icon-btn").trigger("click");
    await typeInEditor("制定发布计划");
    await wrapper.find("button.send-btn").trigger("click");
    await flushPromises();
    expect(activeSessionTab()?.goalArmed).toBe(true);
    expect(activeSessionTab()?.goalText).toBeNull();
    expect(activeSessionTab()?.goalStatus).toBeNull();
    expect(mockedSendPrompt).toHaveBeenCalled();
  });

  it("长目标 tooltip 截断到 120 字符预览", async () => {
    tab.threadId = "t1";
    tab.goalText = "修".repeat(200);
    tab.goalStatus = "active";
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    const label = wrapper.find(".goal-icon-btn").attributes("aria-label") ?? "";
    expect(label).toContain("…");
    expect(label).toContain("进行中");
    expect(label.length).toBeLessThan(200);
    expect(label).not.toContain("修".repeat(200));
  });

  it("勾选后发送纯附件（无纯文本）：不设目标，勾选态保持", async () => {
    mockRpc(true);
    await ensureThreadPlugins(tab);
    wrapper = mount(ComposerBar, { props: { tab } });
    await typeInEditor("@a.cs");
    await waitSearch();
    const row = wrapper
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes("a.cs"));
    expect(row).toBeTruthy();
    await row!.trigger("click");
    await flushPromises();
    await wrapper.find(".goal-icon-btn").trigger("click");
    await wrapper.find("button.send-btn").trigger("click");
    await flushPromises();
    expect(activeSessionTab()?.goalArmed).toBe(true);
    expect(activeSessionTab()?.goalText).toBeNull();
    expect(mockedSendPrompt).toHaveBeenCalled();
    expect(mockedSendPrompt.mock.calls[0][0]).toContain("a.cs");
  });

});

describe("ComposerBar 权限与草稿会话私有", () => {
  let wrapper: VueWrapper | null = null;

  function makeTab(over: Partial<SessionTab> = {}): SessionTab {
    return {
      id: "s1",
      kind: "chat",
      title: "会话",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      collaborationMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: null,
      workspace: null,
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: null,
      interactions: [],
      ...over,
    };
  }

  beforeEach(() => {
    activeSessionTab()?.attachments.splice(0);
    store.workspace = "D:/repo";
    store.settings.enter_to_send = true;
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
    mockRpc(false);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  it("空闲时权限按钮可点击并切换模式", async () => {
    wrapper = mount(ComposerBar, { props: { tab: defaultTab() } });
    await flushPromises();
    const chip = wrapper.find(".perm-chip");
    expect(chip.attributes("disabled")).toBeUndefined();
    await chip.trigger("click");
    await flushPromises();
    expect(wrapper.find(".popup-menu").exists()).toBe(true);
    const fullAccessItem = wrapper
      .findAll(".mode-menu-item")
      .find((w) => w.text().includes("完全访问"))!;
    await fullAccessItem.trigger("click");
    await flushPromises();
    expect(activeSessionTab()?.permissionMode).toBe("full-access");
    expect(wrapper.find(".popup-menu").exists()).toBe(false);
  });

  it("回合进行中权限按钮禁用", async () => {
    const tab = defaultTab();
    tab.turnActive = true;
    wrapper = mount(ComposerBar, { props: { tab } });
    await flushPromises();
    expect(wrapper.find(".perm-chip").attributes("disabled")).toBeDefined();
    await wrapper.find(".perm-chip").trigger("click");
    expect(wrapper.find(".popup-menu").exists()).toBe(false);
  });

  it("标签激活时恢复草稿文本与附件区", async () => {
    const draft: UserInput = {
      type: "mention",
      name: "a.cs",
      path: "D:/repo/src/a.cs",
    };
    const tab = makeTab({
      draftJson: JSON.stringify({
        type: "doc",
        content: [
          {
            type: "paragraph",
            content: [{ type: "text", text: "草稿内容" }],
          },
        ],
      }),
      draftAttachments: [draft],
    });
    __resetSessionTabsForTest();
    tabs.push(tab);
    activeTabId.value = "s1";
    wrapper = mount(ComposerBar, {
      props: { tab, active: true },
    });
    await flushPromises();
    const ed = getEditor();
    const text = ed.getText();
    expect(text).toContain("草稿内容");
    expect(activeSessionTab()?.attachments).toHaveLength(1);
    expect(activeSessionTab()?.attachments[0]).toMatchObject({
      type: "mention",
      path: "D:/repo/src/a.cs",
    });
  });

  it("删空内容切走再切回不恢复旧草稿", async () => {
    const tab = makeTab({
      draftJson: JSON.stringify({
        type: "doc",
        content: [
          {
            type: "paragraph",
            content: [{ type: "text", text: "旧草稿" }],
          },
        ],
      }),
    });
    wrapper = mount(ComposerBar, {
      props: { tab, active: true },
    });
    await flushPromises();
    expect(getEditor().getText()).toContain("旧草稿");

    // 删空内容
    getEditor().commands.setContent("");
    await flushPromises();
    // 切走（快照）再切回（恢复）
    await wrapper.setProps({ active: false });
    await flushPromises();
    await wrapper.setProps({ active: true });
    await flushPromises();

    expect(getEditor().getText()).toBe("");
    expect(tab.draftJson).toBe("");
  });
});

describe("ComposerBar 多会话附件路由（资源面板 @ 入口）", () => {
  let wrapperA: VueWrapper | null = null;
  let wrapperB: VueWrapper | null = null;

  function makeTab(id: string): SessionTab {
    return {
      id,
      kind: "chat",
      title: "会话",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      collaborationMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: null,
      workspace: null,
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
  }

  function makeAttachment(name: string, path: string): UserInput {
    return { type: "mention", name, path };
  }

  beforeEach(() => {
    activeSessionTab()?.attachments.splice(0);
    store.workspace = "D:/repo";
    store.settings.enter_to_send = true;
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
    mockRpc(false);
    __resetSessionTabsForTest();
  });

  afterEach(() => {
    wrapperB?.unmount();
    wrapperB = null;
    wrapperA?.unmount();
    wrapperA = null;
  });

  it("两个会话同时挂载：附件路由到活动会话 A，不进入最后挂载的 B", async () => {
    const tabA = makeTab("tab-a");
    const tabB = makeTab("tab-b");
    tabs.push(tabA, tabB);
    activeTabId.value = "tab-a";

    wrapperA = mount(ComposerBar, {
      props: { tab: tabA, active: true },
    });
    wrapperB = mount(ComposerBar, {
      props: { tab: tabB, active: false },
    });
    await flushPromises();

    const a = makeAttachment("a.txt", "D:/repo/a.txt");
    expect(addAttachmentToActiveSession(a)).toBe(true);
    // 只写入 A 的输入区并同步 store；B 完全不受影响
    expect(activeSessionTab()?.attachments).toEqual([a]);
    expect(tabA.draftAttachments).toEqual([a]);
    expect(tabB.draftAttachments).toEqual([]);
  });

  it("切换活动会话后：附件路由到新的活动会话 B", async () => {
    const tabA = makeTab("tab-a");
    const tabB = makeTab("tab-b");
    tabs.push(tabA, tabB);
    activeTabId.value = "tab-a";

    wrapperA = mount(ComposerBar, {
      props: { tab: tabA, active: true },
    });
    wrapperB = mount(ComposerBar, {
      props: { tab: tabB, active: false },
    });
    await flushPromises();

    activeTabId.value = "tab-b";
    const b = makeAttachment("b.cs", "D:/repo/src/b.cs");
    expect(addAttachmentToActiveSession(b)).toBe(true);
    expect(activeSessionTab()?.attachments).toEqual([b]);
    expect(tabB.draftAttachments).toEqual([b]);
    expect(tabA.draftAttachments).toEqual([]);
  });

  it("卸载后注销：附件不再路由到该会话", async () => {
    const tabA = makeTab("tab-a");
    tabs.push(tabA);
    activeTabId.value = "tab-a";

    wrapperA = mount(ComposerBar, {
      props: { tab: tabA, active: true },
    });
    await flushPromises();
    wrapperA.unmount();
    wrapperA = null;

    const a = makeAttachment("a.txt", "D:/repo/a.txt");
    expect(addAttachmentToActiveSession(a)).toBe(false);
    expect(activeSessionTab()?.attachments).toEqual([]);
    expect(tabA.draftAttachments).toEqual([]);
  });
});
