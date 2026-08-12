import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";

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
  ensureThreadPlugins,
  NEW_CHAT_PLUGIN_KEY,
  sendPrompt,
  store,
} from "../../composables/useCodex";

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
    store.attachments.splice(0);
    store.threadPlugins = {};
    store.skills = [];
    store.skillsLoaded = false;
    store.currentThreadId = null;
    store.server.workspace = "D:/repo";
    store.currentThreadCwd = null;
    store.newChatCwd = null;
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
    wrapper = mount(ComposerBar);
    await flushPromises();
    expect(wrapper.find(".ProseMirror").exists()).toBe(true);
    expect(wrapper.find("textarea").exists()).toBe(false);
    expect(wrapper.find("button.plus-btn").exists()).toBe(false);
  });

  it("新会话态不再渲染输入框上方的项目目录行", () => {
    wrapper = mount(ComposerBar);
    expect(wrapper.find(".newchat-cwd-row").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("项目目录");
  });

  it("@ 空 token：菜单固定行在前、插件列表在后", async () => {
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
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
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await typeInEditor("@doc");
    await waitSearch();
    const labels = menuLabels();
    expect(labels).toContain("a.cs");
    expect(labels).toContain("Documents");
    expect(labels.indexOf("Documents")).toBeLessThan(labels.indexOf("a.cs"));
  });

  it("选中文件进附件区（不内联），触发词被移除且附件同步", async () => {
    mockRpc(true);
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await typeInEditor("看 @a.cs");
    await waitSearch();
    await clickMenuRow("a.cs");
    const ed = getEditor();
    expect(ed.getText().replace(/\s+/g, " ").trim()).toBe("看");
    expect(wrapper.find(".ref-chip").exists()).toBe(false);
    const rowChip = wrapper.find(".attachment-chip");
    expect(rowChip.exists()).toBe(true);
    expect(rowChip.text()).toContain("@a.cs");
    expect(store.attachments).toEqual([
      { type: "mention", name: "a.cs", path: "D:/repo/src/a.cs" },
    ]);
  });

  it("选中插件生成 @ 前缀 chip 与 source=plugin 附件", async () => {
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await typeInEditor("@doc");
    await waitSearch();
    await clickMenuRow("Documents");
    const chip = wrapper.find(".ref-chip");
    expect(chip.exists()).toBe(true);
    expect(chip.text()).toBe("@documents");
    expect(store.attachments).toEqual([
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
    wrapper = mount(ComposerBar);
    await typeInEditor("$csharp-code-rules");
    await flushPromises();
    await clickMenuRow("csharp-code-rules");
    const chip = wrapper.find(".ref-chip");
    expect(chip.exists()).toBe(true);
    expect(chip.text()).toBe("$csharp-code-rules");
    expect(store.attachments).toEqual([
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
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
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
    expect(store.attachments.map((a) => a.type)).toEqual(["skill", "mention"]);
  });

  it("发送时把内联引用写入 store.attachments 并调用 sendPrompt", async () => {
    mockRpc(true);
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
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
    expect(store.attachments).toEqual([
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
    wrapper = mount(ComposerBar);
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
    expect(store.attachments).toEqual([
      { type: "localImage", path: "D:/repo/pic.png" },
    ]);
  });

  it("@ 菜单打开时 Enter 选中高亮行而不是发送", async () => {
    mockRpc(true);
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
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
    wrapper = mount(ComposerBar);
    await typeInEditor("看下@file");
    expect(wrapper.find(".mention-menu").exists()).toBe(false);
    await typeInEditor(" 看下 @file.txt");
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
  });

  it("仅附件无文本时发送按钮可用", async () => {
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await typeInEditor("@doc");
    await waitSearch();
    await clickMenuRow("Documents");
    const btn = wrapper.find("button.send-btn");
    expect(btn.attributes("disabled")).toBeUndefined();
  });

  it("Esc 关闭菜单", async () => {
    wrapper = mount(ComposerBar);
    await typeInEditor("@");
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(false);
  });

  it("快捷发送开启：Ctrl+Enter 插入硬换行且不发送", async () => {
    wrapper = mount(ComposerBar);
    await typeInEditor("abc");
    await wrapper
      .find(".ProseMirror")
      .trigger("keydown", { key: "Enter", ctrlKey: true });
    await flushPromises();
    expect(JSON.stringify(getEditor().getJSON())).toContain("hardBreak");
    expect(mockedSendPrompt).not.toHaveBeenCalled();
  });

  it("快捷发送开启：Shift+Enter 无操作，普通 Enter 发送", async () => {
    wrapper = mount(ComposerBar);
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
    wrapper = mount(ComposerBar);
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
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
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
    store.attachments.splice(0);
    store.toast = "";
    store.threadPlugins = {};
    store.currentThreadId = null;
    store.server.workspace = "D:/repo";
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
  }

  it("粘贴截图位图（无原始路径）：落盘并生成 localImage 附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return [];
      if (cmd === "save_pasted_image") return "C:/tmp/paste/pasted-1-1.png";
      return {};
    });
    wrapper = mount(ComposerBar);
    await pasteItems([makeFileItem("clip.png", "image/png")]);

    expect(mockedInvoke).toHaveBeenCalledWith("save_pasted_image", {
      bytes: expect.any(Array),
      name: "pasted.png",
    });
    expect(store.attachments).toEqual([
      { type: "localImage", path: "C:/tmp/paste/pasted-1-1.png" },
    ]);
    expect(wrapper.find(".attachment-thumb").exists()).toBe(true);
    expect(getEditor().getText()).toBe("");
  });

  it("粘贴图片文件（原始路径可解析）：直接用原路径，不落盘", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return ["D:/repo/shot.png"];
      return {};
    });
    wrapper = mount(ComposerBar);
    await pasteItems([makeFileItem("shot.png", "image/png")]);

    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "save_pasted_image",
      expect.anything(),
    );
    expect(store.attachments).toEqual([
      { type: "localImage", path: "D:/repo/shot.png" },
    ]);
  });

  it("粘贴非图片文件（原始路径可解析）：生成 mention 附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return ["D:/repo/report.pdf"];
      return {};
    });
    wrapper = mount(ComposerBar);
    await pasteItems([makeFileItem("report.pdf", "application/pdf")]);

    expect(store.attachments).toEqual([
      { type: "mention", name: "report.pdf", path: "D:/repo/report.pdf" },
    ]);
    expect(wrapper.find(".attachment-chip").text()).toContain("@report.pdf");
  });

  it("粘贴非图片文件（无原始路径）：提示暂不支持且不加附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return [];
      return {};
    });
    wrapper = mount(ComposerBar);
    await pasteItems([makeFileItem("report.pdf", "application/pdf")]);

    expect(store.toast).toContain("暂不支持该粘贴");
    expect(store.attachments).toEqual([]);
  });

  it("粘贴超大图片（>20MB）：提示跳过且不加附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return [];
      return {};
    });
    wrapper = mount(ComposerBar);
    await pasteItems([
      makeFileItem("huge.png", "image/png", 21 * 1024 * 1024),
    ]);

    expect(store.toast).toContain("图片过大");
    expect(store.attachments).toEqual([]);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "save_pasted_image",
      expect.anything(),
    );
  });

  it("纯文本粘贴：走默认行为，不触发附件逻辑", async () => {
    wrapper = mount(ComposerBar);
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
    expect(store.attachments).toEqual([]);
  });

  it("混合粘贴：截图位图落盘 + 文件原路径，一次全部处理", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "clipboard_file_paths") return ["D:/repo/a.txt"];
      if (cmd === "save_pasted_image") return "C:/tmp/paste/pasted-2-2.png";
      return {};
    });
    wrapper = mount(ComposerBar);
    await pasteItems([
      makeFileItem("clip.png", "image/png"),
      makeFileItem("a.txt", "text/plain"),
    ]);

    expect(store.attachments).toEqual([
      { type: "localImage", path: "C:/tmp/paste/pasted-2-2.png" },
      { type: "mention", name: "a.txt", path: "D:/repo/a.txt" },
    ]);
    expect(mockedInvoke).toHaveBeenCalledTimes(2);
  });
});

