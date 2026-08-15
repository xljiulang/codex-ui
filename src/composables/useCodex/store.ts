// useCodex 拆分模块：全局 store 与共享模块状态（原 useCodex.ts 的一部分，纯移动，行为不变）
import { reactive } from "vue";
import type { PendingInteraction, PermissionId, ThreadItem, ThreadSummary, UserInput } from "../../lib/types";
import {
  defaultSettings,
  type ConfirmRequest,
  type GoalStatus,
  type ModelInfo,
  type PanelTab,
  type PlanPrompt,
  type PluginItem,
  type SkillItem,
} from "./types";


export const store = reactive({
  server: {
    connected: false,
    startupWorkspace: "",
    codexPath: null as string | null,
    logs: [] as string[],
  },
  threads: [] as ThreadSummary[],
  searchActive: false,
  searchSnippets: {} as Record<string, string>,
  currentThreadId: null as string | null,
  currentThreadName: "",
  currentThreadOrigin: null as "new" | "history" | null,
  currentThreadWorkspace: null as string | null,
  resumedThreadId: null as string | null,
  /** 活动标签工作区覆盖（文件/diff/预览/终端标签由 EditorPane 写入；null=跟随会话工作区） */
  workspace: null as string | null,
  itemsByThread: {} as Record<string, ThreadItem[]>,
  // 每个线程“进行中工作”计数（流式文本/进行中工具），避免渲染时全量扫描
  activeWorkByThread: {} as Record<string, number>,
  // 任意消息变更（新增/完成/流式增量）都会递增，供吸底滚动等做 O(1) 变更感知
  itemsRev: 0,
  // 用户手动发送计数器：每次 ComposerBar 提交（sendPrompt）递增，
  // 供 ChatView 在发送后强制恢复吸底（排队消息自动发送不递增）
  userSendRev: 0,
  turnActive: false,
  turnInterrupted: false,
  currentTurnId: null as string | null,
  followupQueue: [] as { text: string; attachments: UserInput[] }[],
  interactions: [] as PendingInteraction[],
  settings: defaultSettings(),
  // 启动加载态：init() 完成（含超时兜底）前为 true，App 据此显示加载动画
  booting: true,
  loadingHistory: false,
  loading: false,
  busy: false,
  currentModel: "",
  // 进程级设置：权限模式 / 模型 / 推理强度，仅当前运行期有效，不写入配置文件
  permissionMode: "ask-for-approval" as PermissionId,
  model: null as string | null,
  effort: null as string | null,
  models: [] as ModelInfo[],
  modelsLoaded: false,
  // 对话级插件缓存：key 为 currentThreadId（未创建会话时为 NEW_CHAT_PLUGIN_KEY）
  threadPlugins: {} as Record<
    string,
    { plugins: PluginItem[]; loaded: boolean }
  >,
  skills: [] as SkillItem[],
  skillsLoaded: false,
  threadTokenUsage: null as { used: number; window: number | null } | null,
  // 新建对话时可选的项目目录（null = 使用启动工作目录）
  newChatWorkspace: null as string | null,
  taskMode: "execute" as "execute" | "plan",
  goalText: null as string | null,
  /** 当前线程目标状态（thread/goal 事件同步，null = 未挂载目标） */
  goalStatus: null as GoalStatus | null,
  /** 目标 flag 勾选态：勾选后无目标值，首条消息纯文本即目标（纯客户端状态，不跨会话） */
  goalArmed: false,
  attachments: [] as UserInput[],
  showSettings: false,
  /** 右侧面板当前激活 Tab：会话/资源/Git，默认会话（首个 Tab） */
  panelTab: "history" as PanelTab,
  permOpen: false,
  taskOpen: false,
  modelOpen: false,
  toast: "",
  /** 全局确认弹窗（会话切换等需用户选择） */
  confirm: null as (ConfirmRequest & { resolve: (ok: boolean) => void }) | null,
  /** 计划模式回合完成后待用户确认的“计划已就绪”弹窗 */
  planPrompt: null as PlanPrompt | null,
});


/**
 * 后台临时线程 id 集合（如标题总结用的 ephemeral 线程）。
 * 这些线程的事件只由各自的一次性监听处理，不得进入全局 UI 状态，
 * 否则临时线程的 turn/completed 会把主对话的进行中状态误置为结束。
 */
export const backgroundThreadIds = new Set<string>();


export function isBackgroundThread(threadId: string | undefined | null): boolean {
  return !!threadId && backgroundThreadIds.has(threadId);
}
