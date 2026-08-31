import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { SessionGroup } from "../../lib/sessionGroup";
import type { ThreadSummary } from "../../lib/types";

vi.mock("../useCodex", () => ({
  deleteThread: vi.fn(() => Promise.resolve()),
  isThreadOpen: vi.fn(() => false),
  renameThread: vi.fn(() => Promise.resolve()),
  setToast: vi.fn(),
  threadTitle: vi.fn((t: ThreadSummary) => t.name ?? "未命名"),
}));

import {
  deleteThread,
  isThreadOpen,
  renameThread,
  setToast,
  threadTitle,
} from "../useCodex";
import { useSessionDialogs } from "../useSessionDialogs";

const mockedDelete = vi.mocked(deleteThread);
const mockedIsOpen = vi.mocked(isThreadOpen);
const mockedRename = vi.mocked(renameThread);
const mockedToast = vi.mocked(setToast);
const mockedTitle = vi.mocked(threadTitle);

function thread(overrides: Partial<ThreadSummary> = {}): ThreadSummary {
  return {
    id: "t1",
    name: "会话 A",
    preview: null,
    createdAt: 0,
    ...overrides,
  };
}

function setup() {
  return useSessionDialogs({ confirmEl: ref<HTMLElement | null>(null) });
}

describe("useSessionDialogs 重命名", () => {
  beforeEach(() => {
    mockedRename.mockClear();
  });

  it("startRename 填充编辑状态", () => {
    const d = setup();
    d.startRename(thread());
    expect(d.editingId.value).toBe("t1");
    expect(d.editName.value).toBe("会话 A");
    expect(mockedTitle).toHaveBeenCalled();
  });

  it("saveRename 名称变化时调 renameThread，未变/空名不调用", () => {
    const d = setup();
    d.startRename(thread());
    d.editName.value = "新名称";
    d.saveRename(thread());
    expect(mockedRename).toHaveBeenCalledWith("t1", "新名称");
    expect(d.editingId.value).toBeNull();

    d.startRename(thread());
    d.editName.value = "会话 A"; // 与 threadTitle 相同
    d.saveRename(thread());
    expect(mockedRename).toHaveBeenCalledTimes(1);
  });
});

describe("useSessionDialogs 删除确认", () => {
  beforeEach(() => {
    mockedDelete.mockClear();
    mockedToast.mockClear();
    mockedIsOpen.mockReset();
  });

  it("单条会话：标题/文案正确，确认后删除该会话", async () => {
    const d = setup();
    d.askDelete(thread());
    expect(d.confirmTitle.value).toBe("删除会话");
    expect(d.confirmMessage.value).toContain("会话 A");
    await d.doDelete();
    expect(mockedDelete).toHaveBeenCalledWith("t1");
    expect(d.confirmDelete.value).toBeNull();
  });

  it("整组删除：跳过已打开会话并提示计数", async () => {
    mockedIsOpen.mockImplementation((id) => id === "t1");
    const group: SessionGroup = {
      key: "D:\\repo",
      label: "repo",
      path: "D:\\repo",
      threads: [thread({ id: "t1" }), thread({ id: "t2", name: "会话 B" })],
    };
    const d = setup();
    d.askDeleteGroup(group);
    expect(d.confirmTitle.value).toBe("删除所有会话");
    expect(d.confirmMessage.value).toContain("1 个已打开将保留");
    await d.doDelete();
    expect(mockedDelete).toHaveBeenCalledTimes(1);
    expect(mockedDelete).toHaveBeenCalledWith("t2");
    expect(mockedToast).toHaveBeenCalledWith("已跳过 1 个已打开的会话");
  });

  it("整组全关闭时删除全部且不提示跳过", async () => {
    const group: SessionGroup = {
      key: "D:\\repo",
      label: "repo",
      path: "D:\\repo",
      threads: [thread({ id: "t1" }), thread({ id: "t2", name: "会话 B" })],
    };
    const d = setup();
    d.askDeleteGroup(group);
    expect(d.confirmMessage.value).toContain("共 2 个");
    await d.doDelete();
    expect(mockedDelete).toHaveBeenCalledTimes(2);
    expect(mockedToast).not.toHaveBeenCalled();
  });

  it("cancelDelete 清除确认状态", () => {
    const d = setup();
    d.askDelete(thread());
    d.cancelDelete();
    expect(d.confirmDelete.value).toBeNull();
  });
});
