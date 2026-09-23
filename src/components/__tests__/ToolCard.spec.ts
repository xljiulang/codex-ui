import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));

import { invoke } from "@tauri-apps/api/core";
import ToolCard from "../ToolCard.vue";
import type { ThreadItem } from "../../lib/types";
import { store } from "../../composables/useCodex";
import { __resetEditorTabsForTest } from "../../composables/useEditorTabs";
import {
  ICON_AGENTS,
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_CHECKLIST,
  ICON_EDIT,
  ICON_GLOBE,
  ICON_IMAGE,
  ICON_TERMINAL,
  ICON_TOOL,
} from "../../lib/icons";

const mockedInvoke = vi.mocked(invoke);

function makeItem(over: Partial<ThreadItem>): ThreadItem {
  return {
    id: "item-1",
    type: "commandExecution",
    command: "echo hi",
    cwd: "C:\\workspace",
    status: "in_progress",
    startedAtMs: 1000,
    aggregatedOutput: "",
    ...over,
  } as ThreadItem;
}

describe("ToolCard 实时耗时", () => {
  it("进行中显示实时计时，完成后用 durationMs 定格", async () => {
    const wrapper = mount(ToolCard, {
      props: { item: makeItem({ startedAtMs: Date.now() }) },
    });
    // 运行中状态文本只出现一次“进行中”，时间标签仅显示秒表
    expect(wrapper.text()).toMatch(/进行中/);
    expect(wrapper.text()).toMatch(/\d\d:\d\d\.\d/);
    expect(wrapper.text().match(/进行中/g)).toHaveLength(1);

    await wrapper.setProps({
      item: makeItem({
        status: "completed",
        durationMs: 1_234,
        exitCode: 0,
        aggregatedOutput: "hi\n",
        startedAtMs: 1000,
      }),
    });
    expect(wrapper.text()).toContain("耗时 1.2s");
    expect(wrapper.text()).not.toContain("进行中");
  });

  it("中断后显示已中断与固定耗时，不再实时跳动", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "interrupted",
          durationMs: 1_234,
          exitCode: null,
        }),
      },
    });
    expect(wrapper.text()).toContain("已中断");
    expect(wrapper.text()).toContain("耗时 1.2s");
    expect(wrapper.text()).not.toMatch(/\d\d:\d\d\.\d/);
  });

  it("失败状态显示退出码", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "failed",
          exitCode: 1,
          durationMs: 500,
          aggregatedOutput: "boom",
          startedAtMs: 1000,
        }),
      },
    });
    expect(wrapper.text()).toContain("失败");
    // 卡片默认折叠，展开后显示退出码
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_RIGHT,
    );
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_DOWN,
    );
    expect(wrapper.text()).toContain("退出码：1");
  });

  it("头部显示真实命令而非工作目录，并展示工作目录", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "completed",
          command:
            '"C:\\Program Files\\PowerShell\\7\\pwsh.exe" -Command whoami',
          commandActions: [{ command: "whoami", type: "unknown" }],
          cwd: "D:\\codex\\codex-ui",
          durationMs: 500,
        }),
      },
    });
    // 头部：执行命令 + 简洁命令（不再是工作目录）
    expect(wrapper.find(".assistant-card-title").text()).toBe("执行命令");
    expect(wrapper.find(".assistant-card-sub").text()).toBe("whoami");
    expect(wrapper.text()).not.toContain("D:\\codex\\codex-ui");
    // 展开后显示工作目录
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.text()).toContain("工作目录：D:\\codex\\codex-ui");
  });

  it("无 commandActions 时回退到完整命令行", () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "completed",
          command: "npm run build",
          commandActions: undefined,
          cwd: undefined,
          durationMs: 100,
        }),
      },
    });
    expect(wrapper.find(".assistant-card-sub").text()).toBe("npm run build");
  });

  it("webSearch 渲染结构化结果", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: {
          id: "w1",
          type: "webSearch",
          query: "rust tauri",
          results: [
            {
              title: "Tauri 官网",
              url: "https://tauri.app",
              snippet: "Rust 桌面框架",
            },
            { title: "第二条", url: "https://example.com" },
          ],
        } as ThreadItem,
      },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    const titles = wrapper.findAll(".web-result-title").map((t) => t.text());
    expect(titles).toContain("Tauri 官网");
    expect(titles).toContain("第二条");
    expect(wrapper.find(".web-result-snippet").text()).toContain(
      "Rust 桌面框架",
    );
  });

  it("收起输出/展开完整输出可切换", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "in_progress",
          startedAtMs: Date.now(),
          aggregatedOutput: "line1\nline2\nline3\nline4\nline5\n",
        }),
      },
    });
    // 默认折叠：不渲染输出区
    expect(wrapper.find(".tool-output").exists()).toBe(false);
    // 点击头部展开卡片，输出折叠为摘要
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".tool-output.collapsed").exists()).toBe(true);
    expect(wrapper.find(".tool-toggle").text()).toContain("展开完整输出");
    // 展开完整输出
    await wrapper.find(".tool-toggle").trigger("click");
    expect(wrapper.find(".tool-output.collapsed").exists()).toBe(false);
    expect(wrapper.find(".tool-toggle").text()).toContain("收起输出");
    // 再收起
    await wrapper.find(".tool-toggle").trigger("click");
    expect(wrapper.find(".tool-output.collapsed").exists()).toBe(true);
    expect(wrapper.find(".tool-toggle").text()).toContain("展开完整输出");
  });

  it("超长输出截断为末尾 5000 行并提示", async () => {
    const lines = Array.from({ length: 6000 }, (_, i) => `line-${i}`);
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "completed",
          durationMs: 100,
          aggregatedOutput: lines.join("\n") + "\n",
        }),
      },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".tool-output-cap").text()).toContain("5000");
    const text = wrapper.find(".tool-output").text();
    expect(text).toContain("line-5999");
    expect(text).not.toContain("line-0");
  });

  it("复制命令按钮写入剪贴板并反馈", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({ status: "completed", durationMs: 100 }),
      },
    });
    const btn = wrapper.find(".assistant-card-header .copy-btn");
    expect(btn.exists()).toBe(true);
    await btn.trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("echo hi");
    expect(btn.text()).toBe("已复制");
  });
});

