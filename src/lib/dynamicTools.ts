/** codex-ui 通过 app-server `thread/start.dynamicTools` 暴露给 agent 的动态工具定义。 */

/** 命名空间：codex-ui 管理工具（agent 侧以 `codexui_get_usage` / `codexui_compact_context` 呈现） */
export const CODEXUI_DYNAMIC_NAMESPACE = "codexui";
export const CODEXUI_TOOL_GET_USAGE = "get_usage";
export const CODEXUI_TOOL_COMPACT_CONTEXT = "compact_context";
export const CODEXUI_TOOL_ADD_SCHEDULED_TASK = "add_scheduled_task";
/** 知识库检索（默认关闭：需在「设置 → 动态工具」显式开启） */
export const CODEXUI_TOOL_SEARCH_DOCS = "search_docs";
/** 知识库建库/增量更新（同一工作目录的知识库，幂等） */
export const CODEXUI_TOOL_INDEX_DOCS = "index_docs";

/** 与协议 `DynamicToolFunctionSpec` 对齐的最小结构（字段名以 generate-ts 绑定为准） */
export interface DynamicToolFunctionSpec {
  type: "function";
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  deferLoading?: boolean;
}

/** 工具定义（含 codex-ui 侧的缺省开关；注入前会剔除 `defaultEnabled`） */
export interface DynamicToolDef {
  type: "function";
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  /** 未在 `dynamic_tools_state` 中显式配置时的缺省：true 注入、false 不注入 */
  defaultEnabled: boolean;
}

/** 与协议 `DynamicToolNamespaceSpec` 对齐的最小结构 */
export interface DynamicToolNamespaceSpec {
  type: "namespace";
  name: string;
  description: string;
  tools: DynamicToolFunctionSpec[];
}

interface DynamicToolNamespaceDef {
  type: "namespace";
  name: string;
  description: string;
  tools: DynamicToolDef[];
}

/** 由 codex-ui 在 `thread/start` 时注入的动态工具（单一 `codexui` 命名空间） */
// 注意：app-server 要求 inputSchema 是 JSON Schema 的 `type: "object"`（空对象 `{}` 会被拒，
// 报 "schema must be a JSON Schema of type object, got type: null"）；无参工具用空 properties。
export const CODEXUI_DYNAMIC_TOOLS: DynamicToolNamespaceDef[] = [
  {
    type: "namespace",
    name: CODEXUI_DYNAMIC_NAMESPACE,
    description: "codex-ui 管理工具：查询用量、压缩上下文、创建定时任务、检索与更新售后知识库",
    tools: [
      {
        type: "function",
        name: CODEXUI_TOOL_GET_USAGE,
        description: "查询当前会话的 token 消耗与上下文窗口占用情况",
        inputSchema: { type: "object", properties: {} },
        defaultEnabled: true,
      },
      {
        type: "function",
        name: CODEXUI_TOOL_COMPACT_CONTEXT,
        description: "压缩当前会话的上下文",
        inputSchema: { type: "object", properties: {} },
        defaultEnabled: true,
      },
      {
        type: "function",
        name: CODEXUI_TOOL_ADD_SCHEDULED_TASK,
        description:
          "创建定时任务：到点后在当前会话自动发送 prompt 开启新回合。" +
          "cron 为 6 字段「秒 分 时 日 月 周」，本地时区，最小间隔 1 分钟，每天 9 点=\"0 0 9 * * *\"，" +
          "单次任务 7 字段末尾年份（如 2026-01-20=\"0 0 9 20 1 * 2026\"）。" +
          "prompt 须自包含。busyPolicy：会话忙时 skip 跳过（默认）/defer 顺延。创建前需在对话中与用户确认。",
        inputSchema: {
          type: "object",
          properties: {
            name: { type: "string", description: "任务名" },
            prompt: { type: "string", description: "到点执行的完整指令（自包含）" },
            cron: { type: "string", description: "6 字段 cron（最小间隔 1 分钟；单次 7 字段末尾年份）" },
            busyPolicy: {
              type: "string",
              enum: ["defer", "skip"],
              description: "会话忙时：skip 跳过（默认）| defer 顺延",
            },
          },
          required: ["name", "prompt", "cron"],
        },
        defaultEnabled: true,
      },
      {
        type: "function",
        name: CODEXUI_TOOL_SEARCH_DOCS,
        description:
          "检索当前会话工作目录对应知识库里的售后文档手册（向量 + 关键词混合召回）。" +
          "回答故障码、型号、操作步骤这类需要事实依据的问题前应先调用本工具，并在回答中标注来源文件名与章节。" +
          "若返回「尚未建库」，可先用 index_docs 建立/更新知识库。",
        inputSchema: {
          type: "object",
          properties: {
            query: {
              type: "string",
              description: "检索词：用用户问题里的关键实体（故障现象、错误码、型号、部件名）",
            },
            topK: {
              type: "integer",
              description: "返回条数（1-20，缺省 8）",
            },
          },
          required: ["query"],
        },
        defaultEnabled: false,
      },
      {
        type: "function",
        name: CODEXUI_TOOL_INDEX_DOCS,
        description:
          "为当前会话工作目录建立/增量更新知识库：扫描给定路径下的 PDF、Word(docx)、Markdown、txt，" +
          "抽取正文、切块、本地向量化后入库（知识库与工作目录一对一，同目录的其它会话共享）。" +
          "幂等：内容没变化时秒回当前统计，因此也可用来查询知识库现状（文档数/切块数/来源）。",
        inputSchema: {
          type: "object",
          properties: {
            paths: {
              type: "array",
              items: { type: "string" },
              description:
                "要索引的目录或文件（绝对路径，或相对当前工作目录）；缺省=当前工作目录自身（递归）",
            },
            full: {
              type: "boolean",
              description: "true=忽略增量判定全量重建（缺省 false）",
            },
          },
        },
        defaultEnabled: false,
      },
    ],
  },
];

