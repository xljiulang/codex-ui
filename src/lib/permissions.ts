import type { PermissionId } from "./types";

export interface PermissionMode {
  id: PermissionId;
  label: string;
  desc: string;
  chip: string;
  /** 24x24 SVG path（fill 风格），菜单与输入框 chip 共用 */
  icon: string;
}

export const PERMISSION_MODES: PermissionMode[] = [
  {
    id: "ask-for-approval",
    label: "请求批准",
    desc: "编辑外部文件和使用互联网时始终询问",
    chip: "请求批准",
    icon: "M6.6 11h.2V6.5a1.5 1.5 0 0 1 3 0V10h.2V4.5a1.5 1.5 0 0 1 3 0V10h.2V6a1.5 1.5 0 0 1 3 0v7.6l-1.5 3.4A4 4 0 0 1 12 20H9.5a4 4 0 0 1-3.7-2.4L4 13.6a1.8 1.8 0 0 1 1.6-2.5z",
  },
  {
    id: "help-me-approve",
    label: "帮我批准",
    desc: "仅对检测到的风险操作请求批准",
    chip: "帮我批准",
    icon: "M12 2 4 5v6c0 5.55 3.84 10.74 8 12 4.16-1.26 8-6.45 8-12V5l-8-3zm0 2.2 6 2.3V11c0 4.4-2.8 8.5-6 9.9C8.8 19.5 6 15.4 6 11V6.5l6-2.3zm.8 4.5-4.2 4.2 1.4 1.4 2.8-2.8 3.6 3.6 1.4-1.4z",
  },
  {
    id: "full-access",
    label: "完全访问权限",
    desc: "可不受限制地访问互联网和您电脑上的任何文件",
    chip: "完全访问",
    icon: "M12 2 4 5v6c0 5.55 3.84 10.74 8 12 4.16-1.26 8-6.45 8-12V5l-8-3zm1 14h-2v-2h2v2zm0-4h-2V8h2v4z",
  },
];

export function permissionMode(id: PermissionId): PermissionMode {
  return PERMISSION_MODES.find((m) => m.id === id) ?? PERMISSION_MODES[2];
}

export function toApprovalPolicy(mode: PermissionId): "on-request" | "never" {
  switch (mode) {
    case "ask-for-approval":
      return "on-request";
    case "help-me-approve":
      return "on-request";
    default:
      return "never";
  }
}

export function toSandbox(
  mode: PermissionId,
): "workspace-write" | "danger-full-access" {
  switch (mode) {
    case "ask-for-approval":
      return "workspace-write";
    case "help-me-approve":
      return "workspace-write";
    default:
      return "danger-full-access";
  }
}

/** 官方映射：请求批准→用户评审；帮我批准→自动评审 */
export function toApprovalsReviewer(
  mode: PermissionId,
): "user" | "auto_review" | null {
  switch (mode) {
    case "ask-for-approval":
      return "user";
    case "help-me-approve":
      return "auto_review";
    default:
      return null;
  }
}

/** turn/start 的沙箱覆盖参数是 sandboxPolicy 对象 */
export function toSandboxPolicy(
  mode: PermissionId,
  workspaceRoot?: string,
): Record<string, unknown> {
  switch (mode) {
    case "ask-for-approval":
    case "help-me-approve":
      return {
        type: "workspaceWrite",
        writableRoots: workspaceRoot ? [workspaceRoot] : [],
        networkAccess: false,
        excludeTmpdirEnvVar: false,
        excludeSlashTmp: false,
      };
    default:
      return { type: "dangerFullAccess" };
  }
}
