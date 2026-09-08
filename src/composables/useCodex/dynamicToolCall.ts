// useCodex 拆分模块：响应 app-server 发起的 `item/tool/call`（codexui 动态工具）管理调用。
import { invoke } from "@tauri-apps/api/core";
import { formatTokens } from "../../lib/format";
import {
  CODEXUI_DYNAMIC_NAMESPACE,
  CODEXUI_TOOL_ADD_SCHEDULED_TASK,
  CODEXUI_TOOL_COMPACT_CONTEXT,
  CODEXUI_TOOL_GET_USAGE,
  isDynamicToolDisabled,
} from "../../lib/dynamicTools";
import type { PendingInteraction } from "../../lib/types";
import { respondInteraction } from "./actions";
import { addScheduledTask } from "./scheduledTasks";
import { findSessionTabByThread } from "./sessionState";
import { store } from "./store";

interface DynamicToolPayload {
  requestId: number | string;
  method: string;
  params: Record<string, unknown>;
}

/** 按当前会话的用量数据拼一句话，供 `get_usage` 返回给 agent */
function buildUsageText(threadId?: string): string {
  const tab = threadId ? findSessionTabByThread(threadId) : undefined;
  const u = tab?.threadTokenUsage;
  if (!u) return "暂无用量数据（会话未打开或尚未收到用量事件）";
  const parts: string[] = [];
  if (u.window != null && u.window > 0) {
    const pct = Math.min(100, Math.round((u.contextUsed / u.window) * 100));
    parts.push(
      `上下文已用 ${formatTokens(u.contextUsed)}/${formatTokens(u.window)}（约 ${pct}%）`,
    );
  } else {
    parts.push(`已用 ${formatTokens(u.contextUsed)} 个上下文 token`);
  }
  const input = u.input;
  const output = u.output;
  if (typeof input === "number" || typeof output === "number") {
    parts.push(
      `会话累计输入 ${formatTokens(input ?? 0)} · 输出 ${formatTokens(output ?? 0)}`,
    );
  }
  return parts.join("；");
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** 处理 `item/tool/call`：读取/执行后用 `interaction_respond` 应答，不进交互气泡 */
export async function handleDynamicToolCall(p: DynamicToolPayload): Promise<void> {
  const interaction: PendingInteraction = {
    requestId: p.requestId,
    method: "item/tool/call",
    params: p.params,
    at: Date.now(),
  };
  const tool = typeof p.params?.tool === "string" ? p.params.tool : "";
  const threadId =
    typeof p.params?.threadId === "string" ? p.params.threadId : undefined;

  // 已被禁用的工具：即便历史/分叉线程仍暴露，也不再应答，返回禁用错误
  if (
    isDynamicToolDisabled(
      CODEXUI_DYNAMIC_NAMESPACE,
      tool,
      store.settings.dynamic_tools_disabled,
    )
  ) {
    await respondInteraction(interaction, {
      contentItems: [{ type: "inputText", text: "该动态工具已被禁用" }],
      success: false,
    });
    return;
  }

  switch (tool) {
    case CODEXUI_TOOL_GET_USAGE: {
      const text = buildUsageText(threadId);
      await respondInteraction(interaction, {
        contentItems: [{ type: "inputText", text }],
        success: true,
      });
      break;
    }
    case CODEXUI_TOOL_COMPACT_CONTEXT: {
      if (!threadId) {
        await respondInteraction(interaction, {
          contentItems: [{ type: "inputText", text: "压缩失败：缺少 threadId" }],
          success: false,
        });
        break;
      }
      try {
        await invoke("codex_rpc", {
          method: "thread/compact/start",
          params: { threadId },
        });
        await respondInteraction(interaction, {
          contentItems: [{ type: "inputText", text: "已请求压缩上下文" }],
          success: true,
        });
      } catch (e) {
        await respondInteraction(interaction, {
          contentItems: [{ type: "inputText", text: `压缩失败：${errText(e)}` }],
          success: false,
        });
      }
      break;
    }
    case CODEXUI_TOOL_ADD_SCHEDULED_TASK: {
      await handleAddScheduledTask(interaction, p.params);
      break;
    }
    default:
      await respondInteraction(interaction, {
        contentItems: [{ type: "inputText", text: `未知的 codexui 工具：${tool || "(空)"}` }],
        success: false,
      });
  }
}

/**
 * `add_scheduled_task`：agent 提供参数，直接创建（不弹前端确认框——任务可能
 * 经微信等无人值守渠道发起，弹窗会卡住回合；对用户的确认由 agent 在对话内完成）。
 * arguments 形态来自协议 DynamicToolCallParams.arguments（JSON object）。
 */
async function handleAddScheduledTask(
  interaction: PendingInteraction,
  params: Record<string, unknown>,
): Promise<void> {
  const raw = params.arguments;
  const args =
    raw && typeof raw === "object" && !Array.isArray(raw)
      ? (raw as Record<string, unknown>)
      : {};
  const name = typeof args.name === "string" ? args.name.trim() : "";
  const prompt = typeof args.prompt === "string" ? args.prompt.trim() : "";
  const cronExpr = typeof args.cron === "string" ? args.cron.trim() : "";
  const busyPolicy = args.busyPolicy === "defer" ? "defer" : "skip";
  if (!name || !prompt || !cronExpr) {
    await respondInteraction(interaction, {
      contentItems: [
        { type: "inputText", text: "创建失败：name、prompt、cron 均为必填" },
      ],
      success: false,
    });
    return;
  }

  try {
    const task = await addScheduledTask({
      name,
      prompt,
      cron: cronExpr,
      threadId: params.threadId as string,
      busyPolicy,
    });
    const next = task.nextRun
      ? new Date(task.nextRun * 1000).toLocaleString()
      : "（无下一次触发）";
    await respondInteraction(interaction, {
      contentItems: [
        {
          type: "inputText",
          text: `定时任务「${task.name}」已创建，下次触发：${next}。可在 设置 → 定时任务 中管理。`,
        },
      ],
      success: true,
    });
  } catch (e) {
    await respondInteraction(interaction, {
      contentItems: [{ type: "inputText", text: `创建失败：${errText(e)}` }],
      success: false,
    });
  }
}
