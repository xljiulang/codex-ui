import { invoke } from "@tauri-apps/api/core";

export type SessionLogLevel = "info" | "warn" | "error";

/**
 * 追加一条会话诊断日志（fire-and-forget）：失败静默，不影响主流程。
 * 只允许传诊断所需的安全字段（threadId/事件名/简短 detail），
 * 严禁传入提示词、文件内容、工具输出等敏感数据。
 */
export async function sessionLog(
  level: SessionLogLevel,
  threadId: string | null | undefined,
  event: string,
  detail?: string,
): Promise<void> {
  try {
    await invoke("session_log", {
      level,
      threadId: threadId ?? null,
      event,
      detail: detail ?? null,
    });
  } catch {
    // 日志失败静默
  }
}
