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
  /** 线程历史存储模式：legacy（codex-ui 创建）| paginated（codex CLI 创建） */
  historyMode?: string | null;
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
    cwd?: string | null;
    /** legacy | paginated；分页线程不支持 includeTurns=true，须用 thread/turns/list 分页读取 */
    historyMode?: string | null;
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
  | ImageGenerationItem
  | SubAgentActivityItem
  | SleepItem
  | UnknownItem;

/** 协议中未枚举的消息/工具类型：保持松散访问 */
export interface UnknownItem extends ThreadItemBase {
  type: string;
}

/**
 * 判别辅助：按 type 收窄到指定消息类型。
 * 相比裸 `item.type !== "x"`，类型谓词能排除 UnknownItem 兜底成员，
 * 保证收窄结果不含松散访问成员。
 */
export function isThreadItemType<T extends ThreadItem>(
  item: ThreadItem,
  type: T["type"],
): item is T {
  return item.type === type;
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

/** 终端 Shell：cmd（命令提示符，默认）｜powershell */
export type TerminalShell = "cmd" | "powershell";

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
  /** 新开终端使用的 Shell */
  terminal_shell: TerminalShell;
}

/** 模型配置（config.toml / model_catalog_json 目标文件）读取结果，字段与 Rust 端一致 */
export interface ModelConfigState {
  config_path: string;
  config_exists: boolean;
  config_content: string;
  /** 顶层 model_catalog_json 的原始配置值（未配置时为空字符串） */
  model_catalog_json: string;
  model_catalog_path: string;
  model_catalog_exists: boolean;
  model_catalog: string;
  model: string;
  model_reasoning_effort: string;
  model_provider: string;
  preferred_auth_method: string;
  forced_login_method: string;
  /** 进程环境是否已设置非空 OPENAI_API_KEY */
  openai_api_key_present: boolean;
  providers: ModelProviderInfo[];
}

/** 单个 model_provider 的可视化字段（标识 key 创建后不可改名） */
export interface ModelProviderInfo {
  key: string;
  name: string;
  base_url: string;
  env_key: string;
  experimental_bearer_token: string;
  wire_api: string;
}

/** 可视化模型配置保存载荷（与 Rust 端 ModelConfigUiEdit 一致） */
export interface ModelConfigUiEdit {
  model: string;
  model_reasoning_effort: string;
  model_provider: string;
  preferred_auth_method: string;
  forced_login_method: string;
  model_catalog_json: string;
  providers: ModelProviderInfo[];
}

/** 自定义指令（CODEX_HOME/AGENTS.md）读取结果，字段与 Rust 端一致 */
export interface CustomInstructionsState {
  agents_path: string;
  exists: boolean;
  content: string;
}

/** env 表中的单个键值对 */
export interface McpEnvEntry {
  key: string;
  value: string;
}

/** 单个 MCP 服务器条目（env 用有序键值对，便于 UI 增删） */
export interface McpServerInfo {
  /** [mcp_servers.<name>] 表名标识 */
  name: string;
  /** STDIO 启动命令（http 服务器为空） */
  command: string;
  args: string[];
  env: McpEnvEntry[];
  /** Streamable HTTP 地址（stdio 服务器为空） */
  url: string;
  /** 静态 HTTP 请求头（http_headers） */
  headers: McpEnvEntry[];
  /** Bearer 令牌来源环境变量名 */
  bearer_token_env_var: string;
}

/** mcp_servers_read 返回结构（与 Rust 端一致） */
export interface McpServersState {
  config_path: string;
  servers: McpServerInfo[];
}

/** mcp_servers_save 输入（与 Rust 端一致） */
export interface McpServersEdit {
  servers: McpServerInfo[];
}

/** 单个本地技能条目（skills_read 返回） */
export interface SkillsItem {
  name: string;
  /** SKILL.md 的绝对路径 */
  path: string;
  description: string;
  enabled: boolean;
}

/** skills_read 返回结构（与 Rust 端一致） */
export interface SkillsState {
  skills_dir: string;
  items: SkillsItem[];
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
  /** item/mcpToolCall/progress 通知写入的进度文本/百分比 */
  progressText?: string;
  progressPercent?: number;
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

/** AI 生成图片条目（协议 imageGeneration：id, status, revisedPrompt?, result） */
export interface ImageGenerationItem extends ThreadItemBase {
  type: "imageGeneration";
  status: string;
  revisedPrompt?: string | null;
  /** 结果形状宽松：本地路径 / http(s) / data: URL，或含 path/url/src/dataUrl 的对象 */
  result?: unknown;
}

/** 子代理活动条目（协议 subAgentActivity：kind, agentThreadId, agentPath） */
export interface SubAgentActivityItem extends ThreadItemBase {
  type: "subAgentActivity";
  kind: string;
  agentThreadId?: string | null;
  agentPath?: string | null;
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
