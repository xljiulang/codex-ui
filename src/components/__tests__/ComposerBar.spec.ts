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
import type { UserInput } from "../../lib/types";

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
        {
          id: "unused@openai-curated",
          name: "unused",
          installed: false,
          enabled: false,
          source: { path: "C:/Users/x/.codex/plugins/cache/unused" },
          interface: {
            displayName: "Unused",
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

function pluginListCalls(): number {
  return mockedInvoke.mock.calls.filter(
    ([cmd, args]) =>
      cmd === "codex_rpc" &&
      (args as { method?: string } | undefined)?.method === "plugin/list",
  ).length;
}

describe("ComposerBar @ 合并菜单 + $ 技能菜单", () => {
  let wrapper: VueWrapper | null = null;

  beforeEach(() => {
    store.attachments.splice(0);
    store.threadPlugins = {};
    store.currentThreadId = null;
    store.server.workspace = "D:/repo";
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    mockedInvoke.mockReset();
    mockedSendPrompt.mockReset();
    mockRpc(false);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  const ta = () => wrapper!.find("textarea");
  const menuLabels = () =>
    wrapper!
      .findAll(".mention-menu button.menu-item .menu-item-label")
      .map((b) => b.text().trim());

  // ---------------- @ 合并菜单 ----------------

  it("输入 @ 弹出菜单：固定行在前、插件列表在后，且 + 按钮已移除", async () => {
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await ta().setValue("@");
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
    const labels = menuLabels();
    expect(labels.slice(0, 2)).toEqual(["选择文件…", "选择文件夹…"]);
    expect(labels).toContain("Documents");
    expect(labels).toContain("PDF");
    expect(labels.indexOf("选择文件夹…")).toBeLessThan(
      labels.indexOf("Documents"),
    );
    // 未安装/未启用的插件被过滤
    expect(labels).not.toContain("Unused");
    // + 添加内容按钮已移除
    expect(wrapper.find("button.plus-btn").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("添加内容");
  });

  it("输入 token 联合搜索：命中插件在命中文件前", async () => {
    mockRpc(true);
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await ta().setValue("@doc");
    await waitSearch();
    const labels = menuLabels();
    expect(labels).toContain("a.cs");
    expect(labels).toContain("Documents");
    expect(labels).not.toContain("PDF");
    expect(labels.indexOf("选择文件夹…")).toBeLessThan(
      labels.indexOf("Documents"),
    );
    expect(labels.indexOf("Documents")).toBeLessThan(labels.indexOf("a.cs"));
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "fuzzyFileSearch",
      params: {
        query: "doc",
        roots: ["D:/repo"],
        cancellationToken: null,
      },
    });
  });

  it("无匹配时显示无匹配文件与无匹配插件提示", async () => {
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await ta().setValue("@zzz");
    await waitSearch();
    const menu = wrapper.find(".mention-menu");
    expect(menu.text()).toContain("无匹配文件");
    expect(menu.text()).toContain("无匹配插件");
  });

  it("插件行渲染接口图标，无图标插件回退首字母占位", async () => {
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await ta().setValue("@");
    await flushPromises();
    const icons = wrapper.findAll(".mention-menu .menu-item-icon img");
    expect(icons.length).toBe(1);
    expect(icons[0].attributes("src")).toContain(
      "asset://mock/C:/Users/x/.codex/plugins/cache/documents/assets/icon.png",
    );
    const fallbacks = wrapper.findAll(".plugin-icon-fallback");
    expect(fallbacks.length).toBe(1);
    expect(fallbacks[0].text()).toBe("P");
    // 图标加载失败后回退占位
    await icons[0].trigger("error");
    await flushPromises();
    expect(wrapper.findAll(".plugin-icon-fallback").length).toBe(2);
  });

  it("对话级缓存：首次进入预加载、菜单只读、切换对话取各自缓存", async () => {
    mockRpc(true);
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    expect(pluginListCalls()).toBe(1);
    wrapper = mount(ComposerBar);
    await ta().setValue("@doc");
    await waitSearch();
    expect(pluginListCalls()).toBe(1); // 菜单打开不重复请求

    // 切换到已缓存的另一对话：显示该对话自己的插件缓存，不再请求
    store.threadPlugins["t2"] = {
      plugins: [
        {
          id: "t2p",
          name: "t2plugin",
          displayName: "T2 Plugin",
          description: "",
          path: "C:/t2",
          iconPath: "",
          iconUrl: "",
          brandColor: "",
        },
      ],
      loaded: true,
    };
    store.currentThreadId = "t2";
    await ta().setValue("@");
    await flushPromises();
    const labels = menuLabels();
    expect(labels).toContain("T2 Plugin");
    expect(labels).not.toContain("Documents");
    expect(pluginListCalls()).toBe(1);
  });

  it("选中插件生成 skill 附件且不调用 skills/list", async () => {
    mockRpc(true);
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await ta().setValue("@doc");
    await waitSearch();
    // @ 菜单不展示技能项（插件数据来自 plugin/list 对话级缓存）
    expect(menuLabels()).not.toContain("csharp-code-rules");
    const row = wrapper
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes("Documents"));
    expect(row).toBeTruthy();
    await row!.trigger("click");
    await flushPromises();
    expect(store.attachments).toEqual([
      {
        type: "skill",
        name: "documents",
        path: "C:/Users/x/.codex/plugins/cache/documents",
      },
    ]);
    expect((ta().element as HTMLTextAreaElement).value).toBe("");
    // 附件标签沿用既有显示（插件仍为 $ 前缀）
    expect(wrapper.text()).toContain("$documents");
  });

  it("键盘导航：↑↓ 移动高亮后 Enter 选中插件而不发送", async () => {
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await ta().setValue("@doc");
    await waitSearch();
    await ta().trigger("keydown", { key: "ArrowDown" });
    await ta().trigger("keydown", { key: "ArrowDown" });
    await ta().trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(mockedSendPrompt).not.toHaveBeenCalled();
    expect(store.attachments).toEqual([
      {
        type: "skill",
        name: "documents",
        path: "C:/Users/x/.codex/plugins/cache/documents",
      },
    ]);
  });

  it("同一条消息混合文件与插件附件", async () => {
    mockRpc(true);
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await ta().setValue("@a.cs");
    await waitSearch();
    const fileRow = wrapper
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes("a.cs"));
    await fileRow!.trigger("click");
    await flushPromises();

    await ta().setValue("@doc");
    await waitSearch();
    const pluginRow = wrapper
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes("Documents"));
    await pluginRow!.trigger("click");
    await flushPromises();

    expect(store.attachments.map((a) => a.type)).toEqual(["mention", "skill"]);
    await ta().setValue("混合使用测试");
    await wrapper.find("button.send-btn").trigger("click");
    expect(mockedSendPrompt).toHaveBeenCalledWith("混合使用测试", false);
    expect(store.attachments).toHaveLength(2);
  });

  it("@ 菜单打开时 ↑↓ 定位后 Enter 选中高亮文件而不是发送", async () => {
    mockRpc(true);
    await ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY);
    wrapper = mount(ComposerBar);
    await ta().setValue("@a.cs");
    await waitSearch();
    await ta().trigger("keydown", { key: "ArrowDown" });
    await ta().trigger("keydown", { key: "ArrowDown" });
    await ta().trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(mockedSendPrompt).not.toHaveBeenCalled();
    expect(store.attachments).toEqual([
      { type: "mention", name: "a.cs", path: "D:/repo/src/a.cs" },
    ]);
  });

  it("@ 空 token 时 Enter 打开本地文件选择器（记录性断言）", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("@");
    await flushPromises();
    await ta().trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(mockedSendPrompt).not.toHaveBeenCalled();
    expect(mockedInvoke).toHaveBeenCalledWith("pick_files", {
      multiple: true,
      initialDir: "D:/repo",
    });
  });

  // ---------------- $ 技能菜单 ----------------

  it("输入 $ 弹出技能菜单并加载技能列表", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("$");
    await flushPromises();
    const menu = wrapper.find(".mention-menu");
    expect(menu.exists()).toBe(true);
    expect(menu.text()).toContain("技能");
    expect(menu.text()).toContain("csharp-code-rules");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "skills/list",
      params: {},
    });
  });

  it("$ token 过滤技能列表，无匹配时提示", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("$csharp");
    await flushPromises();
    let menu = wrapper.find(".mention-menu");
    expect(menu.text()).toContain("csharp-code-rules");
    expect(menu.text()).not.toContain("ida-pro-mcp");
    await ta().setValue("$zzz");
    await flushPromises();
    menu = wrapper.find(".mention-menu");
    expect(menu.text()).toContain("无匹配技能");
  });

  it("$ 菜单 Enter 选中高亮技能并生成附件（不发送）", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("$csharp-code-rules");
    await flushPromises();
    await ta().trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(mockedSendPrompt).not.toHaveBeenCalled();
    expect(store.attachments).toEqual([
      {
        type: "skill",
        name: "csharp-code-rules",
        path: "C:/Users/x/.codex/skills/csharp-code-rules/SKILL.md",
      },
    ]);
  });

  it("$ 菜单 ↑↓ 移动高亮后 Enter 选中对应技能", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("$");
    await flushPromises();
    await ta().trigger("keydown", { key: "ArrowDown" });
    await ta().trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(store.attachments).toEqual([
      {
        type: "skill",
        name: "ida-pro-mcp:idapython",
        path: "C:/Users/x/.codex/plugins/cache/mrexodia/ida-pro-mcp/0.1.0/skills/idapython/SKILL.md",
      },
    ]);
  });

  it("同一条消息混合 @ 与 $，发送时带两个附件", async () => {
    mockRpc(true);
    wrapper = mount(ComposerBar);
    await ta().setValue("@a.cs");
    await waitSearch();
    const fileRow = wrapper
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes("a.cs"));
    await fileRow!.trigger("click");
    await flushPromises();

    await ta().setValue("$csharp-code-rules");
    await flushPromises();
    const skillRow = wrapper
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes("csharp-code-rules"));
    await skillRow!.trigger("click");
    await flushPromises();

    expect(store.attachments.map((a) => a.type)).toEqual(["mention", "skill"]);
    await ta().setValue("混合使用测试");
    await wrapper.find("button.send-btn").trigger("click");
    expect(mockedSendPrompt).toHaveBeenCalledWith("混合使用测试", false);
    expect(store.attachments).toHaveLength(2);
  });

  // ---------------- 通用边界 ----------------

  it("词中 @ 不弹菜单，空格后 @ 弹菜单", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("看下@file");
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(false);
    await ta().setValue("看下 @file.txt");
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
  });

  it("仅附件无文本时发送按钮可用并发送", async () => {
    wrapper = mount(ComposerBar);
    store.attachments.push({
      type: "mention",
      name: "a.cs",
      path: "D:/repo/src/a.cs",
    } satisfies UserInput);
    await flushPromises();
    const btn = wrapper.find("button.send-btn");
    expect(btn.attributes("disabled")).toBeUndefined();
    await btn.trigger("click");
    expect(mockedSendPrompt).toHaveBeenCalledTimes(1);
  });

  it("Esc 关闭菜单，移除附件标签生效", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("@");
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(false);

    store.attachments.push({
      type: "mention",
      name: "a.cs",
      path: "D:/repo/src/a.cs",
    } satisfies UserInput);
    await flushPromises();
    expect(wrapper.text()).toContain("@a.cs");
    await wrapper.find(".attachment-chip button").trigger("click");
    expect(store.attachments).toHaveLength(0);
  });
});
