import { describe, expect, it } from "vitest";
import {
  CODEXUI_DYNAMIC_NAMESPACE,
  CODEXUI_DYNAMIC_TOOLS,
  CODEXUI_TOOL_ADD_SCHEDULED_TASK,
  CODEXUI_TOOL_COMPACT_CONTEXT,
  CODEXUI_TOOL_GET_USAGE,
  CODEXUI_TOOL_SEND_FILE_TO_WECHAT,
  buildInjectedDynamicTools,
  dynamicToolDisplay,
  dynamicToolKey,
  dynamicToolRows,
  isDynamicToolDisabled,
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

  it("包含 get_usage / compact_context / add_scheduled_task / send_file_to_wechat 四条 function 工具且字段合法", () => {
    const names = CODEXUI_DYNAMIC_TOOLS.flatMap((ns) =>
      ns.tools.map((t) => t.name),
    );
    expect(names).toContain(CODEXUI_TOOL_GET_USAGE);
    expect(names).toContain(CODEXUI_TOOL_COMPACT_CONTEXT);
    expect(names).toContain(CODEXUI_TOOL_ADD_SCHEDULED_TASK);
    expect(names).toContain(CODEXUI_TOOL_SEND_FILE_TO_WECHAT);
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
      CODEXUI_DYNAMIC_TOOLS.flatMap((ns) => ns.tools).find(
        (t) => t.name === name,
      )!;
    expect(tool(CODEXUI_TOOL_GET_USAGE).inputSchema).toEqual({
      type: "object",
      properties: {},
    });
    const add = tool(CODEXUI_TOOL_ADD_SCHEDULED_TASK).inputSchema as {
      required: string[];
    };
    expect(add.required).toEqual(["name", "prompt", "cron"]);
    const send = tool(CODEXUI_TOOL_SEND_FILE_TO_WECHAT).inputSchema as {
      required: string[];
      properties: { paths: { type: string; items: { type: string } } };
    };
    expect(send.required).toEqual(["paths"]);
    expect(send.properties.paths.type).toBe("array");
    expect(send.properties.paths.items.type).toBe("string");
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
    expect(keys).toContain("codexui.send_file_to_wechat");
    expect(rows.find((r) => r.tool === "get_usage")?.display).toBe(
      "codexui_get_usage",
    );
  });

  it("空 disabled 原样返回全部工具", () => {
    const result = buildInjectedDynamicTools([]);
    const names = result.flatMap((ns) => ns.tools.map((t) => t.name));
    expect(names).toContain(CODEXUI_TOOL_GET_USAGE);
    expect(names).toContain(CODEXUI_TOOL_COMPACT_CONTEXT);
  });

  it("禁用其一仅保留其余工具", () => {
    const result = buildInjectedDynamicTools(["codexui.get_usage"]);
    const names = result.flatMap((ns) => ns.tools.map((t) => t.name));
    expect(names).toEqual([
      CODEXUI_TOOL_COMPACT_CONTEXT,
      CODEXUI_TOOL_ADD_SCHEDULED_TASK,
      CODEXUI_TOOL_SEND_FILE_TO_WECHAT,
    ]);
  });

  it("全部禁用返回空数组（整体不注入）", () => {
    const result = buildInjectedDynamicTools([
      "codexui.get_usage",
      "codexui.compact_context",
      "codexui.add_scheduled_task",
      "codexui.send_file_to_wechat",
    ]);
    expect(result).toEqual([]);
  });

  it("isDynamicToolDisabled 按 namespace.tool 键命中", () => {
    expect(
      isDynamicToolDisabled("codexui", "get_usage", ["codexui.get_usage"]),
    ).toBe(true);
    expect(
      isDynamicToolDisabled("codexui", "compact_context", [
        "codexui.get_usage",
      ]),
    ).toBe(false);
  });
});
