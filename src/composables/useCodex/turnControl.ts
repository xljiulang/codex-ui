// useCodex 拆分模块：回合控制（原 useCodex.ts 的一部分，纯移动，行为不变）
import { invoke } from "@tauri-apps/api/core";
import { buildTurnInput } from "../../lib/mention";
import { toApprovalPolicy, toApprovalsReviewer, toSandboxPolicy } from "../../lib/permissions";
import type { UserInput } from "../../lib/types";
import { resolveSessionWorkspace, upsertItem } from "./items";
import { activeSessionTab, dropSessionTab, findSessionTabByThread } from "./sessionState";
import { currentModelId, resetToNewChat } from "./settings";
import { store } from "./store";
import { isThreadNotFound } from "./threads";
import { setToast, toastError } from "./toast";
import type { SessionTab } from "./types";


/**
 * 组装 turn/start 参数：权限/沙箱/模型/推理强度/协作模式按当前全局设置，
 * 沙箱可写根跟随传入的 cwd（活动标签用 resolveSessionWorkspace，后台标签用 resolveSessionWorkspace）。
 */
export function buildTurnParams(
  threadId: string,
  input: UserInput[],
  clientId: string,
  cwd: string,
  /** 发送目标标签的任务模式（后台标签传 tab.taskMode；缺省取活动会话模式） */
  taskMode?: "execute" | "plan",
): Record<string, unknown> {
  const params: Record<string, unknown> = {
    threadId,
    input,
    clientUserMessageId: clientId,
  };
  // 权限模式随每一轮发送（协议：本回合及后续回合生效），空闲期切换后立即生效
  params.approvalPolicy = toApprovalPolicy(store.permissionMode);
  params.sandboxPolicy = toSandboxPolicy(store.permissionMode, cwd);
  const reviewer = toApprovalsReviewer(store.permissionMode);
  if (reviewer) params.approvalsReviewer = reviewer;
  // 显式携带（null 表示用默认），避免旧值在会话里粘滞
  params.model = store.model ?? null;
  params.effort = store.effort ?? null;
  // 协作模式会粘滞在会话上：计划模式需要显式切回 default 才能退出；
  // 因此每轮都显式携带当前任务模式对应的 collaborationMode。
  // 模型未知时绝不发送空字符串（上游会报 invalid_request_error），此时省略该字段。
  const collabModel = currentModelId();
  if (collabModel) {
    const mode = taskMode ?? store.taskMode;
    params.collaborationMode = {
      mode: mode === "plan" ? "plan" : "default",
      settings: {
        model: collabModel,
        reasoning_effort: store.effort ?? null,
        developer_instructions: null,
      },
    };
  }
  return params;
}

