import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FsEntry } from "../../lib/sessionFs";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../useSessionFs", () => ({
  deleteEntry: vi.fn(() => Promise.resolve()),
  renameEntry: vi.fn(() => Promise.resolve()),
}));
vi.mock("../useCodex", () => ({
  setToast: vi.fn(),
  toastError: vi.fn((e: unknown) => String(e)),
  workspace: { value: "D:\\repo" },
}));

import { invoke } from "@tauri-apps/api/core";
import { deleteEntry, renameEntry } from "../useSessionFs";
import { setToast } from "../useCodex";
import { useResourceDialogs } from "../useResourceDialogs";

const mockedInvoke = vi.mocked(invoke);
const mockedRename = vi.mocked(renameEntry);
const mockedDelete = vi.mocked(deleteEntry);
const mockedToast = vi.mocked(setToast);

function entry(overrides: Partial<FsEntry> = {}): FsEntry {
  return {
    name: "a.txt",
    path: "D:\\repo\\a.txt",
    relPath: "a.txt",
    isDir: false,
    size: 10,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: null,
    ...overrides,
  };
}

describe("useResourceDialogs 重命名", () => {
  beforeEach(() => {
    mockedRename.mockClear();
  });

  it("startRename 填充编辑状态并聚焦输入框", () => {
    const d = useResourceDialogs();
    d.startRename(entry());
    expect(d.editingPath.value).toBe("D:\\repo\\a.txt");
    expect(d.editName.value).toBe("a.txt");
  });

  it("saveRename 名称变化时调 renameEntry，未变不调用", () => {
    const d = useResourceDialogs();
    d.startRename(entry());
    d.editName.value = "b.txt";
    d.saveRename(entry());
    expect(mockedRename).toHaveBeenCalledWith("D:\\repo\\a.txt", "b.txt");
    expect(d.editingPath.value).toBeNull();

    // 名称与 entry 相同（startRename 预填）→ 不调用
    d.startRename(entry());
    d.saveRename(entry());
    expect(mockedRename).toHaveBeenCalledTimes(1);
  });

  it("cancelRename 清除编辑状态", () => {
    const d = useResourceDialogs();
    d.startRename(entry());
    d.cancelRename();
    expect(d.editingPath.value).toBeNull();
  });
});

describe("useResourceDialogs 删除与属性", () => {
  beforeEach(() => {
    mockedDelete.mockClear();
    mockedInvoke.mockReset();
    mockedToast.mockClear();
  });

  it("doDelete 调 deleteEntry 并清除确认状态", () => {
    const d = useResourceDialogs();
    d.askDelete(entry());
    d.doDelete();
    expect(mockedDelete).toHaveBeenCalledWith("D:\\repo\\a.txt");
    expect(d.confirmDelete.value).toBeNull();
  });

  it("deleteLabel 按目录/文件给出不同文案", () => {
    const d = useResourceDialogs();
    d.askDelete(entry());
    expect(d.deleteLabel.value).toContain("删除文件");
    d.askDelete(entry({ isDir: true, name: "src" }));
    expect(d.deleteLabel.value).toContain("删除文件夹");
  });

  it("openProps 成功后用元信息替换条目，失败时 toast", async () => {
    const meta = entry({ name: "a.txt", size: 99 });
    mockedInvoke.mockResolvedValueOnce(meta);
    const d = useResourceDialogs();
    await d.openProps(entry());
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      workspace: "D:\\repo",
      path: "D:\\repo\\a.txt",
    });
    expect(d.propsEntry.value).toStrictEqual(meta);
    expect(d.propsLoading.value).toBe(false);

    mockedInvoke.mockRejectedValueOnce(new Error("boom"));
    await d.openProps(entry());
    expect(mockedToast).toHaveBeenCalled();
  });

  it("formatAbsolute 对 0 返回 -", () => {
    const d = useResourceDialogs();
    expect(d.formatAbsolute(0)).toBe("-");
    expect(d.formatAbsolute(1700000000000)).toContain("/");
  });
});
