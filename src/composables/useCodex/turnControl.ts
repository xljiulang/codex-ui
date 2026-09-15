// useCodex 拆分模块：回合控制（原 useCodex.ts 的一部分，纯移动，行为不变）
import { invoke } from "@tauri-apps/api/core";
import { buildTurnInput } from "../../lib/mention";
import { toApprovalPolicy, toApprovalsReviewer, toSandboxPolicy } from "../../lib/permissions";
import { extractErrorMessage } from "../../lib/serverMessages";
import { sessionLog } from "../../lib/sessionLog";
import type { UserInput } from "../../lib/types";
import { upsertItem } from "./items";
import {
  activeSessionTab,
  applyResumedSettings,
  dropSessionTab,
  findSessionTabByThread,
} from "./sessionState";
import { currentModelId, effectiveEffort, resetToNewSession } from "./settings";
import { store } from "./store";
import { isThreadNotFound } from "./threads";
import { setToast, toastError } from "./toast";
import { notifyCodexError } from "./errorNotify";
import type { SessionTab } from "./types";


/**
 * 组装 turn/start 参数：权限/沙箱/模型/推理强度/协作模式按发送目标标签（session）取值。
 */
export function buildTurnParams(
  threadId: string,
  input: UserInput[],
  clientId: string,
  /** 发送目标标签（会话设置唯一事实源）；缺省时权限用默认设置、模型/强度为空 */
  session?: Pick<
    SessionTab,
    "permissionMode" | "model" | "effort" | "collaborationMode"
  >,
): Record<string, unknown> {
  const params: Record<string, unknown> = {
    threadId,
    input,
    clientUserMessageId: clientId,
  };
  // 权限模式随每一轮发送（协议：本回合及后续回合生效），空闲期切换后立即生效
  const permission = session?.permissionMode ?? store.settings.default_permission;
  params.approvalPolicy = toApprovalPolicy(permission);
  params.sandboxPolicy = toSandboxPolicy(permission);
  const reviewer = toApprovalsReviewer(permission);
  if (reviewer) params.approvalsReviewer = reviewer;
  // 显式携带（null 表示用默认）；注意：携带 collaborationMode 时服务端以
  // collaborationMode.settings 为准，下面两个字段实际被覆盖（保留仅为字段完整性）
  params.model = session?.model ?? null;
  params.effort = session?.effort ?? null;
  // 协作模式会粘滞在会话上：计划模式需要显式切回 default 才能退出；
  // 因此每轮都显式携带当前会话协作模式对应的 collaborationMode 参数。
  // 三面独立映射：mode 只由标签 collaborationMode 决定，settings.model 只由 currentModelId 解析；
  // 协议要求 settings.model 为非空 string（实测 null 会被服务端拒绝），
  // 模型无法解析时 currentModelId 抛错，由发送方 toast 提示并中止发送。
  const collabModel = currentModelId(session);
  // mode 与标签 collaborationMode 恒等（plan/default），直接取值，无需翻译映射
  const mode = session?.collaborationMode ?? "default";
  params.collaborationMode = {
    mode,
    settings: {
      model: collabModel,
      // 显式解析默认强度：collaborationMode 会整体替换服务端设置，传 null 会回落到
      // 模型自带默认而非 config.toml 的 model_reasoning_effort
      reasoning_effort: session?.effort ?? (effectiveEffort(session) || null),
      developer_instructions: null,
    },
  };
  return params;
}

