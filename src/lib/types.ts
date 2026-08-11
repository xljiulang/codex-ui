export interface ServerStatus {
  connected: boolean;
  workspace: string;
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

export type ThreadItem = {
  id: string;
  type: string;
  [key: string]: unknown;
};

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

export interface PendingInteraction {
  requestId: number;
  method: string;
  params: Record<string, unknown>;
  at: number;
}

export interface AppSettings {
  codex_path?: string | null;
  sound_enabled: boolean;
  enter_to_send: boolean;
  followup_mode: string;
  /** 界面主题：blue（蓝夜，默认）｜dark（曜黑）｜light（晨光） */
  theme: string;
}

export interface AuthStatus {
  authMethod?: string | null;
  authToken?: string | null;
  requiresOpenaiAuth?: boolean | null;
}

export interface AgentMessageItem {
  id: string;
  type: "agentMessage";
  text: string;
  phase?: string | null;
}

export interface ReasoningItem {
  id: string;
  type: "reasoning";
  summary?: string[];
  content?: string[];
}

export interface CommandExecutionItem {
  id: string;
  type: "commandExecution";
  command: string;
  cwd?: string;
  status: string;
  aggregatedOutput?: string | null;
  exitCode?: number | null;
  durationMs?: number | null;
}

export interface McpToolCallItem {
  id: string;
  type: "mcpToolCall";
  server: string;
  tool: string;
  arguments?: unknown;
  status: string;
  result?: { content?: unknown[] } | null;
  error?: { message?: string } | null;
  durationMs?: number | null;
}

export interface DynamicToolCallItem {
  id: string;
  type: "dynamicToolCall";
  tool: string;
  namespace?: string | null;
  arguments?: unknown;
  status: string;
  contentItems?: unknown[] | null;
  success?: boolean | null;
  durationMs?: number | null;
}

export interface FileChangeItem {
  id: string;
  type: "fileChange";
  changes: { path: string; kind: string | { type: string }; diff?: string }[];
  status: string;
}

export interface WebSearchItem {
  id: string;
  type: "webSearch";
  query: string;
}

export interface TodoListItem {
  id: string;
  type: "todoList";
  items: { text: string; completed: boolean }[];
}

/** diff 预览内联行（Rust build_diff_preview 返回，字段 camelCase） */
export type DiffRow =
  | { kind: "ctx"; oldNo: number; newNo: number; text: string }
  | { kind: "del"; oldNo: number; text: string }
  | { kind: "add"; newNo: number; text: string }
  | { kind: "sep" };

export interface UserMessageItem {
  id: string;
  type: "userMessage";
  content: UserInput[];
}
