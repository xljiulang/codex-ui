import { describe, expect, it } from "vitest";
import {
  CODEXUI_DYNAMIC_NAMESPACE,
  CODEXUI_DYNAMIC_TOOLS,
  CODEXUI_TOOL_ADD_SCHEDULED_TASK,
  CODEXUI_TOOL_COMPACT_CONTEXT,
  CODEXUI_TOOL_GET_USAGE,
  CODEXUI_TOOL_INDEX_DOCS,
  CODEXUI_TOOL_SEARCH_DOCS,
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

  it("包含五条 function 工具（含两条默认关闭的知识库工具）且字段合法", () => {
    const names = CODEXUI_DYNAMIC_TOOLS.flatMap((ns) =>
      ns.tools.map((t) => t.name),
    );
    expect(names).toContain(CODEXUI_TOOL_GET_USAGE);
    expect(names).toContain(CODEXUI_TOOL_COMPACT_CONTEXT);
    expect(names).toContain(CODEXUI_TOOL_ADD_SCHEDULED_TASK);
    expect(names).toContain(CODEXUI_TOOL_SEARCH_DOCS);
    expect(names).toContain(CODEXUI_TOOL_INDEX_DOCS);
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
    // search_docs 只要求 query，topK 可选
    const search = tool(CODEXUI_TOOL_SEARCH_DOCS).inputSchema as {
      required: string[];
      properties: Record<string, unknown>;
    };
    expect(search.required).toEqual(["query"]);
    expect(Object.keys(search.properties)).toEqual(["query", "topK"]);
    // index_docs 无必填参数（缺省=当前工作目录、增量）
    const index = tool(CODEXUI_TOOL_INDEX_DOCS).inputSchema as {
      required?: string[];
    };
    expect(index.required).toBeUndefined();
  });

  it("缺省开关：既有三条 true、知识库两条 false", () => {
    const byName = new Map(
      CODEXUI_DYNAMIC_TOOLS.flatMap((ns) => ns.tools).map((t) => [
        t.name,
        t.defaultEnabled,
      ]),
    );
    expect(byName.get(CODEXUI_TOOL_GET_USAGE)).toBe(true);
    expect(byName.get(CODEXUI_TOOL_COMPACT_CONTEXT)).toBe(true);
    expect(byName.get(CODEXUI_TOOL_ADD_SCHEDULED_TASK)).toBe(true);
    expect(byName.get(CODEXUI_TOOL_SEARCH_DOCS)).toBe(false);
    expect(byName.get(CODEXUI_TOOL_INDEX_DOCS)).toBe(false);
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

  it("显式 true 开启缺省关闭的知识库工具", () => {
    const result = buildInjectedDynamicTools({ "codexui.search_docs": true });
    const names = result.flatMap((ns) => ns.tools.map((t) => t.name));
    expect(names).toContain(CODEXUI_TOOL_SEARCH_DOCS);
    expect(names).not.toContain(CODEXUI_TOOL_INDEX_DOCS);
    // 注入前剔除 codex-ui 私有字段，避免协议收到未知键
    const search = result
      .flatMap((ns) => ns.tools)
      .find((t) => t.name === CODEXUI_TOOL_SEARCH_DOCS)!;
    expect(Object.keys(search).sort()).toEqual([
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