/** 构造用户消息回合：生成 clientId、序列化输入、构建协议参数并乐观插入消息项 */
function buildUserTurn(
  threadId: string,
  prompt: string,
  attachments: UserInput[],
  session?: Pick<
    SessionTab,
    "permissionMode" | "model" | "effort" | "collaborationMode"
  >,
): { clientId: string; input: UserInput[]; params: Record<string, unknown> } {
  const clientId = `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const input = buildTurnInput(prompt, attachments);
  const params = buildTurnParams(threadId, input, clientId, session);
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
export async function startTurnForTab(
  tab: SessionTab,
  prompt: string,
  attachments: UserInput[],
) {
  const threadId = tab.threadId;
  if (!threadId) return;
  // 后台历史会话同样按需恢复；新会话（thread/start 创建）已订阅无需恢复
  if (tab.resumedThreadId !== threadId) {
    try {
      const res = await invoke("thread_resume", { params: { threadId } });
      applyResumedSettings(tab, res);
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
  try {
    // 参数构建失败（如无可用模型）时走统一错误路径，toast 提示并中止发送
    const { params } = buildUserTurn(
      threadId,
      prompt,
      attachments,
      tab,
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
    const collabMode = (params.collaborationMode as { mode?: string } | undefined)?.mode ?? null;
    sessionLog("info", threadId, "turn-start-mode", `collaborationMode.mode=${collabMode ?? "?"}`);
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
      // 回合没发出去：窗口没被看到时也发一条系统通知（无有效会话时不发）；
      // 不传 turnId——回合并未建立，标签上的 currentTurnId 属于上一回合，
      // 带上它会让后端把「同一回合」的历史通知当成重复而误压本次。
      notifyCodexError({ message: extractErrorMessage(e), threadId });
    }
    tab.turnActive = false;
  }
}


export async function startTurn(prompt: string, attachments: UserInput[]) {
  const tab = activeSessionTab();
  const threadId = tab?.threadId;
  if (!threadId) return;
  // 历史会话在打开时只读、不恢复，避免带活跃目标的会话被自动持续执行；
  // 用户真正发消息时才恢复（thread/start 新建的会话已订阅，无需恢复）。
  if (tab?.resumedThreadId !== threadId) {
    try {
      const res = await invoke("thread_resume", { params: { threadId } });
      if (tab) {
        applyResumedSettings(tab, res);
        tab.resumedThreadId = threadId;
      }
    } catch (e) {
      if (isThreadNotFound(e)) {
        resetToNewSession();
        setToast("会话已不存在，已切换为新会话");
      } else {
        setToast(toastError(e));
      }
      return;
    }
  }
  try {
    // 与 VS Code Codex 扩展一致：文件引用序列化成文本段落，作为单条 text 输入；
    // 参数构建失败（如无可用模型）时走统一错误路径，toast 提示并中止发送
    const { params } = buildUserTurn(
      threadId,
      prompt,
      attachments,
      tab,
    );
    // 待挂载目标（勾选后首条消息即目标）：先挂载再启动回合，服务端按目标线程自动续跑；
    // 挂载失败清空本地目标状态（toast 已由 setGoal 提示），不阻塞回合
    if (tab?.goalText && !tab.goalStatus) {
      const ok = await setGoal(tab.goalText);
      if (!ok) {
        tab.goalText = null;
        tab.goalStatus = null;
        tab.goalArmed = false;
      }
    }
    const collabMode = (params.collaborationMode as { mode?: string } | undefined)?.mode ?? null;
    sessionLog("info", threadId, "turn-start-mode", `collaborationMode.mode=${collabMode ?? "?"}`);
    const res = await invoke<{ turn?: { id?: string } }>("turn_start", { params });
    if (tab) {
      // 回合状态写活动标签（tab 是唯一事实源）
      tab.turnActive = true;
      // 立即记录回合 id，供 turn/interrupt 使用（turn/started 事件可能稍后才到）
      if (res?.turn?.id) tab.currentTurnId = res.turn.id;
    }
  } catch (e) {
    if (isThreadNotFound(e)) {
      resetToNewSession();
      setToast("会话已不存在，已切换为新会话");
    } else {
      setToast(toastError(e));
      // 回合没发出去：窗口没被看到时也发一条系统通知（不传 turnId，理由同上）
      notifyCodexError({ message: extractErrorMessage(e), threadId });
    }
    if (tab) tab.turnActive = false;
  }
}


/** 向进行中的回合追加输入（“调整方向”），协议 turn/steer */
export async function steerTurn(prompt: string, attachments: UserInput[]) {
  const tab = activeSessionTab();
  const threadId = tab?.threadId;
  const turnId = tab?.currentTurnId ?? null;
  if (!threadId || !turnId) {
    setToast("当前没有进行中的回合");
    return;
  }
  const built = (() => {
    try {
      return buildUserTurn(
        threadId,
        prompt,
        attachments,
        tab,
      );
    } catch (e) {
      setToast(toastError(e));
      return null;
    }
  })();
  if (!built) return;
  const { clientId, input } = built;
  void sessionLog("info", threadId, "user-steer", `chars=${prompt.length}`);
  try {
    await invoke("turn_steer", {
      params: {
        threadId,
        clientUserMessageId: clientId,
        input,
        expectedTurnId: turnId,
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
              expectedTurnId: turnId,
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
  const active = activeSessionTab();
  const tid = threadId ?? active?.threadId;
  if (!tid) return;
  void sessionLog("info", tid, "user-stop");
  const tab = threadId ? findSessionTabByThread(tid) : active;
  // 线程有活跃目标：先清除目标切断服务端 auto-continuation（目标循环回合极快，
  // 回合中断可能追不上；清除目标后当前回合自然结束、不再自动续跑）
  const hasGoal = tab ? Boolean(tab.goalText) : false;
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
  // 绝不借用活动标签的 currentTurnId，避免跨线程误中断；
  // 无显式线程（停止按钮）时才取活动标签的 currentTurnId。
  let target = turnId;
  if (!target && threadId === undefined) target = active?.currentTurnId ?? null;
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
        // 仅在操作当前会话时同步活动标签的回合 id，避免切换会话后写错标签
        if (threadId === undefined && active) active.currentTurnId = target;
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
  const tab = activeSessionTab();
  const tid = tab?.threadId;
  if (!tid) return false; // 仅在线程存在时挂载（防御：调用方应保证有会话）
  try {
    await invoke("goal_set", { threadId: tid, objective: text });
    // 目标状态写活动标签（tab 是唯一事实源）
    if (tab) {
      tab.goalText = text;
      tab.goalStatus = "active";
    }
    setToast("已设置目标");
    return true;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}


export async function clearGoal(threadId?: string | null) {
  const active = activeSessionTab();
  const tid = threadId ?? active?.threadId;
  if (!tid) {
    if (active) {
      active.goalText = null;
      active.goalStatus = null;
      active.goalArmed = false;
    }
    return;
  }
  try {
    await invoke("goal_clear", { threadId: tid });
    // 仅当清除的是当前会话时才清空本地目标展示；
    // 切换会话时对旧线程的清除不应污染新会话的目标状态
    if (active?.threadId === tid) {
      active.goalText = null;
      active.goalStatus = null;
      active.goalArmed = false;
    }
  } catch (e) {
    setToast(toastError(e));
  }
}