describe("文件变更：打开独立 diff 窗口", () => {
  const REPLACE_DIFF = ["@@ -1,3 +1,3 @@", " a", "-b", "+X", " c"].join("\n");

  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(null);
    store.workspace = "D:/repo";
    store.toast = "";
    __resetEditorTabsForTest();
  });

  function changeItem(
    diff: string,
    path = "D:\\repo\\a.cs",
    kind: unknown = "update",
  ): ThreadItem {
    return {
      id: "f1",
      type: "fileChange",
      changes: [{ path, kind, diff }],
      status: "completed",
    } as ThreadItem;
  }

  function openCallParams() {
    const openCall = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "build_diff_preview",
    );
    return openCall?.[1] as
      | {
          params: {
            path: string;
            kind: string;
            diff: string;
            workspace: string;
          };
        }
      | undefined;
  }

  it("点击变更行（自带 diff）直接打开 diff 标签，不依赖 git", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "build_diff_preview") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(ToolCard, {
      props: { item: changeItem(REPLACE_DIFF, "D:\\repo\\a.cs", "update") },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    await wrapper.find(".change-row").trigger("click");
    await flushPromises();

    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "git_changes_diff",
      expect.anything(),
    );
    const params = openCallParams()?.params;
    expect(params?.path).toBe("D:\\repo\\a.cs");
    expect(params?.kind).toBe("modify");
    expect(params?.workspace).toBe("D:/repo");
    expect(params?.diff).toBe(REPLACE_DIFF);
    wrapper.unmount();
  });

  it("无 diff 的行点击不打开窗口", async () => {
    const wrapper = mount(ToolCard, {
      props: { item: changeItem("", "D:\\repo\\empty.cs", "update") },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    await wrapper.find(".change-row").trigger("click");
    await flushPromises();

    expect(openCallParams()).toBeUndefined();
    wrapper.unmount();
  });
});

