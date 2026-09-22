// useCodex 拆分模块：类型与常量（原 useCodex.ts 的一部分，纯移动，行为不变）
import type { AppSettings, PendingInteraction, PermissionId, UserInput } from "../../lib/types";
import { TabIcon, TabKind, type EditorTabBase } from "../../lib/tabs";


export const defaultSettings = (): AppSettings => ({
  codex_path: null,
  enter_to_send: true,
  followup_mode: "adjust",
  theme: "blue",
  default_permission: "ask-for-approval",
  terminal_shell: "cmd",
    dynamic_tools_disabled: [],
    glass_effect: true,
    last_session_id: null,
    compat_proxy_enabled: false,
    compat_proxy_port: 18080,
    compat_proxy_base_url: "https://opencode.ai/zen/v1",
    compat_proxy_nudge_enabled: true,
    compat_proxy_identity_enabled: true,
    error_notify_enabled: true,
    interaction_notify_enabled: true,
  });


export interface ModelInfo {
  id: string;
  model: string;
  displayName: string;
  description: string;
  hidden: boolean;
  isDefault: boolean;
  supportedReasoningEfforts: { reasoningEffort: string; description: string }[];
  defaultReasoningEffort: string;
}


/** 线程目标的持久状态（thread/goal/get 与 thread/goal/updated 携带；值对齐协议绑定 ThreadGoalStatus） */
export type GoalStatus =
  | "active"
  | "paused"
  | "blocked"
  | "usageLimited"
  | "budgetLimited"
  | "complete";


export function isGoalStatus(v: unknown): v is GoalStatus {
  return (
    v === "active" ||
    v === "paused" ||
    v === "blocked" ||
    v === "usageLimited" ||
    v === "budgetLimited" ||
    v === "complete"
  );
}


/** 服务端终态：目标已完成/预算耗尽/用量受限/阻塞/暂停，客户端收到后自动清目标并复位 flag */
const GOAL_TERMINAL_STATUSES = new Set<GoalStatus>([
  "complete",
  "budgetLimited",
  "usageLimited",
  "blocked",
  "paused",
]);


export function isGoalTerminalStatus(v: unknown): v is GoalStatus {
  return typeof v === "string" && GOAL_TERMINAL_STATUSES.has(v as GoalStatus);
}


/** 终态 toast 文案：目标已完成/预算耗尽/用量受限/已阻塞/已暂停 */
export function goalStatusToast(status: GoalStatus): string {
  switch (status) {
    case "complete":
      return "目标已完成";
    case "budgetLimited":
      return "目标预算耗尽";
    case "usageLimited":
      return "目标用量受限";
    case "blocked":
      return "目标已阻塞";
    case "paused":
      return "目标已暂停";
    default:
      return "";
  }
}


/** 计划模式回合完成后的“计划已就绪”确认弹窗数据（纯前端 UX，非协议交互） */
export interface PlanPrompt {
  threadId: string;
  turnId: string;
  planText: string;
}

/** Updated Plan 任务清单步骤状态（协议 turn/plan/updated：pending/inProgress/completed） */
export type PlanStepStatus = "pending" | "inProgress" | "completed";

export interface PlanStep {
  step: string;
  status: PlanStepStatus;
}

/** turn/plan/updated 通知载荷（explanation? + plan: [{step,status}]） */
export interface TurnPlan {
  explanation?: string;
  steps: PlanStep[];
}


/**
 * 会话标签：左侧标签区的每个“会话”标签对应一个打开的会话
 * （threadId 非空 = 已绑定线程；null = 待发送首条消息的新对话）。
 * 会话实时状态全部落在标签对象上（标签即唯一事实源，活动标签直接读写本记录）；
 * 消息列表本身按线程存于 itemsByThread，无需复制。
 */
