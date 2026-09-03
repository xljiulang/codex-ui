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
    id: "read-only",
    label: "只读访问",
    desc: "文件只读，不会联网",
    chip: "只读",
    icon: "M12 2a5 5 0 0 0-5 5v3H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8a2 2 0 0 0-2-2h-1V7a5 5 0 0 0-5-5zm-3 8V7a3 3 0 0 1 6 0v3H9zm3 4a1.5 1.5 0 0 1 1.5 1.5V16a1.5 1.5 0 0 1-3 0v-.5A1.5 1.5 0 0 1 12 14z",
  },
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
    label: "完全访问",
    desc: "不受限制地访问互联网和您电脑的任何文件",
    chip: "完全访问",
    icon: "M12 2 4 5v6c0 5.55 3.84 10.74 8 12 4.16-1.26 8-6.45 8-12V5l-8-3zm1 14h-2v-2h2v2zm0-4h-2V8h2v4z",
  },
];

export function permissionMode(id: PermissionId): PermissionMode {
  return (
    PERMISSION_MODES.find((m) => m.id === id) ??
    PERMISSION_MODES.find((m) => m.id === "full-access") ??
    PERMISSION_MODES[PERMISSION_MODES.length - 1]
  );
}

export function toApprovalPolicy(mode: PermissionId): "on-request" | "never" {
  switch (mode) {
    case "ask-for-approval":
      return "on-request";
    case "help-me-approve":
      return "on-request";
    case "read-only":
      return "never";
    default:
      return "never";
  }
}

export function toSandbox(
  mode: PermissionId,
): "read-only" | "workspace-write" | "danger-full-access" {
  switch (mode) {
    case "read-only":
      return "read-only";
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
  _workspace?: string,
): Record<string, unknown> {
  switch (mode) {
    case "read-only":
      return { type: "readOnly", networkAccess: false };
    case "ask-for-approval":
    case "help-me-approve":
      return {
        type: "workspaceWrite",
        writableRoots: [],
        networkAccess: true,
        excludeTmpdirEnvVar: false,
        excludeSlashTmp: false,
      };
    default:
      return { type: "dangerFullAccess" };
  }
}