describe("文件变更：折叠标题显示文件名", () => {
  function changeItem(
    changes: { path: string; kind: string; diff?: string }[],
  ): ThreadItem {
    return {
      id: "f3",
      type: "fileChange",
      changes,
      status: "completed",
    } as ThreadItem;
  }

  it("单文件时折叠标题显示完整路径", () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: changeItem([
          { path: "D:\\repo\\a.cs", kind: "update", diff: "x" },
        ]),
      },
    });
    expect(wrapper.find(".assistant-card-sub").text()).toBe("D:\\repo\\a.cs");
  });

  it("多文件时折叠标题显示第一个路径和文件总数", () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: changeItem([
          { path: "D:\\repo\\a.cs", kind: "update", diff: "x" },
          { path: "D:\\repo\\b.ts", kind: "add", diff: "y" },
          { path: "D:\\repo\\c.rs", kind: "delete", diff: "z" },
        ]),
      },
    });
    expect(wrapper.find(".assistant-card-sub").text()).toBe(
      "D:\\repo\\a.cs (等3个)",
    );
  });

  it("无变更时折叠标题为空", () => {
    const wrapper = mount(ToolCard, {
      props: { item: changeItem([]) },
    });
    expect(wrapper.find(".assistant-card-sub").text()).toBe("");
  });
});

describe("命令输出增量渲染", () => {
  const wait = () => new Promise((r) => setTimeout(r, 120));

  it("流式追加只追加尾部，不重复旧内容", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "in_progress",
          startedAtMs: Date.now(),
          aggregatedOutput: "line1",
        }),
      },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    await wait();
    expect(wrapper.find(".tool-output").text()).toContain("line1");

    await wrapper.setProps({
      item: makeItem({
        status: "in_progress",
        startedAtMs: Date.now(),
        aggregatedOutput: "line1\nline2",
      }),
    });
    await wait();
    const text = wrapper.find(".tool-output").text();
    expect(text).toContain("line2");
    expect(text.match(/line1/g)).toHaveLength(1);
  });

  it("条目整体替换后重置，不残留旧内容", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "completed",
          durationMs: 100,
          aggregatedOutput: "AAA",
        }),
      },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    await wait();
    expect(wrapper.find(".tool-output").text()).toContain("AAA");

    await wrapper.setProps({
      item: makeItem({
        status: "completed",
        durationMs: 100,
        aggregatedOutput: "BBB",
      }),
    });
    await wait();
    const text = wrapper.find(".tool-output").text();
    expect(text).toContain("BBB");
    expect(text).not.toContain("AAA");
  });
});

describe("ToolCard 新增条目类型", () => {
  it("imageGeneration 分发到 ImageGenerationCard", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: {
          id: "g1",
          type: "imageGeneration",
          status: "completed",
          revisedPrompt: "生成一个图标",
          result: "C:/tmp/icon.png",
        } as ThreadItem,
      },
    });
    expect(wrapper.text()).toContain("生成图片");
    expect(wrapper.text()).toContain("生成一个图标");
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".image-gen-card").exists()).toBe(true);
  });

  it("MCP 进度行与百分比条在展开后显示", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: {
          id: "m1",
          type: "mcpToolCall",
          server: "srv",
          tool: "t",
          status: "in_progress",
          progressText: "下载中",
          progressPercent: 40,
        } as ThreadItem,
      },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.text()).toContain("进度：下载中");
    const fill = wrapper.find(".tool-progress-fill");
    expect(fill.exists()).toBe(true);
    expect(fill.attributes("style")).toContain("width: 40%");
  });
});

