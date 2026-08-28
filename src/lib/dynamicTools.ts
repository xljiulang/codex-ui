/** codex-ui 通过 app-server `thread/start.dynamicTools` 暴露给 agent 的动态工具定义。 */

/** 命名空间：codex-ui 管理工具（agent 侧以 `codexui_get_usage` / `codexui_compact_context` 呈现） */
export const CODEXUI_DYNAMIC_NAMESPACE = "codexui";
export const CODEXUI_TOOL_GET_USAGE = "get_usage";
export const CODEXUI_TOOL_COMPACT_CONTEXT = "compact_context";

/** 与协议 `DynamicToolFunctionSpec` 对齐的最小结构（字段名以 generate-ts 绑定为准） */
export interface DynamicToolFunctionSpec {
  type: "function";
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  deferLoading?: boolean;
}

/** 与协议 `DynamicToolNamespaceSpec` 对齐的最小结构 */
export interface DynamicToolNamespaceSpec {
  type: "namespace";
  name: string;
  description: string;
  tools: DynamicToolFunctionSpec[];
}

/** 由 codex-ui 在 `thread/start` 时注入的动态工具（单一 `codexui` 命名空间，v1 两条） */
// 注意：app-server 要求 inputSchema 是 JSON Schema 的 `type: "object"`（空对象 `{}` 会被拒，
// 报 "schema must be a JSON Schema of type object, got type: null"）；两条均无参数，故用空 properties。
export const CODEXUI_DYNAMIC_TOOLS: DynamicToolNamespaceSpec[] = [
  {
    type: "namespace",
    name: CODEXUI_DYNAMIC_NAMESPACE,
    description: "codex-ui 管理工具：查询用量、压缩上下文",
    tools: [
      {
        type: "function",
        name: CODEXUI_TOOL_GET_USAGE,
        description: "查询当前会话的 token 消耗与上下文窗口占用情况",
        inputSchema: { type: "object", properties: {} },
      },
      {
        type: "function",
        name: CODEXUI_TOOL_COMPACT_CONTEXT,
        description: "压缩当前会话的上下文",
        inputSchema: { type: "object", properties: {} },
      },
    ],
  },
];
