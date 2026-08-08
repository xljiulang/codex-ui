import { describe, expect, it } from "vitest";
import {
  permissionMode,
  toApprovalPolicy,
  toApprovalsReviewer,
  toSandbox,
  toSandboxPolicy,
} from "../permissions";

describe("权限模式映射", () => {
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

  it("未知模式回退到完全访问", () => {
    expect(permissionMode("unknown").id).toBe("full-access");
  });

  it("turn/start sandboxPolicy 映射", () => {
    expect(toSandboxPolicy("ask-for-approval", "C:\\workspace")).toEqual({
      type: "workspaceWrite",
      writableRoots: ["C:\\workspace"],
      networkAccess: false,
      excludeTmpdirEnvVar: false,
      excludeSlashTmp: false,
    });
    expect(toSandboxPolicy("help-me-approve", "C:\\workspace")).toEqual({
      type: "workspaceWrite",
      writableRoots: ["C:\\workspace"],
      networkAccess: false,
      excludeTmpdirEnvVar: false,
      excludeSlashTmp: false,
    });
    expect(toSandboxPolicy("full-access")).toEqual({
      type: "dangerFullAccess",
    });
  });
});
