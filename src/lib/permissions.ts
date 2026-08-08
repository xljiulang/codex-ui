export interface PermissionMode {
  id: string;
  label: string;
  desc: string;
  chip: string;
  icon: "hand" | "shield" | "warn";
}

export const PERMISSION_MODES: PermissionMode[] = [
  {
    id: "ask-for-approval",
    label: "请求批准",
    desc: "编辑外部文件和使用互联网时始终询问",
    chip: "请求批准",
    icon: "hand",
  },
  {
    id: "help-me-approve",
    label: "帮我批准",
    desc: "仅对检测到的风险操作请求批准",
    chip: "帮我批准",
    icon: "shield",
  },
  {
    id: "full-access",
    label: "完全访问权限",
    desc: "可不受限制地访问互联网和您电脑上的任何文件",
    chip: "完全访问",
    icon: "warn",
  },
];

export function permissionMode(id: string): PermissionMode {
  return PERMISSION_MODES.find((m) => m.id === id) ?? PERMISSION_MODES[2];
}

export function toApprovalPolicy(mode: string): string {
  switch (mode) {
    case "ask-for-approval":
      return "on-request";
    case "help-me-approve":
      return "on-request";
    default:
      return "never";
  }
}

export function toSandbox(mode: string): string {
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
export function toApprovalsReviewer(mode: string): string | null {
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
  mode: string,
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
