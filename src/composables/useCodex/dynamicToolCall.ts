// useCodex 拆分模块：响应 app-server 发起的 `item/tool/call`（codexui 动态工具）管理调用。
import { invoke } from "@tauri-apps/api/core";
import { formatTokens } from "../../lib/format";
import {
  CODEXUI_DYNAMIC_NAMESPACE,
  CODEXUI_TOOL_ADD_SCHEDULED_TASK,
  CODEXUI_TOOL_COMPACT_CONTEXT,
  CODEXUI_TOOL_GET_USAGE,
  CODEXUI_TOOL_INDEX_DOCS,
  CODEXUI_TOOL_SEARCH_DOCS,
  isDynamicToolEnabled,
} from "../../lib/dynamicTools";
import type { PendingInteraction } from "../../lib/types";
import { respondInteraction } from "./actions";
import { resolveSessionWorkspace } from "./items";
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

  // 生效开关为关闭的工具：即便历史/分叉线程仍暴露，也不再应答，返回未开启错误
  if (
    !isDynamicToolEnabled(
      CODEXUI_DYNAMIC_NAMESPACE,
      tool,
      store.settings.dynamic_tools_state,
    )
  ) {
    await respondInteraction(interaction, {
      contentItems: [{ type: "inputText", text: "该动态工具未开启" }],
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
    case CODEXUI_TOOL_SEARCH_DOCS: {
      await handleSearchDocs(interaction, p.params);
      break;
    }
    case CODEXUI_TOOL_INDEX_DOCS: {
      await handleIndexDocs(interaction, p.params);
      break;
    }
    default:
      await respondInteraction(interaction, {
        contentItems: [{ type: "inputText", text: `未知的 codexui 工具：${tool || "(空)"}` }],
        success: false,
      });
  }
}

/** 知识库检索命中的切块（Rust `ChunkHit` 的字段形状） */
interface KnowledgeHit {
  doc_path: string;
  title_path: string;
  text: string;
  score: number;
}

/** 动态工具 arguments（协议为 JSON object，做宽松解析） */
function toolArgs(params: Record<string, unknown>): Record<string, unknown> {
  const raw = params.arguments;
  return raw && typeof raw === "object" && !Array.isArray(raw)
    ? (raw as Record<string, unknown>)
    : {};
}

/** 工具调用所属会话的工作目录（知识库与工作目录一对一） */
function workspaceOf(params: Record<string, unknown>): string {
  const threadId =
    typeof params.threadId === "string" ? params.threadId : undefined;
  const tab = threadId ? findSessionTabByThread(threadId) : undefined;
  return resolveSessionWorkspace(tab);
}

function fileNameOf(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** 单个片段正文的展示上限（避免把整块长文塞进对话上下文） */
const HIT_TEXT_LIMIT = 700;

async function handleSearchDocs(
  interaction: PendingInteraction,
  params: Record<string, unknown>,
): Promise<void> {
  const args = toolArgs(params);
  const query = typeof args.query === "string" ? args.query.trim() : "";
  const cwd = workspaceOf(params);
  if (!query) {
    await respondInteraction(interaction, {
      contentItems: [{ type: "inputText", text: "检索失败：缺少 query 参数" }],
      success: false,
    });
    return;
  }
  if (!cwd) {
    await respondInteraction(interaction, {
      contentItems: [
        {
          type: "inputText",
          text: "检索失败：当前会话没有工作目录，无法确定知识库（请为会话选择目录后重试）",
        },
      ],
      success: false,
    });
    return;
  }
  const topK = typeof args.topK === "number" ? args.topK : undefined;
  try {
    const raw = await invoke<KnowledgeHit[] | null | undefined>("knowledge_search", {
      cwd,
      query,
      topK,
    });
    // Rust 正常返回数组；防御异常响应，避免把 undefined 当成"命中 0 条"以外的崩溃
    const hits = Array.isArray(raw) ? raw : [];
    if (!hits.length) {
      await respondInteraction(interaction, {
        contentItems: [
          {
            type: "inputText",
            text:
              `知识库中没有匹配「${query}」的内容（工作目录：${cwd}）。\n` +
              "可能尚未建库：可先用 index_docs 建立/更新知识库；也可换用文档里的原始术语（故障码、型号、部件名）再检索。",
          },
        ],
        success: true,
      });
      return;
    }
    const blocks = hits.map((hit, i) => {
      const section = hit.title_path || "（无章节）";
      const body =
        hit.text.length > HIT_TEXT_LIMIT
          ? `${hit.text.slice(0, HIT_TEXT_LIMIT)}…`
          : hit.text;
      return `[${i + 1}] ${fileNameOf(hit.doc_path)} › ${section}（匹配 ${hit.score.toFixed(2)}）\n${body}`;
    });
    const text =
      `【知识库检索结果】工作目录：${cwd}｜检索词：${query}｜共 ${hits.length} 条\n\n` +
      blocks.join("\n\n") +
      "\n\n回答时请标注来源文件名与章节；若片段不足以回答，请说明不确定或改为追问。";
    await respondInteraction(interaction, {
      contentItems: [{ type: "inputText", text }],
      success: true,
    });
  } catch (e) {
    await respondInteraction(interaction, {
      contentItems: [{ type: "inputText", text: `检索失败：${errText(e)}` }],
      success: false,
    });
  }
}

async function handleIndexDocs(
  interaction: PendingInteraction,
  params: Record<string, unknown>,
): Promise<void> {
  const args = toolArgs(params);
  const cwd = workspaceOf(params);
  if (!cwd) {
    await respondInteraction(interaction, {
      contentItems: [
        {
          type: "inputText",
          text: "建库失败：当前会话没有工作目录，无法确定知识库归属（请为会话选择目录后重试）",
        },
      ],
      success: false,
    });
    return;
  }
  const paths = Array.isArray(args.paths)
    ? args.paths.filter((p): p is string => typeof p === "string" && !!p.trim())
    : undefined;
  const full = typeof args.full === "boolean" ? args.full : undefined;
  try {
    const summary = await invoke<{ text?: string }>("knowledge_index_start", {
      cwd,
      paths,
      full,
    });
    await respondInteraction(interaction, {
      contentItems: [
        { type: "inputText", text: summary?.text || "建库请求已提交" },
      ],
      success: true,
    });
  } catch (e) {
    await respondInteraction(interaction, {
      contentItems: [{ type: "inputText", text: `建库失败：${errText(e)}` }],
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