/** 构造用户消息回合：生成 clientId、序列化输入、构建协议参数并乐观插入消息项 */
function buildUserTurn(
  threadId: string,
  prompt: string,
  attachments: UserInput[],
  cwd: string,
  taskMode?: "execute" | "plan",
): { clientId: string; input: UserInput[]; params: Record<string, unknown> } {
  const clientId = `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const input = buildTurnInput(prompt, attachments);
  const params = buildTurnParams(threadId, input, clientId, cwd, taskMode);
  upsertItem(threadId, {
    id: clientId,
    clientId,
    type: "userMessage",
    content: input,
    startedAtMs: Date.now(),
  });
  return { clientId, input, params };
}


/** 按标签发送回合（后台标签的队列消息等用）：状态写入目标标签记录，不触碰活动标签 */
export async function continueTurnForTab(
  tab: SessionTab,
  prompt: string,
  attachments: UserInput[],
) {
  const threadId = tab.threadId;
  if (!threadId) return;
  // 后台历史会话同样按需恢复；新会话（thread/start 创建）已订阅无需恢复
  if (tab.resumedThreadId !== threadId) {
    try {
      await invoke("thread_resume", { params: { threadId } });
      tab.resumedThreadId = threadId;
    } catch (e) {
      if (isThreadNotFound(e)) {
        dropSessionTab(tab);
        setToast("会话已不存在，已关闭该标签");
      } else {
        setToast(toastError(e));
      }
      return;
    }
  }
  const { params } = buildUserTurn(
    threadId,
    prompt,
    attachments,
    resolveSessionWorkspace(tab),
    tab.taskMode,
  );
  // 待挂载目标：先挂载再启动回合，失败清空该标签目标状态
  if (tab.goalText && !tab.goalStatus) {
    try {
      await invoke("goal_set", { threadId, objective: tab.goalText });
      tab.goalStatus = "active";
    } catch (e) {
      tab.goalText = null;
      tab.goalStatus = null;
      tab.goalArmed = false;
      setToast(toastError(e));
    }
  }
  try {
    const res = await invoke<{ turn?: { id?: string } }>("turn_start", {
      params,
    });
    tab.turnActive = true;
    if (res?.turn?.id) tab.currentTurnId = res.turn.id;
  } catch (e) {
    if (isThreadNotFound(e)) {
      dropSessionTab(tab);
      setToast("会话已不存在，已关闭该标签");
    } else {
      setToast(toastError(e));
    }
    tab.turnActive = false;
  }
}


export async function continueTurn(prompt: string, attachments: UserInput[]) {
  const threadId = store.currentThreadId;
  if (!threadId) return;
  // 历史会话在打开时只读、不恢复，避免带活跃目标的会话被自动持续执行；
  // 用户真正发消息时才恢复（thread/start 新建的会话已订阅，无需恢复）。
  if (store.resumedThreadId !== threadId) {
    try {
      await invoke("thread_resume", { params: { threadId } });
      store.resumedThreadId = threadId;
    } catch (e) {
      if (isThreadNotFound(e)) {
        resetToNewChat();
        setToast("会话已不存在，已切换为新会话");
      } else {
        setToast(toastError(e));
      }
      return;
    }
  }
  // 与 VS Code Codex 扩展一致：文件引用序列化成文本段落，作为单条 text 输入
  const { params } = buildUserTurn(
    threadId,
    prompt,
    attachments,
    resolveSessionWorkspace(),
  );
  // 待挂载目标（勾选后首条消息即目标）：先挂载再启动回合，服务端按目标线程自动续跑；
  // 挂载失败清空本地目标状态（toast 已由 setGoal 提示），不阻塞回合
  if (store.goalText && !store.goalStatus) {
    const ok = await setGoal(store.goalText);
    if (!ok) {
      store.goalText = null;
      store.goalStatus = null;
      store.goalArmed = false;
    }
  }
  try {
    const res = await invoke<{ turn?: { id?: string } }>("turn_start", { params });
    store.turnActive = true;
    // 立即记录回合 id，供 turn/interrupt 使用（turn/started 事件可能稍后才到）
    if (res?.turn?.id) store.currentTurnId = res.turn.id;
  } catch (e) {
    if (isThreadNotFound(e)) {
      resetToNewChat();
      setToast("会话已不存在，已切换为新会话");
    } else {
      setToast(toastError(e));
    }
    store.turnActive = false;
  }
}


/** 向进行中的回合追加输入（“调整方向”），协议 turn/steer */
export async function steerTurn(prompt: string, attachments: UserInput[]) {
  const threadId = store.currentThreadId;
  if (!threadId || !store.currentTurnId) {
    setToast("当前没有进行中的回合");
    return;
  }
  const { clientId, input } = buildUserTurn(
    threadId,
    prompt,
    attachments,
    resolveSessionWorkspace(),
  );
  try {
    await invoke("turn_steer", {
      params: {
        threadId,
        clientUserMessageId: clientId,
        input,
        expectedTurnId: store.currentTurnId,
      },
    });
  } catch (e) {
    const msg = String(e);
    if (msg.includes("no active turn")) {
      // 服务端在 turn/start 响应与 turn/started 事件之间可能尚未把回合
      // 标记为可转向；短暂重试几次，避免“no active turn to steer”导致输入丢失。
      for (let attempt = 0; attempt < 5; attempt++) {
        await new Promise((r) => setTimeout(r, 400));
        try {
          await invoke("turn_steer", {
            params: {
              threadId,
              clientUserMessageId: clientId,
              input,
              expectedTurnId: store.currentTurnId,
            },
          });
          return;
        } catch (e2) {
          const m2 = String(e2);
          if (!m2.includes("no active turn")) {
            setToast(toastError(e2));
            return;
          }
        }
      }
    }
    setToast(toastError(e));
  }
}


/** 标准停止回合：与停止按钮一致，线程有活跃目标时先清目标再 turn/interrupt；
 * 可显式传入线程/回合 id（切换会话时用），缺省时操作当前会话。 */
export async function interrupt(
  threadId?: string | null,
  turnId?: string | null,
) {
  const tid = threadId ?? store.currentThreadId;
  if (!tid) return;
  const tab = threadId ? findSessionTabByThread(tid) : activeSessionTab();
  // 线程有活跃目标：先清除目标切断服务端 auto-continuation（目标循环回合极快，
  // 回合中断可能追不上；清除目标后当前回合自然结束、不再自动续跑）
  const hasGoal = tab ? Boolean(tab.goalText) : Boolean(store.goalText);
  if (hasGoal) {
    await clearGoal(tid);
    // 后台标签的目标状态直接清本地（clearGoal 仅清活动会话本地状态）
    if (tab && tab.threadId === tid) {
      tab.goalText = null;
      tab.goalStatus = null;
      tab.goalArmed = false;
    }
  }
  // 显式线程路径（关闭标签/关窗守卫）只使用目标标签自己的回合 id，
  // 绝不借用活动标签的 store.currentTurnId，避免跨线程误中断；
  // 无显式线程（停止按钮）时 store 即活动会话的 live 字段，才允许取 store。
  let target = turnId;
  if (!target && threadId === undefined) target = store.currentTurnId;
  if (!target) return;
  // 服务端可能在 turn/started 事件之后才把回合标记为 active；
  // 若用户点得过早会收到 “no active turn”，短暂重试几次。
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      await invoke("turn_interrupt", {
        threadId: tid,
        turnId: target,
      });
      return;
    } catch (e) {
      const msg = String(e);
      // 回合 id 不一致时，错误里的 “but found X” 是服务端当前活跃回合 id，用它重试
      const found = /but found ([0-9a-fA-F-]+)/i.exec(msg);
      if (found && found[1] !== target) {
        target = found[1];
        // 仅在操作当前会话时同步 store，避免切换会话后把旧回合 id 写进新会话
        if (threadId === undefined) store.currentTurnId = target;
        continue;
      }
      if (msg.includes("no active turn")) {
        await new Promise((r) => setTimeout(r, 400));
        continue;
      }
      setToast(msg);
      return;
    }
  }
}


/** 为当前线程挂载目标（thread/goal/set）；目标设置成功后由服务端自动续跑回合 */
export async function setGoal(objective: string): Promise<boolean> {
  const text = objective.trim();
  if (!text) {
    setToast("目标不能为空");
    return false;
  }
  if (text.length > 4000) {
    setToast("目标最长 4000 字符");
    return false;
  }
  const tid = store.currentThreadId;
  if (!tid) return false; // 仅在线程存在时挂载（防御：调用方应保证有会话）
  try {
    await invoke("goal_set", { threadId: tid, objective: text });
    store.goalText = text;
    store.goalStatus = "active";
    setToast("已设置目标");
    return true;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}


export async function clearGoal(threadId?: string | null) {
  const tid = threadId ?? store.currentThreadId;
  if (!tid) {
    store.goalText = null;
    store.goalStatus = null;
    store.goalArmed = false;
    return;
  }
  try {
    await invoke("goal_clear", { threadId: tid });
    // 仅当清除的是当前会话时才清空本地目标展示；
    // 切换会话时对旧线程的清除不应污染新会话的目标状态
    if (tid === store.currentThreadId) {
      store.goalText = null;
      store.goalStatus = null;
      store.goalArmed = false;
    }
  } catch (e) {
    setToast(toastError(e));
  }
}