describe("ComposerBar 拖放图片/文件", () => {
  let wrapper: VueWrapper | null = null;

  beforeEach(() => {
    store.attachments.splice(0);
    store.toast = "";
    store.threadPlugins = {};
    store.currentThreadId = null;
    store.server.workspace = "D:/repo";
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
    wrapper = mount(ComposerBar);
    await dropAndFlush([makeDropFile("shot.png", "image/png", 8, "D:/repo/shot.png")]);

    expect(store.attachments).toEqual([
      { type: "localImage", path: "D:/repo/shot.png" },
    ]);
  });

  it("拖入无路径的图片：落盘生成 localImage 附件", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "save_pasted_image") return "C:/tmp/drop/pasted-1-1.png";
      return {};
    });
    wrapper = mount(ComposerBar);
    await dropAndFlush([makeDropFile("clip.png", "image/png")]);

    expect(mockedInvoke).toHaveBeenCalledWith("save_pasted_image", {
      bytes: expect.any(Array),
      name: "pasted.png",
    });
    expect(store.attachments).toEqual([
      { type: "localImage", path: "C:/tmp/drop/pasted-1-1.png" },
    ]);
  });

  it("拖入带路径的非图片：生成 mention 附件", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar);
    await dropAndFlush([makeDropFile("a.txt", "text/plain", 8, "D:/repo/a.txt")]);

    expect(store.attachments).toEqual([
      { type: "mention", name: "a.txt", path: "D:/repo/a.txt" },
    ]);
  });

  it("拖入无路径的非图片：提示暂不支持拖放且不加附件", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar);
    await dropAndFlush([makeDropFile("a.txt", "text/plain")]);

    expect(store.toast).toContain("暂不支持该拖放");
    expect(store.attachments).toEqual([]);
  });

  it("纯文本拖放：不阻止默认行为，不生成附件", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar);
    await flushPromises();
    const ev = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(ev, "dataTransfer", {
      value: { files: [], items: [], types: [] },
    });
    wrapper!.find(".composer").element.dispatchEvent(ev);

    expect(ev.defaultPrevented).toBe(false);
    expect(store.attachments).toEqual([]);
  });

  it("Tauri 拖放事件：over 高亮、drop 路径生成附件（图片 localImage、文件 mention）", async () => {
    mockedInvoke.mockResolvedValue({});
    wrapper = mount(ComposerBar);
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
    expect(store.attachments).toEqual([
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
    store.attachments.splice(0);
    store.threadPlugins = {};
    store.currentThreadId = null;
    store.server.workspace = "D:/repo";
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
    const handle = wrapper!.find(".editor-resize-handle").element;
    handle.dispatchEvent(
      new PointerEvent("pointerdown", { clientY: 200, bubbles: true }),
    );
    window.dispatchEvent(new PointerEvent("pointermove", { clientY }));
    window.dispatchEvent(new PointerEvent("pointerup", {}));
    await flushPromises();
  }

  it("未拖拽时无内联高度；拖拽后设置高度，可继续以当前高度为基准增长", async () => {
    wrapper = mount(ComposerBar);
    await flushPromises();
    expect(rowStyle()).toBe("");

    await dragTo(100);
    const h1 = Number(
      /--editor-h:\s*(\d+)px/.exec(rowStyle())?.[1] ?? "0",
    );
    expect(h1).toBeGreaterThanOrEqual(96);

    await dragTo(50);
    const h2 = Number(
      /--editor-h:\s*(\d+)px/.exec(rowStyle())?.[1] ?? "0",
    );
    // 第二次拖拽以当前高度为基准继续增长
    expect(h2).toBeGreaterThan(h1);
  });

  it("向上拖超过窗口一半被夹紧到半屏（innerHeight/2 = 300）", async () => {
    wrapper = mount(ComposerBar);
    await dragTo(200 - 10000);
    expect(rowStyle()).toContain("--editor-h: 300px");
  });

  it("向下拖低于最低高度被夹紧到 96px", async () => {
    wrapper = mount(ComposerBar);
    await dragTo(200 + 10000);
    expect(rowStyle()).toContain("--editor-h: 96px");
  });
});
