import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));

import { invoke } from "@tauri-apps/api/core";
import ToolCard from "../ToolCard.vue";
import type { ThreadItem } from "../../lib/types";
import { store } from "../../composables/useCodex";

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

  it("失败状态显示退出码", () => {
    const wrapper = mount(
      ToolCard,
      {
        props: {
          item: makeItem({
            status: "failed",
            exitCode: 1,
            durationMs: 500,
            aggregatedOutput: "boom",
            startedAtMs: 1000,
          }),
        },
      },
    );
    expect(wrapper.text()).toContain("失败");
    expect(wrapper.text()).toContain("退出码：1");
  });

  it("头部显示真实命令而非工作目录，并展示工作目录", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: makeItem({
          status: "completed",
          command: '"C:\\Program Files\\PowerShell\\7\\pwsh.exe" -Command whoami',
          commandActions: [{ command: "whoami", type: "unknown" }],
          cwd: "D:\\codex\\codex-ui",
          durationMs: 500,
        }),
      },
    });
    // 头部：执行命令 + 简洁命令（不再是工作目录）
    expect(wrapper.find(".tool-card-title").text()).toBe("执行命令");
    expect(wrapper.find(".tool-card-sub").text()).toBe("whoami");
    expect(wrapper.text()).not.toContain("D:\\codex\\codex-ui");
    // 展开后显示工作目录
    await wrapper.find(".tool-card-header").trigger("click");
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
    expect(wrapper.find(".tool-card-sub").text()).toBe("npm run build");
  });

  it("webSearch 渲染结构化结果", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: {
          id: "w1",
          type: "webSearch",
          query: "rust tauri",
          results: [
            { title: "Tauri 官网", url: "https://tauri.app", snippet: "Rust 桌面框架" },
            { title: "第二条", url: "https://example.com" },
          ],
        } as ThreadItem,
      },
    });
    await wrapper.find(".tool-card-header").trigger("click");
    const titles = wrapper.findAll(".web-result-title").map((t) => t.text());
    expect(titles).toContain("Tauri 官网");
    expect(titles).toContain("第二条");
    expect(wrapper.find(".web-result-snippet").text()).toContain("Rust 桌面框架");
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
    // 进行中且已有输出：卡片展开但输出折叠为摘要
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
    await wrapper.find(".tool-card-header").trigger("click");
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
    await wrapper.find(".tool-card-header").trigger("click");
    const btn = wrapper.find(".tool-command .copy-btn");
    expect(btn.exists()).toBe(true);
    await btn.trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("echo hi");
    expect(btn.text()).toBe("已复制");
  });
});

describe("文件变更：打开独立 diff 窗口", () => {
  const REPLACE_DIFF = [
    "@@ -1,3 +1,3 @@",
    " a",
    "-b",
    "+X",
    " c",
  ].join("\n");

  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(null);
    store.server.workspace = "D:/repo";
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

  it("点击变更行调用 open_diff_window 并传参", async () => {
    const wrapper = mount(ToolCard, {
      props: { item: changeItem(REPLACE_DIFF, "D:\\repo\\a.cs", "update") },
    });
    await wrapper.find(".change-row").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("open_diff_window", {
      params: {
        path: "D:\\repo\\a.cs",
        kind: "update",
        diff: REPLACE_DIFF,
        workspace_root: "D:/repo",
      },
    });
  });

  it("无 diff 的行点击不打开窗口", async () => {
    const wrapper = mount(ToolCard, {
      props: {
        item: {
          id: "f2",
          type: "fileChange",
          changes: [{ path: "D:\\repo\\empty.cs", kind: "update", diff: "" }],
          status: "completed",
        } as ThreadItem,
      },
    });
    await wrapper.find(".change-row").trigger("click");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "open_diff_window",
      expect.anything(),
    );
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
    await wrapper.find(".tool-card-header").trigger("click");
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
