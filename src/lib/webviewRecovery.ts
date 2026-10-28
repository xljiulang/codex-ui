import { invoke } from "@tauri-apps/api/core";

export interface WebviewRecoveryInfo {
  recovered: boolean;
  failureKind: string;
  exitCode: number | null;
  happenedAtMs: number;
}

/** 读取并消费 renderer 崩溃后的安全恢复标记；非 Tauri 环境返回 null。 */
export async function takeWebviewRecoveryInfo(): Promise<WebviewRecoveryInfo | null> {
  try {
    return await invoke<WebviewRecoveryInfo | null>(
      "take_webview_recovery_info",
    );
  } catch {
    return null;
  }
}
