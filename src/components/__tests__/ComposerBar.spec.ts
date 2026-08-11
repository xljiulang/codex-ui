import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
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