describe("ToolCard 头部图标", () => {
  const cases: {
    name: string;
    item: ThreadItem;
    expected: string;
    title: string;
  }[] = [
    {
      name: "执行命令",
      item: makeItem({}),
      expected: ICON_TERMINAL,
      title: "执行命令",
    },
    {
      name: "MCP 工具调用",
      item: {
        id: "t1",
        type: "mcpToolCall",
        server: "srv",
        tool: "t",
      } as ThreadItem,
      expected: ICON_TOOL,
      title: "调用工具 srv::t",
    },
    {
      name: "动态工具调用",
      item: {
        id: "t2",
        type: "dynamicToolCall",
        namespace: "ns",
        tool: "t",
      } as ThreadItem,
      expected: ICON_TOOL,
      title: "调用工具 ns/t",
    },
    {
      name: "子代理协作",
      item: {
        id: "t3",
        type: "collabAgentToolCall",
        tool: "spawn_agent",
      } as ThreadItem,
      expected: ICON_AGENTS,
      title: "子代理协作",
    },
    {
      name: "网络搜索",
      item: { id: "t4", type: "webSearch", query: "q" } as ThreadItem,
      expected: ICON_GLOBE,
      title: "搜索网络",
    },
    {
      name: "文件变更",
      item: {
        id: "t5",
        type: "fileChange",
        changes: [],
        status: "completed",
      } as ThreadItem,
      expected: ICON_EDIT,
      title: "文件变更",
    },
    {
      name: "生成图片",
      item: {
        id: "t6",
        type: "imageGeneration",
        status: "completed",
      } as ThreadItem,
      expected: ICON_IMAGE,
      title: "生成图片",
    },
    {
      name: "计划",
      item: { id: "t7", type: "todoList", items: [] } as ThreadItem,
      expected: ICON_CHECKLIST,
      title: "计划",
    },
  ];

  it.each(cases)(
    "$name 渲染对应图标且标题不变",
    ({ item, expected, title }) => {
      const wrapper = mount(ToolCard, { props: { item } });
      const icon = wrapper.find(".assistant-card-icon");
      expect(icon.exists()).toBe(true);
      expect(icon.attributes("aria-hidden")).toBe("true");
      expect(icon.find("path").attributes("d")).toBe(expected);
      expect(wrapper.find(".assistant-card-title").text()).toBe(title);
    },
  );

  it("未知类型回退扳手图标", () => {
    const wrapper = mount(ToolCard, {
      props: { item: { id: "u1", type: "unknown" } as unknown as ThreadItem },
    });
    expect(wrapper.find(".assistant-card-icon path").attributes("d")).toBe(
      ICON_TOOL,
    );
    expect(wrapper.find(".assistant-card-title").text()).toBe("工具");
  });
});

describe("dynamicToolCall 结果渲染", () => {
  it("contentItems 渲染到 tool-output，空参数不显示 tool-json", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: {
          id: "d1",
          type: "dynamicToolCall",
          namespace: "codexui",
          tool: "compact_context",
          arguments: {},
          status: "completed",
          contentItems: [{ type: "inputText", text: "已请求压缩上下文" }],
          success: true,
        } as ThreadItem,
      },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".tool-json").exists()).toBe(false);
    expect(wrapper.find(".tool-output").text()).toContain("已请求压缩上下文");
  });

  it("多个 contentItems 按行拼接", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: {
          id: "d2",
          type: "dynamicToolCall",
          namespace: "codexui",
          tool: "get_usage",
          arguments: {},
          status: "completed",
          contentItems: [
            { type: "inputText", text: "第一行" },
            { type: "inputText", text: "第二行" },
          ],
          success: true,
        } as ThreadItem,
      },
    });
    await wrapper.find(".assistant-card-toggle").trigger("click");
    const text = wrapper.find(".tool-output").text();
    expect(text).toContain("第一行");
    expect(text).toContain("第二行");
  });
});
