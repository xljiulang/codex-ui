import { describe, expect, it } from "vitest";
import {
  CODEXUI_DYNAMIC_NAMESPACE,
  CODEXUI_DYNAMIC_TOOLS,
  CODEXUI_TOOL_ADD_SCHEDULED_TASK,
  CODEXUI_TOOL_COMPACT_CONTEXT,
  CODEXUI_TOOL_GET_USAGE,
  buildInjectedDynamicTools,
  dynamicToolDisplay,
  dynamicToolKey,
  dynamicToolRows,
  isDynamicToolEnabled,
} from "../dynamicTools";

/** 协议里的保留命名空间（docs/app-server.md 6.5），codexui 不得与之冲突 */
const RESERVED_NAMESPACES = new Set([
  "functions",
  "multi_tool_use",
  "file_search",
  "web",
  "browser",
  "image_gen",
  "computer",
  "container",
  "terminal",
  "python",
  "python_user_visible",
  "api_tool",
  "tool_search",
  "submodel_delegator",
]);

describe("codexui 动态工具定义", () => {
  it("命名空间不与保留列表冲突", () => {
    expect(RESERVED_NAMESPACES.has(CODEXUI_DYNAMIC_NAMESPACE)).toBe(false);
  });

  it("包含三条 function 工具（知识库已改为 skill + CLI，不再注入动态工具）且字段合法", () => {
    const names = CODEXUI_DYNAMIC_TOOLS.flatMap((ns) =>
      ns.tools.map((t) => t.name),
    );
    expect(names).toEqual([
      CODEXUI_TOOL_GET_USAGE,
      CODEXUI_TOOL_COMPACT_CONTEXT,
      CODEXUI_TOOL_ADD_SCHEDULED_TASK,
    ]);
    expect(names).not.toContain("search_docs");
    expect(names).not.toContain("index_docs");
    for (const ns of CODEXUI_DYNAMIC_TOOLS) {
      expect(ns.type).toBe("namespace");
      expect(ns.name).toBe(CODEXUI_DYNAMIC_NAMESPACE);
      expect(ns.description.length).toBeGreaterThan(0);
      for (const t of ns.tools) {
        expect(t.type).toBe("function");
        expect(t.name).toMatch(/^[a-zA-Z0-9_-]{1,128}$/);
        expect(t.description.length).toBeGreaterThan(0);
        expect(t.inputSchema.type).toBe("object");
      }
    }
    // 无参工具保持空 properties；add_scheduled_task 带必填参数 schema
    const tool = (name: string) =>
      CODEXUI_DYNAMIC_TOOLS.flatMap((ns) => ns.tools).find((t) => t.name === name)!;
    expect(tool(CODEXUI_TOOL_GET_USAGE).inputSchema).toEqual({
      type: "object",
      properties: {},
    });
    const add = tool(CODEXUI_TOOL_ADD_SCHEDULED_TASK).inputSchema as {
      required: string[];
    };
    expect(add.required).toEqual(["name", "prompt", "cron"]);
  });

  it("缺省开关：三条工具全部缺省开启", () => {
    const byName = new Map(
      CODEXUI_DYNAMIC_TOOLS.flatMap((ns) => ns.tools).map((t) => [
        t.name,
        t.defaultEnabled,
      ]),
    );
    expect(byName.get(CODEXUI_TOOL_GET_USAGE)).toBe(true);
    expect(byName.get(CODEXUI_TOOL_COMPACT_CONTEXT)).toBe(true);
    expect(byName.get(CODEXUI_TOOL_ADD_SCHEDULED_TASK)).toBe(true);
  });
});

describe("codexui 动态工具注入过滤", () => {
  it("dynamicToolKey/display 生成 namespace.tool 键与 namespace_tool 展示名", () => {
    expect(dynamicToolKey("codexui", "get_usage")).toBe("codexui.get_usage");
    expect(dynamicToolDisplay("codexui", "get_usage")).toBe(
      "codexui_get_usage",
    );
  });

  it("dynamicToolRows 展开全部工具行并携带 key/display/描述", () => {
    const rows = dynamicToolRows();
    const keys = rows.map((r) => r.key);
    expect(keys).toContain("codexui.get_usage");
    expect(keys).toContain("codexui.compact_context");
    expect(rows.find((r) => r.tool === "get_usage")?.display).toBe(
      "codexui_get_usage",
    );
  });

  it("未配置任何开关时只注入缺省开启的三条工具", () => {
    const result = buildInjectedDynamicTools({});
    const names = result.flatMap((ns) => ns.tools.map((t) => t.name));
    expect(names).toEqual([
      CODEXUI_TOOL_GET_USAGE,
      CODEXUI_TOOL_COMPACT_CONTEXT,
      CODEXUI_TOOL_ADD_SCHEDULED_TASK,
    ]);
  });

  it("显式 false 关闭缺省开启的工具（仅保留其余）", () => {
    const result = buildInjectedDynamicTools({ "codexui.get_usage": false });
    const names = result.flatMap((ns) => ns.tools.map((t) => t.name));
    expect(names).toEqual([
      CODEXUI_TOOL_COMPACT_CONTEXT,
      CODEXUI_TOOL_ADD_SCHEDULED_TASK,
    ]);
  });

  it("历史知识库开关不影响注入（键已失效），注入前剔除私有字段", () => {
    const result = buildInjectedDynamicTools({
      "codexui.search_docs": true,
      "codexui.index_docs": true,
    });
    const names = result.flatMap((ns) => ns.tools.map((t) => t.name));
    expect(names).toEqual([
      CODEXUI_TOOL_GET_USAGE,
      CODEXUI_TOOL_COMPACT_CONTEXT,
      CODEXUI_TOOL_ADD_SCHEDULED_TASK,
    ]);
    // 注入前剔除 codex-ui 私有字段，避免协议收到未知键
    const usage = result
      .flatMap((ns) => ns.tools)
      .find((t) => t.name === CODEXUI_TOOL_GET_USAGE)!;
    expect(Object.keys(usage).sort()).toEqual([
      "description",
      "inputSchema",
      "name",
      "type",
    ]);
  });

  it("全部显式关闭返回空数组（整体不注入）", () => {
    const result = buildInjectedDynamicTools({
      "codexui.get_usage": false,
      "codexui.compact_context": false,
      "codexui.add_scheduled_task": false,
    });
    expect(result).toEqual([]);
  });

  it("isDynamicToolEnabled：显式配置优先，缺省回落工具定义", () => {
    expect(isDynamicToolEnabled("codexui", "get_usage", {})).toBe(true);
    expect(
      isDynamicToolEnabled("codexui", "get_usage", { "codexui.get_usage": false }),
    ).toBe(false);
    // 历史知识库工具已从定义中移除：显式配置仍按"显式优先"读取，
    // 但注入列表由定义决定，因此不会再出现在注入结果里（见上一条用例）
    expect(isDynamicToolEnabled("codexui", "search_docs", {})).toBe(false);
    expect(
      isDynamicToolEnabled("codexui", "search_docs", {
        "codexui.search_docs": true,
      }),
    ).toBe(true);
    // 未知工具（未在定义里）缺省视为关闭，不注入
    expect(isDynamicToolEnabled("codexui", "unknown", {})).toBe(false);
    expect(
      isDynamicToolEnabled("codexui", "unknown", { "codexui.unknown": true }),
    ).toBe(true);
    expect(isDynamicToolEnabled("codexui", "get_usage", undefined)).toBe(true);
  });
});
