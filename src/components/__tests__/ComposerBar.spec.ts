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
import { sendPrompt, store } from "../../composables/useCodex";
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

describe("ComposerBar @ 文件引用 / $ 技能引用", () => {
  let wrapper: VueWrapper | null = null;

  beforeEach(() => {
    store.attachments.splice(0);
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

  it("输入 @ 弹出文件引用菜单", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("@");
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
    expect(wrapper.text()).toContain("引用文件");
    expect(wrapper.text()).toContain("选择文件…");
    expect(wrapper.text()).toContain("选择文件夹…");
  });

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

  it("词中 @ 不弹菜单，空格后 @ 弹菜单", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("看下@file");
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(false);
    await ta().setValue("看下 @file.txt");
    await flushPromises();
    expect(wrapper.find(".mention-menu").exists()).toBe(true);
  });

  it("选中模糊搜索文件后生成 @ 附件并移除触发词", async () => {
    mockRpc(true);
    wrapper = mount(ComposerBar);
    await ta().setValue("@a.cs");
    await waitSearch();
    const row = wrapper
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes("a.cs"));
    expect(row).toBeTruthy();
    await row!.trigger("click");
    await flushPromises();
    expect(store.attachments).toEqual([
      { type: "mention", name: "a.cs", path: "D:/repo/src/a.cs" },
    ]);
    expect((ta().element as HTMLTextAreaElement).value).toBe("");
    expect(wrapper.text()).toContain("@a.cs");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "fuzzyFileSearch",
      params: {
        query: "a.cs",
        roots: ["D:/repo"],
        cancellationToken: null,
      },
    });
  });

  it("选中技能后生成 $ 附件并移除触发词", async () => {
    wrapper = mount(ComposerBar);
    await ta().setValue("$csharp-code-rules");
    await flushPromises();
    const row = wrapper
      .findAll(".mention-menu button.menu-item")
      .find((b) => b.text().includes("csharp-code-rules"));
    expect(row).toBeTruthy();
    await row!.trigger("click");
    await flushPromises();
    expect(store.attachments).toEqual([
      {
        type: "skill",
        name: "csharp-code-rules",
        path: "C:/Users/x/.codex/skills/csharp-code-rules/SKILL.md",
      },
    ]);
    expect((ta().element as HTMLTextAreaElement).value).toBe("");
    expect(wrapper.text()).toContain("$csharp-code-rules");
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

  it("@ 菜单打开时 Enter 选中高亮行而不是发送", async () => {
    mockRpc(true);
    wrapper = mount(ComposerBar);
    await ta().setValue("@a.cs");
    await waitSearch();
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

  it("$ 菜单 Enter 选中高亮技能并生成附件（与 @ 一致）", async () => {
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
