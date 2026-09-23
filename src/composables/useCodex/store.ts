// useCodex 拆分模块：全局 store 与共享模块状态（原 useCodex.ts 的一部分，纯移动，行为不变）
import { reactive } from "vue";
import type {
  WeChatSnapshot,
  PendingInteraction,
  ScheduledTask,
  ServerStatus,
  ThreadItem,
  ThreadSummary,
} from "../../lib/types";
import {
  defaultSettings,
  type ConfirmRequest,
  type ModelInfo,
  type PanelTab,
} from "./types";

export const store = reactive({
  server: {
    connected: false,
    codexPath: null as string | null,
    codexVersion: null as string | null,
    versionTooOld: undefined as boolean | undefined,
    logs: [] as string[],
  } as ServerStatus,
  threads: [] as ThreadSummary[],
  searchActive: false,
  searchSnippets: {} as Record<string, string>,
  /** 活动标签工作区覆盖（文件/diff/预览/终端标签由 EditorPane 写入；null=跟随会话工作区） */
  workspace: null as string | null,
  /** 最近一次使用的工作区（仅作目录选择器初始起点，内存态非持久化） */
  lastWorkspace: "" as string,
  itemsByThread: {} as Record<string, ThreadItem[]>,
  // 每个线程“进行中工作”计数（流式文本/进行中工具），避免渲染时全量扫描
  activeWorkByThread: {} as Record<string, number>,
  // 任意消息变更（新增/完成/流式增量）都会递增，供吸底滚动等做 O(1) 变更感知
  itemsRev: 0,
  // 用户手动发送计数器：每次 ComposerBar 提交（sendPrompt）递增，
  // 供 ChatView 在发送后强制恢复吸底（排队消息自动发送不递增）
  userSendRev: 0,
  interactions: [] as PendingInteraction[],
  settings: defaultSettings(),
  /** 微信接入桥状态快照（进入设置页后由 wechat_state / 事件填充） */
  wechat: null as WeChatSnapshot | null,
  // 启动加载态：init() 完成（含超时兜底）前为 true，App 据此显示加载动画
  booting: true,
  loadingSessions: false,
  models: [] as ModelInfo[],
  modelsLoaded: false,
  /** 右侧面板当前激活 Tab：会话/资源/Git，默认会话（首个 Tab） */
  panelTab: "session" as PanelTab,
  /** 右侧面板显隐：标题栏布局按钮切换（仅本次运行生效，不持久化） */
  rightPanelHidden: false,
  /** 启动恢复会话时请求会话列表展开其目录分组（值为线程 id，消费后清空） */
  pendingExpandGroupThread: "",
  /** 定时任务列表（scheduled-tasks/event 快照驱动，管理区块消费） */
  scheduledTasks: [] as ScheduledTask[],
  /** 最近一次定时任务事件（seq 逐次递增保证可观察；taskId 为变更任务，供展开记录定向刷新） */
  scheduledTaskChange: { seq: 0, taskId: "" },
  toast: "",
  /** 全局确认弹窗（会话切换等需用户选择） */
  confirm: null as (ConfirmRequest & { resolve: (ok: boolean) => void }) | null,
});

/**
 * 后台临时线程 id 集合（如标题总结用的 ephemeral 线程）。
 * 这些线程的事件只由各自的一次性监听处理，不得进入全局 UI 状态，
 * 否则临时线程的 turn/completed 会把主对话的进行中状态误置为结束。
 */
export const backgroundThreadIds = new Set<string>();

export function isBackgroundThread(
  threadId: string | undefined | null,
): boolean {
  return !!threadId && backgroundThreadIds.has(threadId);
}
