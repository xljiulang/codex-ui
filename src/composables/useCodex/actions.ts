// useCodex 拆分模块：会话/发送/计划/目标动作（原 useCodex.ts 的一部分，纯移动，行为不变）
import { nextTick, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { focusComposer } from "../../lib/composerFocus";
import { buildInjectedDynamicTools } from "../../lib/dynamicTools";
import { stripMentionContext } from "../../lib/mention";
import { toApprovalPolicy, toApprovalsReviewer, toSandbox } from "../../lib/permissions";
import { sessionLog } from "../../lib/sessionLog";
import { TabKind } from "../../lib/tabs";
import type {
  PendingInteraction,
  ThreadSummary,
  Turn,
  UserInput,
} from "../../lib/types";
import { activeTab, activateTab, insertTab, tabs } from "../useTabs";
import { flattenTurns, isActiveItem, loadFullItems, resolveSessionWorkspace, workspace } from "./items";
import {
  activeSessionTab,
  allSessionTabs,
  dropSessionTab,
  findSessionTabByThread,
  freshSessionTab,
  hydrateSessionState,
  removeSessionState,
  saveSessionState,
  sessionTabTitle,
} from "./sessionState";
import {
  effectiveSessionModelEffort,
  ensureSkills,
  ensureThreadPlugins,
  resetToNewSession,
} from "./settings";
import { switchSessionTab } from "./sessionTabs";
import { store } from "./store";
import {
  autoTitleThread,
  isThreadNotFound,
  refreshThreads,
  renameThread,
  sanitizeTitle,
  upsertThreadSummary,
} from "./threads";
import { setToast, toastError } from "./toast";
import { clearGoal, startTurn, setGoal, steerTurn } from "./turnControl";
import { isThreadBound, wechatUnbind } from "./wechat";
import {
  goalStatusToast,
  isGoalStatus,
  isGoalTerminalStatus,
  type GoalStatus,
  type SessionTab,
} from "./types";


/**
 * 打开历史会话的统一入口（唯一性约束）：已有标签绑定该线程则直接切换（不新建、
 * 不重载）；否则新建标签并加载。返回 true 表示已进入目标会话。
 */
export async function openSessionTabForThread(
  threadId: string,
): Promise<boolean> {
  // 仅当目标线程的会话标签是真正的活动标签时短路：文件/终端标签活动时
  // activeSessionTab() 返回的是“最近投影”会话而非显示中的标签，点击历史会话
  // 仍必须走 switchSessionTab 把对应会话标签切回前台。
  const active = activeTab.value;
  if (active && active.kind === TabKind.Session && active.threadId === threadId) {
    return true;
  }
  void sessionLog("info", threadId, "session-open");
  const existing = findSessionTabByThread(threadId);
  if (existing) {
    await switchSessionTab(existing.id);
    return true;
  }
  const tab = freshSessionTab();
  tab.threadId = threadId; // 先绑定，加载期间也满足唯一性
  insertTab(tab);
  if (!(await switchSessionTab(tab.id))) {
    const i = tabs.indexOf(tab);
    if (i >= 0) tabs.splice(i, 1);
    return false;
    }
    tab.origin = "history";
    await hydrateSessionState(tab);
    return await loadThreadInto(tab, threadId);
  }


async function startNewSession(prompt: string, attachments: UserInput[]) {
  const active = activeSessionTab();
  const tabId = active?.id ?? null;
  // 创建中标记写发起时的标签；即使创建期间用户切换标签，finally 也按 tabId 清回原标签
  if (active) active.creatingSession = true;
  try {
    // startNewSession 仅在无当前会话时被调用，resolveSessionWorkspace 走 newSessionWorkspace → workspace 分支
    const cwd = resolveSessionWorkspace(active ?? undefined);
    const permission =
      active?.permissionMode ?? store.settings.default_permission;
    const params: Record<string, unknown> = {
      approvalPolicy: toApprovalPolicy(permission),
      sandbox: toSandbox(permission),
    };
    // 注入 codexui 动态工具：agent 可在会话内查询用量/压缩上下文（仅 thread/start 支持）；
    // 按设置中被禁用的工具过滤，全部禁用则整体不注入
    const dynamicTools = buildInjectedDynamicTools(
      store.settings.dynamic_tools_disabled,
    );
    if (dynamicTools.length) params.dynamicTools = dynamicTools;
    // 无确定工作区时不传 cwd（交由 codex 用服务端默认目录），避免 cwd 为空串
    if (cwd) params.cwd = cwd;
    const reviewer = toApprovalsReviewer(permission);
    if (reviewer) params.approvalsReviewer = reviewer;
    // 显式携带（null 表示用默认），避免旧值在会话里粘滞；effort 由随后的 turn/start 携带
    params.model = active?.model ?? null;
    const res = await invoke<{ thread: ThreadSummary; model?: string }>(
      "thread_start",
      { params },
    );
    const threadId = res.thread.id;
    // 防御：该新线程已被其它标签绑定（异常路径），聚焦已有标签并释放当前标签
    const existing = findSessionTabByThread(threadId);
    if (existing) {
      const cur = activeSessionTab();
      if (cur) dropSessionTab(cur);
      activateTab(existing.id);
      await refreshThreads();
      return;
    }
    // 创建期间用户已切换到其它标签：结果直接写入原标签记录，避免污染当前会话
    if (tabId !== null && activeSessionTab()?.id !== tabId) {
      const tab = tabs.find(
        (t): t is SessionTab => t.kind === TabKind.Session && t.id === tabId,
      );
      if (tab) {
        tab.threadId = threadId;
        tab.name = res.thread.name ?? "";
        tab.origin = "new";
        tab.workspace = cwd;
        tab.resumedThreadId = threadId;
        tab.newSessionWorkspace = null;
        tab.loading = false;
        tab.title = sessionTabTitle(tab);
        const resolved = effectiveSessionModelEffort(tab);
        tab.model = resolved.model;
        tab.effort = resolved.effort;
        void saveSessionState(tab, resolved);
      }
      // 面板一致性兜底：新线程可能尚未被 thread_list 返回，本地先写入摘要
      upsertThreadSummary({
        id: threadId,
        name: tab?.name || res.thread.name || null,
        createdAt: res.thread.createdAt ?? Date.now(),
        updatedAt: res.thread.updatedAt ?? Date.now(),
        recencyAt: res.thread.recencyAt ?? Date.now(),
        cwd,
        source: res.thread.source ?? "appServer",
      });
      store.itemsByThread[threadId] = [];
      store.activeWorkByThread[threadId] = 0;
      await refreshThreads();
      return;
    }
    const activeTab = activeSessionTab();
    if (activeTab) {
      activeTab.threadId = threadId;
      activeTab.name = res.thread.name ?? "";
      activeTab.origin = "new";
      activeTab.workspace = cwd;
      activeTab.resumedThreadId = threadId;
      activeTab.newSessionWorkspace = null; // 本次新建已消费，恢复默认
      activeTab.loading = false;
      activeTab.title = sessionTabTitle(activeTab);
      const resolved = effectiveSessionModelEffort(activeTab);
      activeTab.model = resolved.model;
      activeTab.effort = resolved.effort;
      void saveSessionState(activeTab, resolved);
    }
    // 会话级插件/技能缓存兜底：标签创建时已预取，此处失败重试
    if (activeTab) {
      void ensureThreadPlugins(activeTab);
      void ensureSkills(activeTab);
    }
    store.itemsByThread[threadId] = [];
    store.activeWorkByThread[threadId] = 0;
    // 面板一致性兜底：新线程可能尚未被 thread_list 返回，本地先写入摘要
    upsertThreadSummary({
      id: threadId,
      name: activeTab?.name || res.thread.name || null,
      createdAt: res.thread.createdAt ?? Date.now(),
      updatedAt: res.thread.updatedAt ?? Date.now(),
      recencyAt: res.thread.recencyAt ?? Date.now(),
      cwd,
      source: res.thread.source ?? "appServer",
    });
    // 待挂载目标（勾选后首条消息即目标）：创建会话后挂载到新线程；失败不阻塞新建，
    // 清空本地目标状态，用户可重新勾选
    const pendingGoal = activeSessionTab()?.goalText;
    if (pendingGoal) {
      const ok = await setGoal(pendingGoal);
      if (!ok) {
        const t = activeSessionTab();
        if (t) {
          t.goalText = null;
          t.goalStatus = null;
          t.goalArmed = false;
        }
      }
    }
    await refreshThreads();
    if (prompt.trim() || attachments.length) {
      // 第 2 步：首条消息内容作为会话标题（AI 总结完成后由 autoTitleThread 覆盖）
      const firstText = stripMentionContext(prompt).replace(/\s+/g, " ").trim();
      if (firstText) {
        const tab = findSessionTabByThread(threadId);
        if (
          await renameThread(threadId, sanitizeTitle(firstText), "first-message")
        ) {
          if (tab) tab.nameIsFirstMessage = true;
        }
      }
      // 仿 VS Code：后台临时线程总结首条消息生成短标题（不阻塞主回合）
      void autoTitleThread(threadId, stripMentionContext(prompt));
      await startTurn(prompt, attachments);
    }
  } finally {
    const target = tabId
      ? allSessionTabs().find((t) => t.id === tabId)
      : undefined;
    if (target) target.creatingSession = false;
  }
}


export async function sendPrompt(text: string, invertFollowup = false) {
  const tab = activeSessionTab();
  const attachments = tab?.attachments.splice(0) ?? [];
  if (!text.trim() && attachments.length === 0) return;
  void sessionLog("info", tab?.threadId ?? null, "user-send", `chars=${text.length}`);
  // 手动发送标记：ChatView 据此在发送后强制恢复吸底回到底部
  // （队列消息在回合结束后自动发送时走 startTurn/startNewSession，不递增）
  store.userSendRev++;
  // 回合进行中：按“跟进处理方式”转向或入队；Ctrl+Enter 对单条消息取相反方式
  if (tab?.turnActive && tab.threadId) {
    const base = store.settings.followup_mode;
    const mode = invertFollowup ? (base === "adjust" ? "queue" : "adjust") : base;
    if (mode === "adjust") {
      await steerTurn(text, attachments);
    } else {
      tab.followupQueue.push({ text, attachments });
      setToast("已加入队列，回合结束后自动发送");
    }
    return;
  }
  try {
    if (!tab?.threadId) {
      await startNewSession(text, attachments);
    } else {
      await startTurn(text, attachments);
    }
  } catch (e) {
    setToast(toastError(e));
  }
}


/** “待在计划”：关闭“计划已就绪”弹窗，保持计划模式，不发消息（Esc 同此行为） */
export function dismissPlanPrompt() {
  const tab = activeSessionTab();
  if (tab) tab.planPrompt = null;
}


/** “退出计划模式”：切回默认模式并关闭弹窗，不发消息 */
export function exitPlanMode() {
  const tab = activeSessionTab();
  if (tab) {
    tab.planPrompt = null;
    tab.collaborationMode = "default";
  }
}


/** “执行计划”：仿 VS Code —— 发送 `PLEASE IMPLEMENT THIS PLAN:` 消息并切到默认模式 */
export async function executePlan() {
  const tab = activeSessionTab();
  const prompt = tab?.planPrompt;
  if (!prompt) return;
  if (tab) tab.planPrompt = null;
  // 先切模式，使本轮 turn/start 显式携带 collaborationMode default（计划模式粘滞，需显式退出）
  if (tab) tab.collaborationMode = "default";
  const text = `PLEASE IMPLEMENT THIS PLAN:\n${prompt.planText}`;
  // 目标勾选：执行计划即首条执行消息，目标=该合成消息（含计划全文）
  if (tab?.goalArmed) {
    tab.goalText = text;
    tab.goalArmed = false;
    tab.goalStatus = null;
  }
  try {
    if (tab?.threadId) {
      await startTurn(text, []);
    } else {
      await startNewSession(text, []);
    }
  } catch (e) {
    setToast(toastError(e));
  }
}


/**
 * 新建空会话标签（所有“新会话”入口的统一函数）：会话多开，不打断/不停止
 * 当前或后台标签的回合；可预置本次会话的工作目录 cwd。恒返回 true。
 */
export async function newEmptySession(cwd?: string | null): Promise<boolean> {
  const tab = freshSessionTab();
  if (cwd) {
    tab.newSessionWorkspace = cwd;
    store.lastWorkspace = cwd;
  }
  tab.title = sessionTabTitle(tab);
  insertTab(tab);
  activateTab(tab.id);
  void ensureThreadPlugins(tab); // 进入新对话编辑态即预初始化插件缓存
  void ensureSkills(tab); // 技能列表随会话拉取（$ 菜单与回显共用）
  return true;
}


/**
 * 把历史会话加载进指定会话标签（结果按“是否仍为活动标签”写入 live 字段或标签记录，
 * 避免加载期间用户切换标签导致状态串味）。
 */
async function loadThreadInto(tab: SessionTab, threadId: string): Promise<boolean> {
  const isActive = () => activeSessionTab()?.id === tab.id;
  tab.loading = true;
  void ensureThreadPlugins(tab); // 进入历史对话即预初始化插件缓存
  void ensureSkills(tab); // 技能列表随会话拉取
  try {
    // 统一只读元数据：codex CLI 创建的分页线程（historyMode=paginated）不支持
    // includeTurns=true；完整消息一律走 thread/turns/list 分页，legacy 线程在
    // turns/list 不可用时才回退 includeTurns=true 摘要读取
    const res = await invoke<{
      thread: {
        id: string;
        name?: string | null;
        preview?: string;
        cwd?: string | null;
        historyMode?: string | null;
        turns?: Turn[];
      };
    }>("thread_read", { threadId, includeTurns: false });
    const historyMode = res.thread.historyMode;
    // 优先用全量 items（含命令/工具详情），失败时按 historyMode 分流回退
    let fullItems = await loadFullItems(threadId);
    if (!fullItems) {
      if (historyMode === "paginated") {
        // 分页线程不支持 includeTurns=true，保持空列表打开（仍可继续发送消息）
        fullItems = [];
        setToast("该会话为分页存储，当前 Codex 版本无法读取历史，仍可继续发送消息");
      } else {
        try {
          const legacy = await invoke<{ thread: { turns?: Turn[] } }>(
            "thread_read",
            { threadId, includeTurns: true },
          );
          fullItems = flattenTurns(legacy.thread.turns);
        } catch {
          fullItems = [];
        }
      }
    }
    store.itemsByThread[threadId] = fullItems;
    store.activeWorkByThread[threadId] = (
      store.itemsByThread[threadId] ?? []
    ).filter((x) => isActiveItem(x)).length;
    const name = res.thread.name ?? "";
    const cwd = res.thread.cwd ?? null;
    if (cwd) store.lastWorkspace = cwd;
    // 回合/计划/目标等状态一律写标签（tab 是唯一事实源，不再写 store）
    tab.threadId = threadId;
    tab.name = name;
    tab.workspace = cwd;
    tab.resumedThreadId = null;
    tab.turnActive = false;
    tab.turnInterrupted = false;
    tab.currentTurnId = null;
    tab.threadTokenUsage = null;
    tab.goalArmed = false; // 勾选态不跨会话；服务端目标经 goal_get 回填
    tab.title = sessionTabTitle(tab);
    let goalText: string | null = null;
    let goalStatus: GoalStatus | null = null;
    try {
      const g = await invoke<{
        objective?: string;
        status?: string;
        goal?: { objective?: string; status?: string };
      }>("goal_get", { threadId });
      const goal = g?.goal ?? g;
      goalText = goal?.objective ?? null;
      goalStatus = isGoalStatus(goal?.status) ? goal.status : null;
    } catch {
      // 服务端不支持/失败：按无目标处理
    }
    // 终态目标（已完成/受限等）：打开即 toast 提示并复位（相当于没有目标），服务端同步清除
    if (goalStatus && isGoalTerminalStatus(goalStatus)) {
      setToast(goalStatusToast(goalStatus));
      goalText = null;
      goalStatus = null;
      void clearGoal(threadId);
    }
    tab.goalText = goalText;
    tab.goalStatus = goalStatus;
    // 打开即恢复：让服务端推送 thread/tokenUsage/updated（token 用量唯一来源），
    // 使右上角立即显示累计用量。带活跃目标（goal 非终态）的会话不恢复，
    // 避免「只看一眼」就触发服务端围绕目标自动续跑；用户真正发消息时才恢复。
    if (!goalStatus) {
      try {
        await invoke("thread_resume", { params: { threadId } });
        tab.resumedThreadId = threadId;
      } catch (e) {
        if (isThreadNotFound(e)) throw e; // 交给外层“会话已不存在”分支处理
        // 其它失败：不阻断打开，token 用量可能随后续事件补上
      }
    }
    return true;
  } catch (e) {
    if (isThreadNotFound(e)) {
      if (isActive()) {
        resetToNewSession();
        setToast("会话已不存在，已切换为新会话");
      } else {
        dropSessionTab(tab);
        setToast("会话已不存在，已关闭该标签");
      }
    } else {
      setToast(toastError(e));
    }
    return false;
  } finally {
    tab.loading = false;
  }
}


/** 切换会话成功后的统一收尾：聚焦输入框（右侧面板保持当前 Tab） */
async function finishSessionSwitch() {
  await nextTick();
  focusComposer();
}


/**
 * 新建会话统一入口（头部按钮 / 历史目录右键「新建会话」）：
 * 切换成功（未被取消）才聚焦输入框；右侧面板保持当前 Tab。
 */
export async function openNewSession(cwd?: string | null): Promise<void> {
  if (!(await newEmptySession(cwd))) return;
  await finishSessionSwitch();
}


/** 目录选择对话框打开中：禁止重复触发（头部「+」与空状态按钮共用，供按钮禁用态绑定） */
export const pickingNewSessionDir = ref(false);


/**
 * 选择工作目录并新建会话（头部「+」与零会话空状态按钮共用）：
 * 先弹目录选择（初始定位当前维护的工作目录），取消则流程直接结束。
 */
export async function pickAndOpenNewSession(): Promise<void> {
  if (pickingNewSessionDir.value) return;
  pickingNewSessionDir.value = true;
  try {
    const dir = await invoke<string | null>("pick_directory", {
      // 有工作区（会话/编辑器标签）时以其为起点；无工作区时用最近一次，无则空
      initialDir: workspace.value || store.lastWorkspace || "",
    });
    if (!dir) return;
    store.lastWorkspace = dir;
    await openNewSession(dir);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pickingNewSessionDir.value = false;
  }
}


/**
 * 打开历史会话统一入口（会话行单击 / 右键「打开」）：
 * 切换成功（未被取消）才聚焦输入框；右侧面板保持当前 Tab。
 */
export async function openSession(threadId: string): Promise<void> {
  // 唯一性约束：已打开则聚焦，未打开则新建标签加载
  if (!(await openSessionTabForThread(threadId))) return;
  await finishSessionSwitch();
}


export async function deleteThread(threadId: string) {
  try {
    // 删除已绑定微信的会话前先解除绑定（单条删除与「删除所有会话」组删共用此路径）
    if (isThreadBound(threadId)) {
      await wechatUnbind(threadId).catch(() => {});
    }
    await invoke("thread_delete", { threadId });
    void sessionLog("warn", threadId, "thread-delete");
    store.threads = store.threads.filter((t) => t.id !== threadId);
      // 释放该会话的本地缓存，避免历史列表长期累积内存
      delete store.itemsByThread[threadId];
      delete store.activeWorkByThread[threadId];
      // 清空会话统一状态记录
      void removeSessionState(threadId);
      // 关闭绑定该线程的会话标签（线程已删除，无需确认/中断）
    for (const tab of allSessionTabs().filter((t) => t.threadId === threadId)) {
      dropSessionTab(tab);
    }
  } catch (e) {
    setToast(toastError(e));
  }
}


/**
 * 从历史会话分叉出新会话（整会话复制）：调用 thread/fork 复制源线程为新线程，
 * 打开新会话标签并沿用源名称；失败只提示、不新建标签。
 */
export async function forkThread(threadId: string) {
  const source = store.threads.find((t) => t.id === threadId);
  try {
    const res = await invoke<{
      thread: {
        id: string;
        name?: string | null;
        createdAt?: number;
        updatedAt?: number;
        recencyAt?: number | null;
        cwd?: string | null;
        source?: string;
        historyMode?: string | null;
      };
    }>("thread_fork", { params: { threadId } });
    const newId = res.thread.id;
    const newName = res.thread.name ?? "";
    // 面板一致性兜底：新线程可能尚未被 thread_list 返回，本地先写入摘要
    upsertThreadSummary({
      id: newId,
      name: newName || source?.name || null,
      createdAt: res.thread.createdAt ?? Date.now(),
      updatedAt: res.thread.updatedAt ?? Date.now(),
      recencyAt: res.thread.recencyAt ?? Date.now(),
      cwd: res.thread.cwd ?? source?.cwd ?? undefined,
      source: res.thread.source ?? "appServer",
      historyMode: res.thread.historyMode ?? "",
    });
    await openSessionTabForThread(newId);
    // 沿用源会话名称：仅当源有名称且新线程无名称时写入，覆盖服务端给定名
    if (source?.name && !newName) {
      await renameThread(newId, source.name);
    }
    void sessionLog("info", threadId, "thread-fork");
    setToast("已创建分叉会话");
    await finishSessionSwitch();
  } catch (e) {
    setToast(toastError(e));
  }
}


export async function respondInteraction(interaction: PendingInteraction, result: unknown) {
  try {
    await invoke("interaction_respond", {
      requestId: interaction.requestId,
      result,
    });
  } catch (e) {
    setToast(toastError(e));
  } finally {
    store.interactions = store.interactions.filter((i) => i.requestId !== interaction.requestId);
    for (const tab of allSessionTabs()) {
      if (tab.interactions?.some((i) => i.requestId === interaction.requestId)) {
        tab.interactions = tab.interactions.filter(
          (i) => i.requestId !== interaction.requestId,
        );
      }
    }
  }
}

/** 历史会话按需恢复（模型切换等操作需要线程已恢复）；返回是否可继续 */
export async function ensureThreadResumed(tab: SessionTab): Promise<boolean> {
  const threadId = tab.threadId;
  if (!threadId || tab.resumedThreadId === threadId) return true;
  try {
    await invoke("thread_resume", { params: { threadId } });
    tab.resumedThreadId = threadId;
    return true;
  } catch (e) {
    if (isThreadNotFound(e)) {
      resetToNewSession();
      setToast("会话已不存在，已切换为新会话");
    } else {
      setToast(toastError(e));
    }
    return false;
  }
}
