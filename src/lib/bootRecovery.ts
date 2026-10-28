import type { WebviewRecoveryInfo } from "./webviewRecovery";

export const WEBVIEW_RECOVERY_NOTICE =
  "界面渲染进程异常，已使用安全模式恢复。请从右侧历史重新打开会话。";

export interface FinishStartupOptions {
  recovery: WebviewRecoveryInfo | null;
  restoreLastSession: () => Promise<boolean>;
  openSettingsTab: () => void;
  notifyRecovery: (message: string) => void;
}

/** 启动初始化完成后的唯一恢复决策：恢复态绝不自动重开上次重型会话。 */
export async function finishStartup(
  options: FinishStartupOptions,
): Promise<void> {
  if (options.recovery?.recovered) {
    options.openSettingsTab();
    options.notifyRecovery(WEBVIEW_RECOVERY_NOTICE);
    return;
  }
  if (!(await options.restoreLastSession())) options.openSettingsTab();
}
