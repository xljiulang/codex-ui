export interface ServerStatus {
  connected: boolean;
  startupWorkspace: string;
  codexPath?: string | null;
  logs: string[];
}

export interface ThreadSummary {
  id: string;
  name?: string | null;
  preview?: string | null;
  createdAt: number;
  updatedAt?: number;
  recencyAt?: number | null;
  cwd?: string;
  source?: string;
  cliVersion?: string;
  status?: { type: string };
  /** 服务端持久化的分区（新版协议用内置 “Pinned” 分区表示置顶） */
  section?: { id: string; name: string } | null;
  isPinned?: boolean;
}

export interface Turn {
  id: string;
  items: ThreadItem[];
  status: string;
  error?: { message: string } | null;
  startedAt?: number | null;
  completedAt?: number | null;
  durationMs?: number | null;
}

export interface ThreadRead {
  thread: {
    id: string;
    name?: string | null;
    preview?: string;
    turns?: Turn[];
  };
}

/** 消息/工具项公共字段（协议推送的动态字段，各类型按需具备） */
export interface ThreadItemBase {
  id: string;
  streaming?: boolean;
  startedAtMs?: number;
  completedAtMs?: number;
  durationMs?: number | null;
  clientId?: string;
  /** 协议未枚举字段保持松散访问（迁移期兼容，按需 as 收窄） */
  [key: string]: unknown;
}

/**
 * 消息/工具项：type 为判别字段，按类型收窄后字段有真实类型；
 * 未枚举的类型落入 UnknownItem（松散访问）。
 */
export type ThreadItem =
  | UserMessageItem
  | AgentMessageItem
  | PlanItem
  | ReasoningItem
  | ErrorItem
  | CommandExecutionItem
  | CollabAgentToolCallItem
  | McpToolCallItem
  | DynamicToolCallItem
  | FileChangeItem
  | WebSearchItem
  | TodoListItem
  | ContextCompactionItem
  | ImageViewItem
  | SleepItem
  | UnknownItem;

/** 协议中未枚举的消息/工具类型：保持松散访问 */
export interface UnknownItem extends ThreadItemBase {
  type: string;
}

export interface TextInput {
  type: "text";
  text: string;
  text_elements: unknown[];
}
export interface MentionInput {
  type: "mention";
  name: string;
  path: string;
}
export interface SkillInput {
  type: "skill";
  name: string;
  path: string;
  /** 客户端标记：来自 @ 菜单的插件（source="plugin"）或 $ 菜单的技能（source="skill"）；协议发送时不含该字段 */
  source?: "plugin" | "skill";
  /** 客户端标记：插件在 plugin/list 中的 pluginId（如 documents@openai-primary-runtime），用于生成 plugin:// URI；协议发送时不含该字段 */
  pluginId?: string;
}
export interface LocalImageInput {
  type: "localImage";
  path: string;
}
export type UserInput = TextInput | MentionInput | SkillInput | LocalImageInput;

/** 运行时校验：内容项是否为合法的 UserInput（协议外未知形状直接丢弃） */
export function isUserInput(c: unknown): c is UserInput {
  if (!c || typeof c !== "object") return false;
  const t = (c as { type?: unknown }).type;
  if (t === "text") return typeof (c as TextInput).text === "string";
  if (t === "localImage") return typeof (c as LocalImageInput).path === "string";
  if (t === "mention" || t === "skill") {
    const m = c as MentionInput | SkillInput;
    return typeof m.name === "string" && typeof m.path === "string";
  }
  return false;
}

export interface PendingInteraction {
  /** 协议 RequestId 为 string | number，原样透传给 interaction_respond */
  requestId: number | string;
  method: string;
  params: Record<string, unknown>;
  at: number;
}

/** 界面主题：blue（蓝夜，默认）｜dark（曜黑）｜light（晨光） */
export type ThemeId = "blue" | "dark" | "light";

/** 权限批准模式：ask-for-approval（请求批准）｜help-me-approve（帮我批准）｜full-access（完全访问） */
export type PermissionId =
  | "ask-for-approval"
  | "help-me-approve"
  | "full-access";

/** 记忆模式：disabled（关闭，默认）｜enabled（启用） */
export type MemoryMode = "disabled" | "enabled";

/** 跟进处理方式：adjust（调整方向）｜queue（加入队列） */
export type FollowupMode = "adjust" | "queue";

export interface AppSettings {
  codex_path?: string | null;
  sound_enabled: boolean;
  enter_to_send: boolean;
  followup_mode: FollowupMode;
  theme: ThemeId;
  /** 权限模式的启动初始值 */
  default_permission: PermissionId;
  /** 作为新建会话的初始值 */
  memory_mode: MemoryMode;
}

export interface AuthStatus {
  authMethod?: string | null;
  authToken?: string | null;
  requiresOpenaiAuth?: boolean | null;
}

export interface AgentMessageItem extends ThreadItemBase {
  type: "agentMessage";
  text: string;
  phase?: string | null;
}

/** 计划模式确认消息（与 agentMessage 同形状） */
export interface PlanItem extends ThreadItemBase {
  type: "plan";
  text: string;
  phase?: string | null;
}

export interface ReasoningItem extends ThreadItemBase {
  type: "reasoning";
  summary?: string[];
  content?: string[];
}

/** 服务端错误消息（未知消息类型兜底渲染） */
export interface ErrorItem extends ThreadItemBase {
  type: "error";
  message?: string | null;
}

export interface CommandExecutionItem extends ThreadItemBase {
  type: "commandExecution";
  command: string;
  cwd?: string;
  status: string;
  aggregatedOutput?: string | null;
  exitCode?: number | null;
}

/** 子代理协作调用（字段形状同动态工具，宽松访问） */
export interface CollabAgentToolCallItem extends ThreadItemBase {
  type: "collabAgentToolCall";
  status?: string;
  result?: unknown;
  error?: { message?: string } | null;
}

export interface McpToolCallItem extends ThreadItemBase {
  type: "mcpToolCall";
  server: string;
  tool: string;
  arguments?: unknown;
  status: string;
  result?: { content?: unknown[] } | null;
  error?: { message?: string } | null;
}

export interface DynamicToolCallItem extends ThreadItemBase {
  type: "dynamicToolCall";
  tool: string;
  namespace?: string | null;
  arguments?: unknown;
  status: string;
  contentItems?: unknown[] | null;
  success?: boolean | null;
}

export interface FileChangeItem extends ThreadItemBase {
  type: "fileChange";
  changes: { path: string; kind: string | { type: string }; diff?: string }[];
  status: string;
}

export interface WebSearchItem extends ThreadItemBase {
  type: "webSearch";
  query: string;
}

export interface TodoListItem extends ThreadItemBase {
  type: "todoList";
  items: { text: string; completed: boolean }[];
}

/** 上下文已压缩提示（无附加字段） */
export interface ContextCompactionItem extends ThreadItemBase {
  type: "contextCompaction";
}

/** 图片查看消息（独立图片渲染分支） */
export interface ImageViewItem extends ThreadItemBase {
  type: "imageView";
  path?: string;
}

/** 等待提示（如命令执行中的 sleep） */
export interface SleepItem extends ThreadItemBase {
  type: "sleep";
}

/** diff 预览内联行（Rust build_diff_preview 返回，字段 camelCase） */
export type DiffRow =
  | { kind: "ctx"; oldNo: number; newNo: number; text: string }
  | { kind: "del"; oldNo: number; text: string }
  | { kind: "add"; newNo: number; text: string }
  | { kind: "sep" };

export interface UserMessageItem extends ThreadItemBase {
  type: "userMessage";
  content: UserInput[];
}
