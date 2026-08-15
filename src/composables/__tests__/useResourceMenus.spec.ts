import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { CtxItem } from "../useActionMenu";
import type { FsEntry } from "../../lib/sessionFs";

vi.mock("../useEditorTabs", () => ({
  isFileTabOpen: vi.fn(() => false),
  openTerminalTab: vi.fn(),
}));
vi.mock("../useSessionFs", () => ({
  addAsAttachment: vi.fn(),
  copyEntry: vi.fn(),
  pasteAvailable: vi.fn(),
  pasteInto: vi.fn(),
  revealInExplorer: vi.fn(),
  textFileMenuIcon: vi.fn(),
}));
vi.mock("../useCodex", () => ({
  workspace: ref("D:\\repo"),
}));

import { pasteAvailable } from "../useSessionFs";
import { useResourceMenus } from "../useResourceMenus";

const mockedPasteAvailable = vi.mocked(pasteAvailable);

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

function mockEvent() {
  return {
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  } as unknown as MouseEvent;
}

function setup(overrides: { paste?: boolean } = {}) {
  const captured: CtxItem[][] = [];
  const openCtx = vi.fn((_e: MouseEvent, items: CtxItem[]) => {
    captured.push(items);
  });
  const menus = useResourceMenus({
    openCtx,
    rootEntry: ref<FsEntry | null>(entry({ isDir: true, name: "repo" })),
    hasActiveSessionTab: ref(false),
    onCreateFolder: vi.fn(),
    onCreateTextFile: vi.fn(),
    requestOpen: vi.fn(),
    startRename: vi.fn(),
    askDelete: vi.fn(),
    openProps: vi.fn(),
  });
  mockedPasteAvailable.mockResolvedValue(overrides.paste ?? false);
  return { captured, openCtx, menus };
}

describe("useResourceMenus", () => {
  beforeEach(() => {
    mockedPasteAvailable.mockReset();
  });

  it("文件菜单：打开/复制/属性/删除/重命名/资源管理器（无附件入口）", () => {
    const { captured, menus } = setup();
    menus.openFileMenu(entry(), mockEvent());
    expect(captured[0].map((i) => i.label)).toEqual([
      "打开",
      "复制",
      "属性",
      "删除",
      "重命名",
      "在资源管理器中打开",
    ]);
  });

  it("目录菜单：新建/复制/删除/重命名/终端/资源管理器，粘贴按可用性出现", async () => {
    const { captured, menus } = setup({ paste: true });
    await menus.openDirMenu(entry({ isDir: true, name: "src" }), mockEvent());
    expect(captured[0].map((i) => i.label)).toEqual([
      "新建文本文件",
      "新建文件夹",
      "复制",
      "粘贴",
      "删除",
      "重命名",
      "在此打开终端",
      "在资源管理器中打开",
    ]);
  });

  it("根菜单：新建/终端/资源管理器", async () => {
    const { captured, menus } = setup();
    await menus.openRootMenu(mockEvent());
    expect(captured[0].map((i) => i.label)).toEqual([
      "新建文本文件",
      "新建文件夹",
      "在此打开终端",
      "在资源管理器中打开",
    ]);
  });

  it("openEntryMenu 按目录/文件分发", async () => {
    const { captured, menus } = setup();
    await menus.openEntryMenu(entry({ isDir: true }), mockEvent());
    expect(captured[0][0].label).toBe("新建文本文件");
    menus.openEntryMenu(entry(), mockEvent());
    expect(captured[1][0].label).toBe("打开");
  });
});
