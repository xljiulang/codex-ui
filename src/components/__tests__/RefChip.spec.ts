import { describe, expect, it, beforeEach, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

import RefChip from "../RefChip.vue";
import { NEW_CHAT_PLUGIN_KEY, store } from "../../composables/useCodex";

const waitTick = () => new Promise((r) => setTimeout(r, 20));

function tooltipText(): string {
  return document.body.querySelector(".ref-tooltip")?.textContent ?? "";
}

describe("RefChip 自定义悬浮卡片", () => {
  beforeEach(() => {
    document.body.innerHTML = ""; // 清掉跨用例残留的 Teleport 悬浮卡片
    store.currentThreadId = null;
    store.threadPlugins = {};
    store.skills = [];
    store.skillsLoaded = false;
    store.threadPlugins[NEW_CHAT_PLUGIN_KEY] = {
      loaded: true,
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          displayName: "Documents",
          description: "创建和编辑文档工件",
          path: "C:/x/documents",
          iconPath: "",
          iconUrl: "",
          brandColor: "",
        },
      ],
    };
    store.skills = [
      {
        name: "csharp-code-rules",
        key: "csharp-code-rules",
        path: "C:/x/skills/csharp-code-rules/SKILL.md",
        desc: "C# 代码规范说明",
        shortDesc: "C# 代码规范短说明",
      },
    ];
  });

  it("不再使用原生 title 属性", () => {
    const wrapper = mount(RefChip, {
      props: { path: "src/a.cs", label: "@a.cs", kind: "file" },
    });
    expect(wrapper.attributes("title")).toBeUndefined();
  });

  it("文件 chip 悬浮显示路径卡片", async () => {
    const wrapper = mount(RefChip, {
      props: { path: "src/a.cs", label: "@a.cs", kind: "file", delay: 0 },
    });
    await wrapper.find(".mention-inline").trigger("mouseenter");
    await waitTick();
    expect(tooltipText()).toContain("@a.cs");
    expect(tooltipText()).toContain("文件");
    expect(tooltipText()).toContain("src/a.cs");
    await wrapper.find(".mention-inline").trigger("mouseleave");
    await waitTick();
    expect(document.body.querySelector(".ref-tooltip")).toBeNull();
  });

  it("插件 chip 悬浮显示插件说明，缺失回退路径", async () => {
    const wrapper = mount(RefChip, {
      props: {
        path: "plugin://documents@openai-primary-runtime",
        label: "@documents",
        kind: "plugin",
        delay: 0,
      },
    });
    await wrapper.find(".mention-inline").trigger("mouseenter");
    await waitTick();
    expect(tooltipText()).toContain("创建和编辑文档工件");
    await wrapper.find(".mention-inline").trigger("mouseleave");
    await waitTick();

    const missing = mount(RefChip, {
      props: {
        path: "plugin://unknown@market",
        label: "@unknown",
        kind: "plugin",
        delay: 0,
      },
    });
    await missing.find(".mention-inline").trigger("mouseenter");
    await waitTick();
    expect(tooltipText()).toContain("plugin://unknown@market");
  });

  it("技能 chip 悬浮显示技能说明，缺失回退路径", async () => {
    const wrapper = mount(RefChip, {
      props: {
        path: "C:/x/skills/csharp-code-rules/SKILL.md",
        label: "$csharp-code-rules",
        kind: "skill",
        delay: 0,
      },
    });
    await wrapper.find(".mention-inline").trigger("mouseenter");
    await waitTick();
    expect(tooltipText()).toContain("C# 代码规范说明");
    await wrapper.find(".mention-inline").trigger("mouseleave");
    await waitTick();

    const missing = mount(RefChip, {
      props: { path: "C:/x/SKILL.md", label: "$zzz", kind: "skill", delay: 0 },
    });
    await missing.find(".mention-inline").trigger("mouseenter");
    await waitTick();
    expect(tooltipText()).toContain("C:/x/SKILL.md");
  });

  it("plugin:// chip 不可点击（渲染为 span）", () => {
    const wrapper = mount(RefChip, {
      props: {
        path: "plugin://documents@openai-primary-runtime",
        label: "@documents",
        kind: "plugin",
      },
    });
    expect(wrapper.find("button").exists()).toBe(false);
    expect(wrapper.find("span.mention-inline").exists()).toBe(true);
  });

  it("点击文件 chip 触发 reveal_path（测试钩子）", async () => {
    store.server.workspace = "D:/repo";
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__ = true;
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST_LOG__ = [];
    const wrapper = mount(RefChip, {
      props: { path: "src/a.cs", label: "@a.cs", kind: "file" },
    });
    await wrapper.find(".mention-inline").trigger("click");
    await flushPromises();
    const log = (window as unknown as Record<string, unknown>)
      .__CODEX_UI_TEST_LOG__ as { cmd: string; args: { path: string } }[];
    expect(log).toEqual([
      { cmd: "reveal_path", args: { path: "D:\\repo\\src\\a.cs" } },
    ]);
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__ = false;
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST_LOG__ = [];
  });
});
