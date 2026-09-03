import { describe, expect, it } from "vitest";
import type { PermissionId } from "../types";
import {
  PERMISSION_MODES,
  permissionMode,
  toApprovalPolicy,
  toApprovalsReviewer,
  toSandbox,
  toSandboxPolicy,
} from "../permissions";

describe("权限模式映射", () => {
  it("只读访问为权限模式首项", () => {
    expect(PERMISSION_MODES[0].id).toBe("read-only");
    expect(PERMISSION_MODES.map((m) => m.label)).toEqual([
      "只读访问",
      "请求批准",
      "帮我批准",
      "完全访问",
    ]);
    expect(PERMISSION_MODES[3].desc).toBe("不受限制地访问互联网和您电脑的任何文件");
    expect(PERMISSION_MODES[0].desc).toBe("文件只读，不会联网");
  });

  it("请求批准 → on-request + workspace-write + user 评审", () => {
    expect(toApprovalPolicy("ask-for-approval")).toBe("on-request");
    expect(toSandbox("ask-for-approval")).toBe("workspace-write");
    expect(toApprovalsReviewer("ask-for-approval")).toBe("user");
  });

  it("帮我批准 → on-request + workspace-write + auto_review 评审", () => {
    expect(toApprovalPolicy("help-me-approve")).toBe("on-request");
    expect(toSandbox("help-me-approve")).toBe("workspace-write");
    expect(toApprovalsReviewer("help-me-approve")).toBe("auto_review");
  });

  it("完全访问 → never + danger-full-access，无评审", () => {
    expect(toApprovalPolicy("full-access")).toBe("never");
    expect(toSandbox("full-access")).toBe("danger-full-access");
    expect(toApprovalsReviewer("full-access")).toBeNull();
  });

  it("只读访问 → never + read-only 沙箱，无评审", () => {
    expect(toApprovalPolicy("read-only")).toBe("never");
    expect(toSandbox("read-only")).toBe("read-only");
    expect(toApprovalsReviewer("read-only")).toBeNull();
  });

  it("未知模式回退到完全访问", () => {
    expect(permissionMode("unknown" as PermissionId).id).toBe("full-access");
  });

  it("turn/start sandboxPolicy 映射", () => {
    expect(toSandboxPolicy("ask-for-approval", "C:\\workspace")).toEqual({
      type: "workspaceWrite",
      writableRoots: [],
      networkAccess: true,
      excludeTmpdirEnvVar: false,
      excludeSlashTmp: false,
    });
    expect(toSandboxPolicy("help-me-approve", "C:\\workspace")).toEqual({
      type: "workspaceWrite",
      writableRoots: [],
      networkAccess: true,
      excludeTmpdirEnvVar: false,
      excludeSlashTmp: false,
    });
    expect(toSandboxPolicy("full-access")).toEqual({
      type: "dangerFullAccess",
    });
    expect(toSandboxPolicy("read-only")).toEqual({
      type: "readOnly",
      networkAccess: false,
    });
  });
});