/** 动态工具唯一键：`namespace.tool`（如 codexui.get_usage），用于设置持久化与注入过滤 */
export function dynamicToolKey(namespace: string, tool: string): string {
  return `${namespace}.${tool}`;
}

/** agent 侧展示名：`namespace_tool`（如 codexui_get_usage） */
export function dynamicToolDisplay(namespace: string, tool: string): string {
  return `${namespace}_${tool}`;
}

/** 设置页工具行（由静态定义展开，供开关 UI 渲染） */
export interface DynamicToolRow {
  key: string;
  display: string;
  description: string;
  namespace: string;
  tool: string;
  /** 未显式配置时的缺省开关（设置页开关显示的是「生效值」） */
  defaultEnabled: boolean;
}

export function dynamicToolRows(): DynamicToolRow[] {
  return CODEXUI_DYNAMIC_TOOLS.flatMap((ns) =>
    ns.tools.map((t) => ({
      key: dynamicToolKey(ns.name, t.name),
      display: dynamicToolDisplay(ns.name, t.name),
      description: t.description,
      namespace: ns.name,
      tool: t.name,
      defaultEnabled: t.defaultEnabled,
    })),
  );
}

/**
 * 生效开关：`dynamic_tools_state[key]` 显式配置优先，缺省用工具定义里的 `defaultEnabled`。
 */
export function isDynamicToolEnabled(
  namespace: string,
  tool: string,
  state: Record<string, boolean> | undefined,
): boolean {
  const key = dynamicToolKey(namespace, tool);
  const explicit = state?.[key];
  if (typeof explicit === "boolean") return explicit;
  for (const ns of CODEXUI_DYNAMIC_TOOLS) {
    if (ns.name !== namespace) continue;
    for (const t of ns.tools) {
      if (t.name === tool) return t.defaultEnabled;
    }
  }
  return false;
}

/**
 * 过滤出实际注入的动态工具：只保留生效开关为 true 的工具，
 * 命名空间内无工具则整组不注入；注入前剔除 codex-ui 私有的 `defaultEnabled`。
 */
export function buildInjectedDynamicTools(
  state: Record<string, boolean> | undefined,
): DynamicToolNamespaceSpec[] {
  return CODEXUI_DYNAMIC_TOOLS.flatMap((ns) => {
    const tools: DynamicToolFunctionSpec[] = ns.tools
      .filter((t) => isDynamicToolEnabled(ns.name, t.name, state))
      .map((t) => ({
        type: t.type,
        name: t.name,
        description: t.description,
        inputSchema: t.inputSchema,
      }));
    return tools.length
      ? [{ type: ns.type, name: ns.name, description: ns.description, tools }]
      : [];
  });
}
