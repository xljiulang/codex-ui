import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockAskConfirm = vi.hoisted(() => vi.fn());
const mockSetToast = vi.hoisted(() => vi.fn());
const mockOpenPath = vi.hoisted(() => vi.fn());

vi.mock("../useCodex", () => ({
  askConfirm: mockAskConfirm,
  setToast: mockSetToast,
}));

vi.mock("../usePathOpen", () => ({
  openPathInAppOrReveal: mockOpenPath,
}));

import {
  applyModelSnapshot,
  createModelSnapshot,
  deleteModelSnapshot,
  listModelSnapshots,
  openModelSnapshot,
} from "../useModelSnapshots";

const mockedInvoke = vi.mocked(invoke);

describe("useModelSnapshots", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockAskConfirm.mockReset();
    mockSetToast.mockReset();
    mockOpenPath.mockReset();
  });

  it("listModelSnapshots 调用后端列表命令", async () => {
    mockedInvoke.mockResolvedValue(["a", "b"]);
    await expect(listModelSnapshots()).resolves.toEqual(["a", "b"]);
    expect(mockedInvoke).toHaveBeenCalledWith("model_snapshots_list");
  });

  it("createModelSnapshot 调用保存命令并提示含密钥", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    await createModelSnapshot(" dev ");
    expect(mockedInvoke).toHaveBeenCalledWith("model_snapshots_save", {
      name: "dev",
    });
    expect(mockSetToast).toHaveBeenCalledWith(
      expect.stringContaining("可能含 API Key"),
    );
  });

  it("applyModelSnapshot 调用还原命令并提示重启", async () => {
    mockedInvoke.mockResolvedValue({
      modelCatalogJson: "model_catalog.json",
      modelCatalogPath: "C:\\x\\.codex\\model_catalog.json",
    });
    await applyModelSnapshot("dev");
    expect(mockedInvoke).toHaveBeenCalledWith("model_snapshots_apply", {
      name: "dev",
    });
    expect(mockSetToast).toHaveBeenCalledWith(
      "已应用模型快照「dev」，重启 codex-ui 后生效",
    );
  });

  it("deleteModelSnapshot 确认后删除", async () => {
    mockAskConfirm.mockResolvedValue(true);
    mockedInvoke.mockResolvedValue(undefined);
    await deleteModelSnapshot("dev");
    expect(mockAskConfirm).toHaveBeenCalledWith(
      expect.objectContaining({ title: "删除模型快照" }),
    );
    expect(mockedInvoke).toHaveBeenCalledWith("model_snapshots_delete", {
      name: "dev",
    });
  });

  it("deleteModelSnapshot 取消时不删除", async () => {
    mockAskConfirm.mockResolvedValue(false);
    await deleteModelSnapshot("dev");
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("openModelSnapshot 打开 JSON 文件", async () => {
    mockedInvoke.mockResolvedValue("C:\\x\\.codex\\codex-ui\\dev.json");
    await openModelSnapshot("dev");
    expect(mockedInvoke).toHaveBeenCalledWith("model_snapshots_open", {
      name: "dev",
    });
    expect(mockOpenPath).toHaveBeenCalledWith(
      "C:\\x\\.codex\\codex-ui\\dev.json",
    );
  });
});
