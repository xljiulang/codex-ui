import { describe, expect, it, beforeEach, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import {
  applyConfigProfile,
  createConfigProfile,
  deleteConfigProfile,
  listConfigProfiles,
  openConfigProfile,
} from "../useConfigProfiles";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockedInvoke = vi.mocked(invoke);

const mockSetToast = vi.hoisted(() => vi.fn());
const mockAskConfirm = vi.hoisted(() => vi.fn());
const mockOpenPath = vi.hoisted(() => vi.fn());

vi.mock("../useCodex", () => ({
  setToast: mockSetToast,
  askConfirm: mockAskConfirm,
}));

vi.mock("../usePathOpen", () => ({
  openPathInAppOrReveal: mockOpenPath,
}));

describe("useConfigProfiles", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockSetToast.mockReset();
    mockAskConfirm.mockReset();
    mockOpenPath.mockReset();
  });

  it("listConfigProfiles 读取配置快照名列表", async () => {
    mockedInvoke.mockResolvedValue(["a", "b"]);
    expect(await listConfigProfiles()).toEqual(["a", "b"]);
    expect(mockedInvoke).toHaveBeenCalledWith("config_profiles_list");
  });

  it("createConfigProfile 调用 config_profiles_save（同名覆盖由后端负责）", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    await createConfigProfile("dev");
    expect(mockedInvoke).toHaveBeenCalledWith("config_profiles_save", {
      name: "dev",
    });
    // 新建仅快照当前状态，不自动激活/应用/热重载
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd]) => cmd === "config_profiles_apply",
      ),
    ).toBe(false);
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd]) => cmd === "codex_rpc",
      ),
    ).toBe(false);
  });

  it("applyConfigProfile 覆盖文件、热重载并 toast 提示（无激活记录）", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    await applyConfigProfile("dev");

    // config_profiles_apply
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd]) => cmd === "config_profiles_apply",
      ),
    ).toBe(true);
    // 触发 config/batchWrite 热重载
    const batch = mockedInvoke.mock.calls.find(
      ([, args]) =>
        (args as { method?: string } | undefined)?.method === "config/batchWrite",
    );
    expect(batch).toBeTruthy();
    expect(
      (batch![1] as { params: { reloadUserConfig: boolean } }).params
        .reloadUserConfig,
    ).toBe(true);
    // toast 提示（不再记录/切换激活项）
    expect(mockSetToast).toHaveBeenCalledWith(
      '已应用配置快照「dev」，重启 codex-ui 后生效',
    );
  });

  it("applyConfigProfile 在空编辑失败时回退为当前 model 无变更写回", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      const a = args as { method?: string } | undefined;
      if (cmd === "codex_rpc" && a?.method === "config/batchWrite") {
        const edits = (a as { params?: { edits?: unknown[] } }).params?.edits;
        if (edits?.length === 0) throw new Error("empty edits rejected");
        return {};
      }
      if (cmd === "codex_rpc" && a?.method === "config/read") {
        return { config: { model: "gpt" } };
      }
      return undefined;
    });
    await applyConfigProfile("dev");
    const batch = mockedInvoke.mock.calls.find(([, args]) => {
      const a = args as
        | { method?: string; params?: { edits?: unknown[] } }
        | undefined;
      return (
        a?.method === "config/batchWrite" && (a.params?.edits?.length ?? 0) > 0
      );
    });
    const params = (batch![1] as { params: Record<string, unknown> }).params;
    expect(params.reloadUserConfig).toBe(true);
    expect((params.edits as { value: string }[])[0].value).toBe("gpt");
  });

  it("deleteConfigProfile 确认后删除并 toast", async () => {
    mockAskConfirm.mockResolvedValue(true);
    mockedInvoke.mockResolvedValue(undefined);

    await deleteConfigProfile("dev");

    expect(mockAskConfirm).toHaveBeenCalledOnce();
    expect(mockedInvoke).toHaveBeenCalledWith("config_profiles_delete", {
      name: "dev",
    });
    expect(mockSetToast).toHaveBeenCalledWith('已删除配置「dev」');
  });

  it("deleteConfigProfile 取消确认时不删除也不提示", async () => {
    mockAskConfirm.mockResolvedValue(false);
    mockedInvoke.mockResolvedValue(undefined);

    await deleteConfigProfile("dev");

    expect(mockedInvoke).not.toHaveBeenCalledWith("config_profiles_delete", {
      name: "dev",
    });
    expect(mockSetToast).not.toHaveBeenCalled();
  });

  it("openConfigProfile 拉取快照文件并在编辑器逐个打开", async () => {
    mockedInvoke.mockResolvedValue([
      "/home/.codex/codex-ui/dev/config.toml",
      "/home/.codex/codex-ui/dev/custom.json",
    ]);

    await openConfigProfile("dev");

    expect(mockedInvoke).toHaveBeenCalledWith("config_profiles_open", {
      name: "dev",
    });
    expect(mockOpenPath).toHaveBeenCalledTimes(2);
    expect(mockOpenPath).toHaveBeenNthCalledWith(
      1,
      "/home/.codex/codex-ui/dev/config.toml",
    );
    expect(mockOpenPath).toHaveBeenNthCalledWith(
      2,
      "/home/.codex/codex-ui/dev/custom.json",
    );
  });
});