export interface SessionTab extends EditorTabBase {
  kind: (typeof TabKind)["Session"];
  title: string;
  icon: (typeof TabIcon)["Session"];
  /** 标签唯一 id（线程绑定前后保持稳定，供编辑器标签 key 使用） */
  id: string;
  /** 绑定的线程 id；一个会话最多对应一个标签（唯一性约束） */
  threadId: string | null;
  /** 会话名称（自动生成/手动重命名，随 thread/name/updated 同步） */
  name: string;
  /** 会话名是否来自「首条消息内容」（供 AI 总结覆盖；手动重命名后置 false） */
  nameIsFirstMessage: boolean;
  /** 会话私有：权限模式（新会话取默认权限） */
  permissionMode: PermissionId;
  /** 会话私有：协作模式 */
  collaborationMode: "default" | "plan";
  /** 会话私有：模型（null = 服务端默认） */
  model: string | null;
  /** 会话私有：推理强度（null = 默认） */
  effort: string | null;
  /** 会话私有：插件列表缓存（plugin/list 归一化，随会话创建/打开拉取） */
  plugins: { plugins: PluginItem[]; loaded: boolean };
  /** 会话私有：技能列表缓存（skills/list 归一化，随会话创建/打开拉取） */
  skills: { skills: SkillItem[]; loaded: boolean };
  /** 会话私有：输入框草稿（Tiptap 文档 JSON 序列化，ComposerBar 维护） */
  draftJson: string;
  /** 会话私有：输入框附件区（ComposerBar 维护） */
  draftAttachments: UserInput[];
  /** 会话私有：输入框内联引用 map（refId → 附件，ComposerBar 维护） */
  draftRefs: Record<string, UserInput>;
  /** 标签来源：new=新建会话（未发送首条消息），history=来自历史会话列表 */
  origin: "new" | "history" | null;
  workspace: string | null;
  resumedThreadId: string | null;
  turnActive: boolean;
  currentTurnId: string | null;
  turnInterrupted: boolean;
  goalText: string | null;
  goalStatus: GoalStatus | null;
  goalArmed: boolean;
  threadTokenUsage: {
    /** 最近一次请求的上下文占用（tokenUsage.last，非会话累计） */
    contextUsed: number;
    window: number | null;
    /** 会话累计输入 token（thread/tokenUsage/updated 的 total.inputTokens） */
    input?: number;
    /** 会话累计输出 token（thread/tokenUsage/updated 的 total.outputTokens） */
    output?: number;
    /** 会话累计总 token（total.totalTokens） */
    totalTokens?: number;
    /** 会话累计缓存读取输入（total.cachedInputTokens，输入的子集） */
    cachedInput?: number;
    /** 会话累计缓存写入输入（total.cacheWriteInputTokens，输入的子集） */
    cacheWriteInput?: number;
    /** 会话累计推理输出（total.reasoningOutputTokens，输出的子集） */
    reasoningOutput?: number;
  } | null;
  followupQueue: { text: string; attachments: UserInput[] }[];
  attachments: UserInput[];
  planPrompt: PlanPrompt | null;
  /** 当前回合的 Updated Plan 任务清单（turn/plan/updated 驱动，新回合重置） */
  plan: TurnPlan | null;
  loading: boolean;
  /** 会话私有：正在创建新会话并发送首条消息（startNewSession 流程中） */
  creatingSession: boolean;
  /** 新建对话时可选的项目目录（null = 使用启动工作目录） */
  newSessionWorkspace: string | null;
  /** 该标签待处理的交互（审批/提问/elicitation），按 threadId 路由 */
  interactions: PendingInteraction[];
}


/** @ 菜单中展示的插件条目（plugin/list 归一化结果） */
export interface PluginItem {
  id: string;
  name: string;
  displayName: string;
  description: string;
  path: string;
  iconPath: string;
  iconUrl: string;
  brandColor: string;
}

/** 插件目录条目（plugin/list 的 PluginSummary 归一化，供设置页插件管理） */
export interface PluginCatalogItem {
  id: string;
  name: string;
  remotePluginId: string | null;
  version: string | null;
  installed: boolean;
  enabled: boolean;
  availability: string;
  disabledReason: string | null;
  authPolicy: string;
  installPolicy: string;
  displayName: string;
  description: string;
  /** 本地图标路径（interface.composerIcon）；null = 无本地图标 */
  iconPath: string | null;
  /** 远程图标 URL（interface.composerIconUrl）；null = 无远程图标 */
  iconUrl: string | null;
  /** 品牌色（interface.brandColor），用于无图标时的首字母回退底色 */
  brandColor: string | null;
  keywords: string[];
}

/** 插件市场（plugin/list 的 PluginMarketplaceEntry 归一化，供设置页插件管理） */
export interface PluginMarketplaceInfo {
  name: string;
  /** 市场本地路径；null = 纯远程目录（官方 curated 等） */
  path: string | null;
  /** 安装入参分支：远程目录市场走 remoteMarketplaceName，带本地路径的市场走 marketplacePath */
  isRemote: boolean;
  displayName: string;
  plugins: PluginCatalogItem[];
}

/** plugin/list 返回的市场加载错误 */
export interface PluginMarketplaceLoadError {
  name: string;
  error: string;
}


/** $ 菜单与回显悬浮提示共用的技能条目（skills/list 归一化结果） */
export interface SkillItem {
  name: string;
  key: string;
  path: string;
  /** 长描述（悬浮提示用） */
  desc: string;
  /** 短描述（$ 菜单展示用，优先 interface.shortDescription） */
  shortDesc: string;
}

/** 右侧面板 Tab：资源管理器 / 会话历史 / Git */
export type PanelTab = "session" | "resources" | "git";


export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
}
