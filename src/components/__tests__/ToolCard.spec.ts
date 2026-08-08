import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import ToolCard from "../ToolCard.vue";
import type { ThreadItem } from "../../lib/types";

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
    expect(wrapper.text()).toMatch(/进行中 · \d\d:\d\d\.\d/);

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
});
