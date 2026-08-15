import type { Ref } from "vue";
import type { CtxItem } from "./useActionMenu";
import { openTerminalTab, isFileTabOpen } from "./useEditorTabs";
import {
  addAsAttachment,
  copyEntry,
  pasteAvailable,
  pasteInto,
  revealInExplorer,
  textFileMenuIcon,
} from "./useSessionFs";
import { workspace } from "./useCodex";
import type { FsEntry } from "../lib/sessionFs";
import {
  ICON_AT,
  ICON_COPY,
  ICON_DELETE,
  ICON_INFO,
  ICON_OPEN,
  ICON_PASTE,
  ICON_PLUS,
  ICON_RENAME,
  ICON_REVEAL,
  ICON_TERMINAL,
} from "../lib/icons";

/** 资源树右键菜单构建（根/目录/文件统一分发） */
export function useResourceMenus(options: {
  openCtx: (e: MouseEvent, items: CtxItem[]) => void;
  rootEntry: Ref<FsEntry | null>;
  hasActiveSessionTab: Ref<boolean>;
  onCreateFolder: (parent: FsEntry) => void;
  onCreateTextFile: (parent: FsEntry) => void;
  requestOpen: (entry: FsEntry) => void;
  startRename: (entry: FsEntry) => void;
  askDelete: (entry: FsEntry) => void;
  openProps: (entry: FsEntry) => void;
}) {
  async function openRootMenu(e: MouseEvent) {
    const root = options.rootEntry.value;
    if (!root) return;
    const canPaste = await pasteAvailable();
    options.openCtx(e, [
      {
        label: "新建文本文件",
        icon: ICON_PLUS,
        img: textFileMenuIcon(),
        action: () => options.onCreateTextFile(root),
      },
      {
        label: "新建文件夹",
        icon: ICON_PLUS,
        action: () => options.onCreateFolder(root),
      },
      ...(canPaste
        ? [
            {
              label: "粘贴",
              icon: ICON_PASTE,
              action: () => void pasteInto(root.path),
            },
          ]
        : []),
      {
        label: "在此打开终端",
        icon: ICON_TERMINAL,
        action: () => void openTerminalTab(root.path),
      },
      {
        label: "在资源管理器中打开",
        icon: ICON_REVEAL,
        action: () => revealInExplorer(root.path),
      },
    ]);
  }

  async function openDirMenu(entry: FsEntry, e: MouseEvent) {
    const canPaste = await pasteAvailable();
    options.openCtx(e, [
      {
        label: "新建文本文件",
        icon: ICON_PLUS,
        img: textFileMenuIcon(),
        action: () => options.onCreateTextFile(entry),
      },
      {
        label: "新建文件夹",
        icon: ICON_PLUS,
        action: () => options.onCreateFolder(entry),
      },
      { label: "复制", icon: ICON_COPY, action: () => copyEntry(entry) },
      ...(canPaste
        ? [
            {
              label: "粘贴",
              icon: ICON_PASTE,
              action: () => void pasteInto(entry.path),
            },
          ]
        : []),
      {
        label: "删除",
        icon: ICON_DELETE,
        danger: true,
        action: () => options.askDelete(entry),
      },
      {
        label: "重命名",
        icon: ICON_RENAME,
        action: () => options.startRename(entry),
      },
      ...(options.hasActiveSessionTab.value
        ? [
            {
              label: "添加为会话附件",
              icon: ICON_AT,
              action: () => addAsAttachment(entry),
            },
          ]
        : []),
      {
        label: "在此打开终端",
        icon: ICON_TERMINAL,
        action: () => void openTerminalTab(entry.path),
      },
      {
        label: "在资源管理器中打开",
        icon: ICON_REVEAL,
        action: () => revealInExplorer(entry.path),
      },
    ]);
  }

  function openFileMenu(entry: FsEntry, e: MouseEvent) {
    const items: CtxItem[] = [
      ...(isFileTabOpen(workspace.value, entry.path)
        ? []
        : [
            {
              label: "打开",
              icon: ICON_OPEN,
              action: () => options.requestOpen(entry),
            },
          ]),
      { label: "复制", icon: ICON_COPY, action: () => copyEntry(entry) },
      {
        label: "属性",
        icon: ICON_INFO,
        action: () => options.openProps(entry),
      },
      {
        label: "删除",
        icon: ICON_DELETE,
        danger: true,
        action: () => options.askDelete(entry),
      },
      {
        label: "重命名",
        icon: ICON_RENAME,
        action: () => options.startRename(entry),
      },
      ...(options.hasActiveSessionTab.value
        ? [
            {
              label: "添加为会话附件",
              icon: ICON_AT,
              action: () => addAsAttachment(entry),
            },
          ]
        : []),
      {
        label: "在资源管理器中打开",
        icon: ICON_REVEAL,
        action: () => revealInExplorer(entry.path),
      },
    ];
    options.openCtx(e, items);
  }

  function openEntryMenu(entry: FsEntry, e: MouseEvent) {
    if (entry.isDir) openDirMenu(entry, e);
    else openFileMenu(entry, e);
  }

  return { openRootMenu, openDirMenu, openFileMenu, openEntryMenu };
}
